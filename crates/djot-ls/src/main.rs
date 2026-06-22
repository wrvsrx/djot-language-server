use std::collections::HashSet;
use std::ffi::OsString;
use std::ops::ControlFlow;
use std::path::{Component, Path, PathBuf};

mod code_action;
mod completion;
mod edit_context;
mod hover;
mod lsp_utils;
mod rename;

use code_action::resolve_code_actions as resolve_code_actions_for_document;
use completion::*;
use hover::{anchor_hover_markdown, file_hover_markdown};
use lsp_utils::*;
use rename::{anchor_rename_workspace_edit, path_rename_workspace_edit};

use async_lsp::client_monitor::ClientProcessMonitorLayer;
use async_lsp::concurrency::ConcurrencyLayer;
use async_lsp::panic::CatchUnwindLayer;
use async_lsp::router::Router;
use async_lsp::server::LifecycleLayer;
use async_lsp::tracing::TracingLayer;
use async_lsp::{ClientSocket, LanguageServer, ResponseError};
use djot_core::{
    heading_outline, metadata_block, resolve_target, AnalysisDiagnostic, DiagnosticKind, Heading,
    PathRenameError, RefTarget, RenameTargetError, Workspace,
};
use futures::future::BoxFuture;
use lsp_types::{
    CodeActionKind, CodeActionOptions, CodeActionParams, CodeActionProviderCapability,
    CodeActionResponse, CompletionItem, CompletionItemKind, CompletionOptions, CompletionParams,
    CompletionResponse, Diagnostic, DiagnosticRelatedInformation, DiagnosticSeverity,
    DidChangeConfigurationParams, DidChangeTextDocumentParams, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, DidSaveTextDocumentParams, DocumentSymbol, DocumentSymbolParams,
    DocumentSymbolResponse, GotoDefinitionParams, GotoDefinitionResponse, Hover, HoverContents,
    HoverParams, HoverProviderCapability, InitializeParams, InitializeResult, InitializedParams,
    Location, MarkupContent, MarkupKind, NumberOrString, OneOf, Position, PrepareRenameResponse,
    ProgressParams, ProgressParamsValue, PublishDiagnosticsParams, ReferenceParams, RenameOptions,
    RenameParams, ServerCapabilities, SymbolKind, TextDocumentPositionParams,
    TextDocumentSyncCapability, TextDocumentSyncKind, Url, WorkDoneProgress, WorkDoneProgressBegin,
    WorkDoneProgressEnd, WorkDoneProgressOptions, WorkDoneProgressReport, WorkspaceEdit,
};
use tower::ServiceBuilder;
use tracing::Level;

/// Server state. async-lsp's omni-trait hands us `&mut self` on every request and
/// notification, so plain owned state needs no locking.
struct ServerState {
    #[allow(dead_code)]
    client: ClientSocket,
    /// Parsed documents, keyed by file path. Open buffers are inserted on
    /// did_open/did_change; cross-file link targets are loaded from disk lazily.
    workspace: Workspace,
    /// Roots supplied by the LSP client during initialize.
    workspace_roots: Vec<PathBuf>,
    /// Client support for workspace edits that include resource operations.
    #[allow(dead_code)]
    workspace_edit_capabilities: ClientWorkspaceEditCapabilities,
    /// Open buffers that should receive publishDiagnostics updates.
    open_documents: HashSet<PathBuf>,
}

impl LanguageServer for ServerState {
    type Error = ResponseError;
    type NotifyResult = ControlFlow<async_lsp::Result<()>>;

    fn initialize(
        &mut self,
        params: InitializeParams,
    ) -> BoxFuture<'static, Result<InitializeResult, Self::Error>> {
        self.workspace_roots = workspace_roots(&params);
        self.workspace_edit_capabilities = client_workspace_edit_capabilities(&params);

        Box::pin(async move {
            Ok(InitializeResult {
                capabilities: ServerCapabilities {
                    // Full-document sync keeps things simple for now.
                    text_document_sync: Some(TextDocumentSyncCapability::Kind(
                        TextDocumentSyncKind::FULL,
                    )),
                    document_symbol_provider: Some(OneOf::Left(true)),
                    definition_provider: Some(OneOf::Left(true)),
                    references_provider: Some(OneOf::Left(true)),
                    hover_provider: Some(HoverProviderCapability::Simple(true)),
                    rename_provider: Some(OneOf::Right(RenameOptions {
                        prepare_provider: Some(true),
                        work_done_progress_options: WorkDoneProgressOptions::default(),
                    })),
                    completion_provider: Some(CompletionOptions {
                        resolve_provider: Some(false),
                        trigger_characters: Some(vec![
                            "[".to_string(),
                            "(".to_string(),
                            "/".to_string(),
                            "#".to_string(),
                        ]),
                        all_commit_characters: None,
                        work_done_progress_options: WorkDoneProgressOptions::default(),
                        completion_item: None,
                    }),
                    code_action_provider: Some(CodeActionProviderCapability::Options(
                        CodeActionOptions {
                            code_action_kinds: Some(vec![
                                CodeActionKind::QUICKFIX,
                                CodeActionKind::REFACTOR_REWRITE,
                            ]),
                            resolve_provider: Some(false),
                            work_done_progress_options: WorkDoneProgressOptions::default(),
                        },
                    )),
                    ..ServerCapabilities::default()
                },
                server_info: None,
            })
        })
    }

    fn initialized(&mut self, _params: InitializedParams) -> Self::NotifyResult {
        self.index_workspace_roots_with_progress();
        self.publish_open_document_diagnostics();
        ControlFlow::Continue(())
    }

    fn did_open(&mut self, params: DidOpenTextDocumentParams) -> Self::NotifyResult {
        let doc = params.text_document;
        if let Ok(path) = doc.uri.to_file_path() {
            self.workspace.insert(path.clone(), doc.text);
            self.open_documents.insert(path);
            self.publish_open_document_diagnostics();
        }
        ControlFlow::Continue(())
    }

    fn did_change(&mut self, params: DidChangeTextDocumentParams) -> Self::NotifyResult {
        // FULL sync: the last change contains the entire document.
        if let Some(change) = params.content_changes.into_iter().last() {
            if let Ok(path) = params.text_document.uri.to_file_path() {
                self.workspace.insert(path, change.text);
                self.publish_open_document_diagnostics();
            }
        }
        ControlFlow::Continue(())
    }

    fn did_close(&mut self, params: DidCloseTextDocumentParams) -> Self::NotifyResult {
        if let Ok(path) = params.text_document.uri.to_file_path() {
            self.open_documents.remove(&path);
            self.clear_diagnostics_for(&path);
            // Drop the open-buffer text. For workspace files, keep the disk
            // version indexed so cross-file lookups and references remain
            // available after the editor closes the buffer.
            if self.is_in_workspace(&path) {
                if let Ok(text) = std::fs::read_to_string(&path) {
                    self.workspace.insert(path, text);
                } else {
                    self.workspace.remove(&path);
                }
            } else {
                self.workspace.remove(&path);
            }
            self.publish_open_document_diagnostics();
        }
        ControlFlow::Continue(())
    }

    // async-lsp breaks the main loop on any notification we don't explicitly
    // handle (the omni-trait default is `ControlFlow::Break(Routing(..))`), so
    // editors sending these would otherwise kill the server. Accept and ignore
    // them for now; `did_save` is a natural hook for re-running diagnostics later.
    fn did_save(&mut self, _params: DidSaveTextDocumentParams) -> Self::NotifyResult {
        ControlFlow::Continue(())
    }

    fn did_change_configuration(
        &mut self,
        _params: DidChangeConfigurationParams,
    ) -> Self::NotifyResult {
        ControlFlow::Continue(())
    }

    fn document_symbol(
        &mut self,
        params: DocumentSymbolParams,
    ) -> BoxFuture<'static, Result<Option<DocumentSymbolResponse>, Self::Error>> {
        let symbols = params
            .text_document
            .uri
            .to_file_path()
            .ok()
            .and_then(|path| {
                self.workspace.get(&path).map(|entry| {
                    heading_outline(&entry.text)
                        .iter()
                        .map(|h| to_document_symbol(&entry.text, h))
                        .collect::<Vec<_>>()
                })
            });
        Box::pin(async move { Ok(symbols.map(DocumentSymbolResponse::Nested)) })
    }

    fn definition(
        &mut self,
        params: GotoDefinitionParams,
    ) -> BoxFuture<'static, Result<Option<GotoDefinitionResponse>, Self::Error>> {
        let pos = params.text_document_position_params;
        let location = self.resolve_definition(&pos.text_document.uri, pos.position);
        Box::pin(async move { Ok(location.map(GotoDefinitionResponse::Scalar)) })
    }

    fn references(
        &mut self,
        params: ReferenceParams,
    ) -> BoxFuture<'static, Result<Option<Vec<Location>>, Self::Error>> {
        let pos = params.text_document_position;
        let locations = self.resolve_references(
            &pos.text_document.uri,
            pos.position,
            params.context.include_declaration,
        );
        Box::pin(async move { Ok(locations) })
    }

    fn hover(
        &mut self,
        params: HoverParams,
    ) -> BoxFuture<'static, Result<Option<Hover>, Self::Error>> {
        let pos = params.text_document_position_params;
        let hover = self.resolve_hover(&pos.text_document.uri, pos.position);
        Box::pin(async move { Ok(hover) })
    }

    fn completion(
        &mut self,
        params: CompletionParams,
    ) -> BoxFuture<'static, Result<Option<CompletionResponse>, Self::Error>> {
        let pos = params.text_document_position;
        let completions = self.resolve_completion(&pos.text_document.uri, pos.position);
        Box::pin(async move { Ok(completions.map(CompletionResponse::Array)) })
    }

    fn code_action(
        &mut self,
        params: CodeActionParams,
    ) -> BoxFuture<'static, Result<Option<CodeActionResponse>, Self::Error>> {
        let actions = self.resolve_code_actions(&params);
        Box::pin(async move { Ok(actions) })
    }

    fn prepare_rename(
        &mut self,
        params: TextDocumentPositionParams,
    ) -> BoxFuture<'static, Result<Option<PrepareRenameResponse>, Self::Error>> {
        let response = self.resolve_prepare_rename(&params.text_document.uri, params.position);
        Box::pin(async move { response })
    }

    fn rename(
        &mut self,
        params: RenameParams,
    ) -> BoxFuture<'static, Result<Option<WorkspaceEdit>, Self::Error>> {
        let pos = params.text_document_position;
        let edit = self.resolve_rename(&pos.text_document.uri, pos.position, params.new_name);
        Box::pin(async move { edit })
    }
}

impl ServerState {
    fn index_workspace_root(&mut self, root: &Path) -> usize {
        index_djot_files(root, &mut |path, text| {
            self.workspace.insert(path, text);
        })
    }

    fn is_in_workspace(&self, path: &Path) -> bool {
        self.workspace_roots
            .iter()
            .any(|root| path.starts_with(root))
    }

    fn index_workspace_roots_with_progress(&mut self) {
        if self.workspace_roots.is_empty() {
            return;
        }

        self.notify_index_progress(WorkDoneProgress::Begin(WorkDoneProgressBegin {
            title: "Indexing Djot workspace".to_string(),
            cancellable: Some(false),
            message: Some("Scanning .dj/.djot files".to_string()),
            percentage: None,
        }));

        let mut indexed = 0usize;
        for root in self.workspace_roots.clone() {
            indexed += self.index_workspace_root(&root);
        }

        self.notify_index_progress(WorkDoneProgress::Report(WorkDoneProgressReport {
            cancellable: Some(false),
            message: Some(format!("Indexed {indexed} files")),
            percentage: None,
        }));
        self.notify_index_progress(WorkDoneProgress::End(WorkDoneProgressEnd {
            message: Some(format!("Indexed {indexed} Djot files")),
        }));
    }

    fn notify_index_progress(&self, progress: WorkDoneProgress) {
        let _ = self
            .client
            .notify::<lsp_types::notification::Progress>(ProgressParams {
                token: NumberOrString::String("djot-ls-index".to_string()),
                value: ProgressParamsValue::WorkDone(progress),
            });
    }

    fn publish_open_document_diagnostics(&self) {
        for path in &self.open_documents {
            self.publish_diagnostics_for(path);
        }
    }

    fn publish_diagnostics_for(&self, path: &Path) {
        let Some(entry) = self.workspace.get(path) else {
            return;
        };
        let Some(uri) = Url::from_file_path(path).ok() else {
            return;
        };
        let diagnostics = self
            .workspace
            .diagnostics_for(path)
            .into_iter()
            .map(|diagnostic| to_lsp_diagnostic(&entry.text, &uri, diagnostic))
            .collect();

        let _ = self
            .client
            .notify::<lsp_types::notification::PublishDiagnostics>(PublishDiagnosticsParams {
                uri,
                diagnostics,
                version: None,
            });
    }

    fn clear_diagnostics_for(&self, path: &Path) {
        let Some(uri) = Url::from_file_path(path).ok() else {
            return;
        };
        let _ = self
            .client
            .notify::<lsp_types::notification::PublishDiagnostics>(PublishDiagnosticsParams {
                uri,
                diagnostics: Vec::new(),
                version: None,
            });
    }

    /// Resolve goto-definition for the link under `position` in `uri`. Same-file
    /// `#id` links and cross-file `path#id` links are handled uniformly through
    /// the workspace index; a cross-file target not yet indexed is loaded from
    /// disk on demand.
    fn resolve_definition(&mut self, uri: &Url, position: Position) -> Option<Location> {
        let from = uri.to_file_path().ok()?;
        let offset = position_to_offset(&self.workspace.get(&from)?.text, position);

        // Resolve the link under the cursor to a (path, id) target.
        let target = {
            let reference = self.workspace.reference_at(&from, offset)?;
            resolve_target(&from, &reference.target)?
        };

        // Pull the target file into the index if we have not parsed it yet.
        if !self.workspace.contains(&target.path) {
            if let Ok(text) = std::fs::read_to_string(&target.path) {
                self.workspace.insert(target.path.clone(), text);
            }
        }

        let entry = self.workspace.get(&target.path)?;
        let range = match &target.id {
            Some(id) => entry.analysis.index.anchors.get(id)?.range.clone(),
            None => 0..0, // a link to the file itself jumps to its top
        };
        Some(Location {
            uri: Url::from_file_path(&target.path).ok()?,
            range: byte_range_to_lsp(&entry.text, &range),
        })
    }

    /// Resolve find-references for either an anchor under the cursor or a link
    /// under the cursor. Only anchored targets (`#id` / `path#id`) have
    /// references; file-only links do not name a symbol.
    fn resolve_references(
        &mut self,
        uri: &Url,
        position: Position,
        include_declaration: bool,
    ) -> Option<Vec<Location>> {
        let from = uri.to_file_path().ok()?;
        let offset = position_to_offset(&self.workspace.get(&from)?.text, position);
        let (target_path, target_id) = self.reference_target_at(&from, offset)?;

        let mut locations = Vec::new();
        if include_declaration {
            let entry = self.workspace.get(&target_path)?;
            let anchor = entry.analysis.index.anchors.get(&target_id)?;
            locations.push(Location {
                uri: Url::from_file_path(&target_path).ok()?,
                range: byte_range_to_lsp(&entry.text, &anchor.range),
            });
        }

        for (path, range) in self.workspace.references_to(&target_path, &target_id) {
            let Some(entry) = self.workspace.get(&path) else {
                continue;
            };
            let Some(uri) = Url::from_file_path(&path).ok() else {
                continue;
            };
            locations.push(Location {
                uri,
                range: byte_range_to_lsp(&entry.text, &range),
            });
        }

        Some(locations)
    }

    fn reference_target_at(&self, path: &Path, offset: usize) -> Option<(PathBuf, String)> {
        if let Some((id, _)) = self.workspace.anchor_at(path, offset) {
            return Some((path.to_path_buf(), id.to_string()));
        }

        let reference = self.workspace.reference_at(path, offset)?;
        let target = resolve_target(path, &reference.target)?;
        target.id.map(|id| (target.path, id))
    }

    fn resolve_prepare_rename(
        &self,
        uri: &Url,
        position: Position,
    ) -> Result<Option<PrepareRenameResponse>, ResponseError> {
        let from = match uri.to_file_path() {
            Ok(path) => path,
            Err(()) => return Ok(None),
        };
        let Some(entry) = self.workspace.get(&from) else {
            return Ok(None);
        };
        let offset = position_to_offset(&entry.text, position);
        match self.workspace.rename_target_at(&from, offset) {
            Ok(target) => {
                return Ok(Some(PrepareRenameResponse::RangeWithPlaceholder {
                    range: byte_range_to_lsp(&entry.text, &target.range),
                    placeholder: target.id,
                }));
            }
            Err(RenameTargetError::NotRenameable) => {}
            Err(RenameTargetError::ImplicitHeadingAnchor) => {
                return Err(implicit_heading_rename_error());
            }
        }

        let target = match self.workspace.path_rename_target_at(&from, offset) {
            Ok(target) => target,
            Err(PathRenameError::NotRenameable) => return Ok(None),
            Err(PathRenameError::NonDjotPath) => return Err(non_djot_path_rename_error()),
            Err(PathRenameError::TargetNotIndexed) => return Err(unindexed_path_rename_error()),
        };
        let placeholder = entry
            .text
            .get(target.range.clone())
            .unwrap_or_default()
            .to_string();
        Ok(Some(PrepareRenameResponse::RangeWithPlaceholder {
            range: byte_range_to_lsp(&entry.text, &target.range),
            placeholder,
        }))
    }

    fn resolve_rename(
        &mut self,
        uri: &Url,
        position: Position,
        new_name: String,
    ) -> Result<Option<WorkspaceEdit>, ResponseError> {
        let from = match uri.to_file_path() {
            Ok(path) => path,
            Err(()) => return Ok(None),
        };
        let Some(entry) = self.workspace.get(&from) else {
            return Ok(None);
        };
        let offset = position_to_offset(&entry.text, position);
        match self.workspace.rename_target_at(&from, offset) {
            Ok(target) => {
                if !is_valid_anchor_id(&new_name) {
                    return Ok(None);
                }
                return self.resolve_anchor_rename(&target.path, &target.id, new_name);
            }
            Err(RenameTargetError::NotRenameable) => {}
            Err(RenameTargetError::ImplicitHeadingAnchor) => {
                return Err(implicit_heading_rename_error());
            }
        }

        self.resolve_path_rename(&from, offset, new_name)
    }

    fn resolve_anchor_rename(
        &self,
        target_path: &Path,
        target_id: &str,
        new_name: String,
    ) -> Result<Option<WorkspaceEdit>, ResponseError> {
        Ok(anchor_rename_workspace_edit(
            &self.workspace,
            target_path,
            target_id,
            &new_name,
        ))
    }

    fn resolve_path_rename(
        &mut self,
        from: &Path,
        offset: usize,
        new_name: String,
    ) -> Result<Option<WorkspaceEdit>, ResponseError> {
        let target = match self.workspace.path_rename_target_at(from, offset) {
            Ok(target) => target,
            Err(PathRenameError::NotRenameable) => return Ok(None),
            Err(PathRenameError::NonDjotPath) => return Err(non_djot_path_rename_error()),
            Err(PathRenameError::TargetNotIndexed) => return Err(unindexed_path_rename_error()),
        };

        if !self.workspace_edit_capabilities.document_changes {
            return Err(document_changes_capability_error());
        }
        if !self.workspace_edit_capabilities.rename_resource_operation {
            return Err(rename_resource_operation_capability_error());
        }

        let new_path = self.resolve_new_link_path(from, &new_name)?;
        if new_path == target.old_path {
            return Ok(None);
        }
        if self.workspace.contains(&new_path) || new_path.exists() {
            return Err(rename_target_exists_error());
        }

        let edit = path_rename_workspace_edit(&self.workspace, &target.old_path, &new_path)?;

        if let Some(entry) = self.workspace.get(&target.old_path) {
            let text = entry.text.clone();
            self.workspace.insert(new_path, text);
            self.workspace.remove(&target.old_path);
        }

        Ok(edit)
    }

    fn resolve_new_link_path(&self, from: &Path, new_name: &str) -> Result<PathBuf, ResponseError> {
        if !is_valid_link_path_rename(new_name) {
            return Err(invalid_rename_path_error());
        }
        let target = resolve_target(
            from,
            &RefTarget::External {
                path: new_name.to_string(),
                id: None,
            },
        )
        .ok_or_else(invalid_rename_path_error)?;
        if !is_djot_file(&target.path) {
            return Err(non_djot_path_rename_error());
        }
        if !self.workspace_roots.is_empty()
            && !self
                .workspace_roots
                .iter()
                .any(|root| target.path.starts_with(root))
        {
            return Err(rename_target_outside_workspace_error());
        }
        Ok(target.path)
    }

    fn resolve_hover(&mut self, uri: &Url, position: Position) -> Option<Hover> {
        let from = uri.to_file_path().ok()?;
        let offset = position_to_offset(&self.workspace.get(&from)?.text, position);

        if let Some((id, anchor)) = self.workspace.anchor_at(&from, offset) {
            let entry = self.workspace.get(&from)?;
            return Some(Hover {
                contents: HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: anchor_hover_markdown(
                        self.display_path(&from),
                        id,
                        &entry.text,
                        &anchor.range,
                    ),
                }),
                range: Some(byte_range_to_lsp(&entry.text, &anchor.range)),
            });
        }

        let (target, source_range) = {
            let reference = self.workspace.reference_at(&from, offset)?;
            (
                resolve_target(&from, &reference.target)?,
                reference.source.clone(),
            )
        };

        if !self.workspace.contains(&target.path) {
            if let Ok(text) = std::fs::read_to_string(&target.path) {
                self.workspace.insert(target.path.clone(), text);
            }
        }

        let source_lsp_range = {
            let entry = self.workspace.get(&from)?;
            byte_range_to_lsp(&entry.text, &source_range)
        };
        let entry = self.workspace.get(&target.path)?;
        let value = match &target.id {
            Some(id) => {
                let anchor = entry.analysis.index.anchors.get(id)?;
                anchor_hover_markdown(
                    self.display_path(&target.path),
                    id,
                    &entry.text,
                    &anchor.range,
                )
            }
            None => file_hover_markdown(self.display_path(&target.path), &entry.text),
        };

        Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value,
            }),
            range: Some(source_lsp_range),
        })
    }

    fn resolve_completion(&self, uri: &Url, position: Position) -> Option<Vec<CompletionItem>> {
        let from = uri.to_file_path().ok()?;
        let entry = self.workspace.get(&from)?;
        let offset = position_to_offset(&entry.text, position);
        let context = link_completion_context(&entry.text, offset)?;

        let items = match context {
            LinkCompletionContext::Label { replace, query } => self
                .workspace_link_targets(&from)
                .into_iter()
                .filter(|target| {
                    fuzzy_match(&query, &target.title) || fuzzy_match(&query, &target.path)
                })
                .map(|target| {
                    completion_item(
                        target.title.clone(),
                        Some(target.path.clone()),
                        format!(
                            "[{}]({})",
                            escape_link_label(&target.title),
                            escape_link_destination(&target.path)
                        ),
                        &entry.text,
                        &replace,
                        CompletionItemKind::FILE,
                    )
                })
                .collect(),
            LinkCompletionContext::Destination { replace, query } => self
                .workspace_link_targets(&from)
                .into_iter()
                .filter(|target| {
                    fuzzy_match(&query, &target.path) || fuzzy_match(&query, &target.title)
                })
                .map(|target| {
                    completion_item(
                        target.path.clone(),
                        Some(target.title.clone()),
                        escape_link_destination(&target.path),
                        &entry.text,
                        &replace,
                        CompletionItemKind::FILE,
                    )
                })
                .collect(),
            LinkCompletionContext::Anchor {
                path,
                replace,
                query,
            } => self
                .anchor_completions(&from, &path)?
                .into_iter()
                .filter(|anchor| fuzzy_match(&query, &anchor.id))
                .map(|anchor| {
                    completion_item(
                        anchor.id.clone(),
                        Some(anchor.path.clone()),
                        escape_link_destination(&anchor.id),
                        &entry.text,
                        &replace,
                        CompletionItemKind::REFERENCE,
                    )
                })
                .collect(),
        };

        Some(items)
    }

    fn resolve_code_actions(&self, params: &CodeActionParams) -> Option<CodeActionResponse> {
        let path = params.text_document.uri.to_file_path().ok()?;
        let entry = self.workspace.get(&path)?;
        let offset = position_to_offset(&entry.text, params.range.start);
        Some(resolve_code_actions_for_document(
            &self.workspace,
            params,
            &path,
            entry,
            offset,
        ))
    }

    fn workspace_link_targets(&self, from: &Path) -> Vec<LinkTargetCompletion> {
        let mut targets: Vec<_> = self
            .workspace
            .documents()
            .map(|(path, entry)| {
                let path =
                    relative_link_path(from, path).unwrap_or_else(|| self.display_path(path));
                let title = document_title(&entry.text).unwrap_or_else(|| path.clone());
                LinkTargetCompletion { title, path }
            })
            .collect();
        targets.sort_by(|a, b| {
            a.title
                .to_lowercase()
                .cmp(&b.title.to_lowercase())
                .then_with(|| a.path.cmp(&b.path))
        });
        targets
    }

    fn anchor_completions(&self, from: &Path, link_path: &str) -> Option<Vec<AnchorCompletion>> {
        let target_path = if link_path.is_empty() {
            from.to_path_buf()
        } else {
            resolve_target(
                from,
                &RefTarget::External {
                    path: link_path.to_string(),
                    id: None,
                },
            )?
            .path
        };

        let entry = self.workspace.get(&target_path)?;
        let display_path = relative_link_path(from, &target_path).unwrap_or_else(|| {
            self.workspace_roots
                .iter()
                .find_map(|root| target_path.strip_prefix(root).ok())
                .unwrap_or(&target_path)
                .display()
                .to_string()
        });
        let mut anchors: Vec<_> = entry
            .analysis
            .index
            .anchors
            .keys()
            .map(|id| AnchorCompletion {
                id: id.clone(),
                path: display_path.clone(),
            })
            .collect();
        anchors.sort_by(|a, b| a.id.to_lowercase().cmp(&b.id.to_lowercase()));
        Some(anchors)
    }

    fn display_path(&self, path: &Path) -> String {
        self.workspace_roots
            .iter()
            .find_map(|root| path.strip_prefix(root).ok())
            .unwrap_or(path)
            .display()
            .to_string()
    }
}

fn to_lsp_diagnostic(text: &str, uri: &Url, diagnostic: AnalysisDiagnostic) -> Diagnostic {
    let (code, message, related_information, severity) = match diagnostic.kind {
        DiagnosticKind::UnresolvedAnchor { id } => (
            "unresolved-anchor",
            format!("Unresolved anchor `{}`", id),
            None,
            DiagnosticSeverity::WARNING,
        ),
        DiagnosticKind::UnresolvedPath { path } => (
            "unresolved-path",
            format!("Unresolved Djot path `{}`", path),
            None,
            DiagnosticSeverity::WARNING,
        ),
        DiagnosticKind::DuplicateAnchor { id, first_range } => (
            "duplicate-anchor",
            format!("Duplicate anchor `{}`", id),
            Some(vec![DiagnosticRelatedInformation {
                location: Location::new(uri.clone(), byte_range_to_lsp(text, &first_range)),
                message: "First definition is here.".to_string(),
            }]),
            DiagnosticSeverity::WARNING,
        ),
        DiagnosticKind::MissingTaskDueForRecur => (
            "missing-task-due-for-recur",
            "Recurring tasks with `recur` need a valid RFC 3339 `due` datetime.".to_string(),
            None,
            DiagnosticSeverity::WARNING,
        ),
        DiagnosticKind::InvalidTaskRecur { recur } => (
            "invalid-task-recur",
            format!(
                "Unsupported task `recur` value `{}`. Use an ISO 8601 duration like `P1D`, `P1W`, `P1M`, or `P1Y`.",
                recur
            ),
            None,
            DiagnosticSeverity::WARNING,
        ),
        DiagnosticKind::ConflictingTaskClosedState => (
            "conflicting-task-closed-state",
            "Task cannot have both `done` and `canceled`.".to_string(),
            None,
            DiagnosticSeverity::WARNING,
        ),
        DiagnosticKind::InvalidTaskPrevTarget { id } => (
            "invalid-task-prev-target",
            format!("Task `prev` target `{}` must be a task.", id),
            None,
            DiagnosticSeverity::WARNING,
        ),
        DiagnosticKind::InvalidTaskDependencyTarget { target } => (
            "invalid-task-dependency-target",
            format!("Task dependency target `{}` must be a task.", target),
            None,
            DiagnosticSeverity::WARNING,
        ),
        DiagnosticKind::TaskSelfDependency { target } => (
            "task-self-dependency",
            format!("Task cannot depend on itself via `{}`.", target),
            None,
            DiagnosticSeverity::WARNING,
        ),
        DiagnosticKind::TaskDependencyCycle { id } => (
            "task-dependency-cycle",
            format!("Task dependency cycle includes `{}`.", id),
            None,
            DiagnosticSeverity::WARNING,
        ),
        DiagnosticKind::TaskBlocked { count } => (
            "task-blocked",
            match count {
                1 => "Blocked by 1 open dependency.".to_string(),
                _ => format!("Blocked by {count} open dependencies."),
            },
            None,
            DiagnosticSeverity::HINT,
        ),
    };

    Diagnostic {
        range: byte_range_to_lsp(text, &diagnostic.range),
        severity: Some(severity),
        code: Some(NumberOrString::String(code.to_string())),
        code_description: None,
        source: Some("djot-ls".to_string()),
        message,
        related_information,
        tags: None,
        data: None,
    }
}

fn document_title(text: &str) -> Option<String> {
    let metadata = metadata_block(text)?;
    let value: toml::Value = toml::from_str(&metadata).ok()?;
    value
        .get("title")
        .and_then(|title| title.as_str())
        .map(str::to_string)
}

fn relative_link_path(from: &Path, target: &Path) -> Option<String> {
    let base = from.parent()?;
    Some(relative_path(base, target)?.display().to_string())
}

fn relative_path(base: &Path, target: &Path) -> Option<PathBuf> {
    let base_components = lexical_components(base)?;
    let target_components = lexical_components(target)?;

    if base_components.first() != target_components.first() {
        return None;
    }

    let common_len = base_components
        .iter()
        .zip(target_components.iter())
        .take_while(|(base, target)| base == target)
        .count();

    let mut out = PathBuf::new();
    for _ in common_len..base_components.len() {
        out.push("..");
    }
    for component in &target_components[common_len..] {
        out.push(component);
    }

    Some(if out.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        out
    })
}

fn lexical_components(path: &Path) -> Option<Vec<OsString>> {
    let mut out = Vec::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop()?;
            }
            Component::Normal(part) => out.push(part.to_os_string()),
            Component::RootDir => out.push(OsString::from(std::path::MAIN_SEPARATOR.to_string())),
            Component::Prefix(prefix) => out.push(prefix.as_os_str().to_os_string()),
        }
    }
    Some(out)
}

/// Convert a core [`Heading`] (byte offsets) into an LSP `DocumentSymbol`.
fn to_document_symbol(text: &str, heading: &Heading) -> DocumentSymbol {
    let children: Vec<_> = heading
        .children
        .iter()
        .map(|child| to_document_symbol(text, child))
        .collect();
    #[allow(deprecated)]
    DocumentSymbol {
        name: if heading.name.is_empty() {
            format!("H{}", heading.level)
        } else {
            heading.name.clone()
        },
        detail: Some(format!("H{}", heading.level)),
        kind: SymbolKind::STRING,
        tags: None,
        deprecated: None,
        range: byte_range_to_lsp(text, &heading.range),
        selection_range: byte_range_to_lsp(text, &heading.selection_range),
        children: if children.is_empty() {
            None
        } else {
            Some(children)
        },
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let (server, _) = async_lsp::MainLoop::new_server(|client| {
        ServiceBuilder::new()
            .layer(TracingLayer::default())
            .layer(LifecycleLayer::default())
            .layer(CatchUnwindLayer::default())
            .layer(ConcurrencyLayer::default())
            .layer(ClientProcessMonitorLayer::new(client.clone()))
            .service(Router::from_language_server(ServerState {
                client,
                workspace: Workspace::new(),
                workspace_roots: Vec::new(),
                workspace_edit_capabilities: ClientWorkspaceEditCapabilities::default(),
                open_documents: HashSet::new(),
            }))
    });

    tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .with_ansi(false)
        .with_writer(std::io::stderr)
        .init();

    // Prefer truly asynchronous piped stdin/stdout without blocking tasks.
    let stdin = async_lsp::stdio::PipeStdin::lock_tokio().unwrap();
    let stdout = async_lsp::stdio::PipeStdout::lock_tokio().unwrap();

    server.run_buffered(stdin, stdout).await.unwrap();
}

fn index_djot_files(root: &Path, insert: &mut impl FnMut(PathBuf, String)) -> usize {
    let Ok(entries) = std::fs::read_dir(root) else {
        return 0;
    };

    let mut indexed = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };

        if file_type.is_dir() {
            indexed += index_djot_files(&path, insert);
        } else if file_type.is_file() && is_djot_file(&path) {
            if let Ok(text) = std::fs::read_to_string(&path) {
                insert(path, text);
                indexed += 1;
            }
        }
    }
    indexed
}

fn is_djot_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext == "dj" || ext == "djot")
}
