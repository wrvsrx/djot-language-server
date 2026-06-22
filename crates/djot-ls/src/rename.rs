use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use async_lsp::ResponseError;
use djot_core::{Workspace, WorkspaceEdit as CoreWorkspaceEdit};
use lsp_types::{
    DocumentChangeOperation, DocumentChanges, OneOf, OptionalVersionedTextDocumentIdentifier,
    RenameFile, RenameFileOptions, ResourceOp, TextDocumentEdit, TextEdit, Url, WorkspaceEdit,
};

use crate::edit_context::EditContext;
use crate::lsp_utils::invalid_rename_path_error;

pub(crate) fn anchor_rename_workspace_edit(
    workspace: &Workspace,
    target_path: &Path,
    target_id: &str,
    new_name: &str,
) -> Option<WorkspaceEdit> {
    let edits = workspace.anchor_rename_edits(target_path, target_id, new_name);
    if edits.is_empty() {
        return None;
    }

    let mut changes: HashMap<Url, Vec<TextEdit>> = HashMap::new();
    for edit in edits {
        let entry = workspace.get(&edit.path)?;
        let uri = Url::from_file_path(&edit.path).ok()?;
        changes
            .entry(uri)
            .or_default()
            .push(EditContext::lsp_text_edit(&entry.text, edit.edit));
    }

    Some(WorkspaceEdit::new(changes))
}

pub(crate) fn path_rename_workspace_edit(
    workspace: &Workspace,
    old_path: &Path,
    new_path: &Path,
) -> Result<Option<WorkspaceEdit>, ResponseError> {
    let plan = workspace.path_rename_edit_plan(old_path, new_path);
    let mut operations = Vec::new();
    let mut edits_by_path: BTreeMap<PathBuf, Vec<TextEdit>> = BTreeMap::new();

    for edit in plan {
        match edit {
            CoreWorkspaceEdit::RenameFile(edit) => {
                let old_uri = Url::from_file_path(&edit.old_path)
                    .ok()
                    .ok_or_else(invalid_rename_path_error)?;
                let new_uri = Url::from_file_path(&edit.new_path)
                    .ok()
                    .ok_or_else(invalid_rename_path_error)?;
                operations.push(DocumentChangeOperation::Op(ResourceOp::Rename(
                    RenameFile {
                        old_uri,
                        new_uri,
                        options: Some(RenameFileOptions {
                            overwrite: Some(false),
                            ignore_if_exists: Some(false),
                        }),
                        annotation_id: None,
                    },
                )));
            }
            CoreWorkspaceEdit::Text(edit) => {
                let entry = workspace
                    .get(&edit.path)
                    .ok_or_else(invalid_rename_path_error)?;
                edits_by_path
                    .entry(edit.path)
                    .or_default()
                    .push(EditContext::lsp_text_edit(&entry.text, edit.edit));
            }
        }
    }

    for (path, edits) in edits_by_path {
        let uri = Url::from_file_path(&path)
            .ok()
            .ok_or_else(invalid_rename_path_error)?;
        operations.push(DocumentChangeOperation::Edit(TextDocumentEdit {
            text_document: OptionalVersionedTextDocumentIdentifier { uri, version: None },
            edits: edits.into_iter().map(OneOf::Left).collect(),
        }));
    }

    Ok(Some(WorkspaceEdit {
        changes: None,
        document_changes: Some(DocumentChanges::Operations(operations)),
        change_annotations: None,
    }))
}
