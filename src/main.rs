use std::collections::HashMap;
use std::ops::ControlFlow;

use async_lsp::client_monitor::ClientProcessMonitorLayer;
use async_lsp::concurrency::ConcurrencyLayer;
use async_lsp::panic::CatchUnwindLayer;
use async_lsp::router::Router;
use async_lsp::server::LifecycleLayer;
use async_lsp::tracing::TracingLayer;
use async_lsp::{ClientSocket, LanguageServer, ResponseError};
use futures::future::BoxFuture;
use jotdown::{Container, Event, Parser};
use lsp_types::{
    DidChangeConfigurationParams, DidChangeTextDocumentParams, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, DidSaveTextDocumentParams, DocumentSymbol, DocumentSymbolParams,
    DocumentSymbolResponse, InitializeParams, InitializeResult, OneOf, Position, Range,
    ServerCapabilities, SymbolKind, TextDocumentSyncCapability, TextDocumentSyncKind, Url,
};
use tower::ServiceBuilder;
use tracing::Level;

/// Server state. async-lsp's omni-trait hands us `&mut self` on every request and
/// notification, so plain owned state needs no locking.
struct ServerState {
    #[allow(dead_code)]
    client: ClientSocket,
    /// Full text of every open document, keyed by URI.
    documents: HashMap<Url, String>,
}

impl LanguageServer for ServerState {
    type Error = ResponseError;
    type NotifyResult = ControlFlow<async_lsp::Result<()>>;

    fn initialize(
        &mut self,
        _params: InitializeParams,
    ) -> BoxFuture<'static, Result<InitializeResult, Self::Error>> {
        Box::pin(async move {
            Ok(InitializeResult {
                capabilities: ServerCapabilities {
                    // Full-document sync keeps things simple for now.
                    text_document_sync: Some(TextDocumentSyncCapability::Kind(
                        TextDocumentSyncKind::FULL,
                    )),
                    document_symbol_provider: Some(OneOf::Left(true)),
                    ..ServerCapabilities::default()
                },
                server_info: None,
            })
        })
    }

    fn did_open(&mut self, params: DidOpenTextDocumentParams) -> Self::NotifyResult {
        let doc = params.text_document;
        self.documents.insert(doc.uri, doc.text);
        ControlFlow::Continue(())
    }

    fn did_change(&mut self, params: DidChangeTextDocumentParams) -> Self::NotifyResult {
        // FULL sync: the last change contains the entire document.
        if let Some(change) = params.content_changes.into_iter().last() {
            self.documents.insert(params.text_document.uri, change.text);
        }
        ControlFlow::Continue(())
    }

    fn did_close(&mut self, params: DidCloseTextDocumentParams) -> Self::NotifyResult {
        self.documents.remove(&params.text_document.uri);
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
        let symbols = self
            .documents
            .get(&params.text_document.uri)
            .map(|text| heading_symbols(text));
        Box::pin(async move { Ok(symbols.map(DocumentSymbolResponse::Nested)) })
    }
}

/// A djot section being assembled while walking the event stream.
struct SectionFrame {
    /// Byte where the section (heading line) starts — the symbol's full range start.
    range_start: usize,
    /// Heading level, for the `detail` label.
    level: u16,
    /// Accumulated heading text.
    name: String,
    /// Byte span of the heading line itself — the symbol's selection range.
    selection_start: usize,
    selection_end: usize,
    /// Whether we are currently inside this section's own heading, collecting text.
    capturing: bool,
    /// Whether this section's heading has already been captured (guards against
    /// stray headings inside nested non-section containers, e.g. a blockquote).
    captured: bool,
    /// Child sections closed while this one was still open.
    children: Vec<DocumentSymbol>,
}

impl SectionFrame {
    fn into_symbol(self, text: &str, section_end: usize) -> DocumentSymbol {
        let range = Range {
            start: offset_to_position(text, self.range_start),
            end: offset_to_position(text, section_end),
        };
        let selection_range = Range {
            start: offset_to_position(text, self.selection_start),
            end: offset_to_position(text, self.selection_end),
        };
        #[allow(deprecated)]
        DocumentSymbol {
            name: if self.name.is_empty() {
                format!("H{}", self.level)
            } else {
                self.name
            },
            detail: Some(format!("H{}", self.level)),
            kind: SymbolKind::STRING,
            tags: None,
            deprecated: None,
            range,
            selection_range,
            children: if self.children.is_empty() {
                None
            } else {
                Some(self.children)
            },
        }
    }
}

/// Build a hierarchy of `DocumentSymbol`s from the document's heading sections.
///
/// jotdown wraps each heading in a `Section` container that nests by heading
/// level, so the section nesting *is* the symbol hierarchy. Each section's span
/// (heading + body + nested subsections) becomes the symbol `range`, while the
/// heading line becomes the `selection_range`.
fn heading_symbols(text: &str) -> Vec<DocumentSymbol> {
    let mut roots: Vec<DocumentSymbol> = Vec::new();
    let mut stack: Vec<SectionFrame> = Vec::new();

    for (event, span) in Parser::new(text).into_offset_iter() {
        match event {
            Event::Start(Container::Section { .. }, _) => {
                stack.push(SectionFrame {
                    range_start: span.start,
                    level: 0,
                    name: String::new(),
                    selection_start: span.start,
                    selection_end: span.start,
                    capturing: false,
                    captured: false,
                    children: Vec::new(),
                });
            }
            Event::Start(Container::Heading { level, .. }, _) => {
                // Only the first heading directly inside a section is that
                // section's title; ignore headings in nested non-section blocks.
                if let Some(top) = stack.last_mut() {
                    if !top.captured {
                        top.level = level;
                        top.selection_start = span.start;
                        top.selection_end = span.end;
                        top.capturing = true;
                    }
                }
            }
            Event::Str(s) => {
                if let Some(top) = stack.last_mut() {
                    if top.capturing {
                        top.name.push_str(&s);
                    }
                }
            }
            Event::End(Container::Heading { .. }) => {
                if let Some(top) = stack.last_mut() {
                    if top.capturing {
                        top.capturing = false;
                        top.captured = true;
                        top.selection_end = span.end;
                    }
                }
            }
            Event::End(Container::Section { .. }) => {
                if let Some(frame) = stack.pop() {
                    let symbol = frame.into_symbol(text, span.end);
                    match stack.last_mut() {
                        Some(parent) => parent.children.push(symbol),
                        None => roots.push(symbol),
                    }
                }
            }
            _ => {}
        }
    }

    roots
}

/// Convert a byte offset into an LSP `Position` (line + UTF-16 column).
fn offset_to_position(text: &str, offset: usize) -> Position {
    let mut line = 0u32;
    let mut character = 0u32;
    for (i, c) in text.char_indices() {
        if i >= offset {
            break;
        }
        if c == '\n' {
            line += 1;
            character = 0;
        } else {
            character += c.len_utf16() as u32;
        }
    }
    Position { line, character }
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
                documents: HashMap::new(),
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
