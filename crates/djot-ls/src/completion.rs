use std::ops::Range as ByteRange;

use jotdown::{Container, Event, Parser};
use lsp_types::{CompletionItem, CompletionItemKind, CompletionTextEdit, TextEdit};

use crate::lsp_utils::byte_range_to_lsp;

#[derive(Debug, Clone)]
pub(crate) struct LinkTargetCompletion {
    pub(crate) title: String,
    pub(crate) path: String,
}

#[derive(Debug, Clone)]
pub(crate) struct AnchorCompletion {
    pub(crate) id: String,
    pub(crate) path: String,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum LinkCompletionContext {
    Label {
        replace: ByteRange<usize>,
        query: String,
    },
    Destination {
        replace: ByteRange<usize>,
        query: String,
    },
    Anchor {
        path: String,
        replace: ByteRange<usize>,
        query: String,
    },
}

#[derive(Debug, Clone, Copy)]
enum LinkScanState {
    Text,
    Label { open: usize },
    AfterLabel,
    Destination { start: usize },
}

pub(crate) fn link_completion_context(text: &str, offset: usize) -> Option<LinkCompletionContext> {
    incomplete_link_completion_context(text, offset)
        .or_else(|| closed_link_anchor_completion_context(text, offset))
}

fn incomplete_link_completion_context(text: &str, offset: usize) -> Option<LinkCompletionContext> {
    let str_span = str_event_touching_cursor(text, offset)?;
    let prefix = &text[str_span.start..offset];
    let mut state = LinkScanState::Text;

    for (i, c) in prefix.char_indices() {
        let absolute = str_span.start + i;
        if is_escaped(prefix, i) {
            continue;
        }

        state = match state {
            LinkScanState::Text => {
                if c == '[' {
                    LinkScanState::Label { open: absolute }
                } else {
                    LinkScanState::Text
                }
            }
            LinkScanState::Label { open } => {
                if c == ']' {
                    LinkScanState::AfterLabel
                } else if c == '[' {
                    LinkScanState::Label { open: absolute }
                } else {
                    LinkScanState::Label { open }
                }
            }
            LinkScanState::AfterLabel => {
                if c == '(' {
                    LinkScanState::Destination {
                        start: absolute + c.len_utf8(),
                    }
                } else if c == '[' {
                    LinkScanState::Label { open: absolute }
                } else {
                    LinkScanState::Text
                }
            }
            LinkScanState::Destination { start } => {
                if c == ')' {
                    LinkScanState::Text
                } else {
                    LinkScanState::Destination { start }
                }
            }
        };
    }

    match state {
        LinkScanState::Label { open } => Some(LinkCompletionContext::Label {
            replace: open..label_completion_replace_end(text, offset, str_span.end),
            query: text[open + 1..offset].to_string(),
        }),
        LinkScanState::Destination { start } => {
            let query = &text[start..offset];
            if let Some((path, anchor_query)) = query.split_once('#') {
                Some(LinkCompletionContext::Anchor {
                    path: path.to_string(),
                    replace: start + path.len() + '#'.len_utf8()..offset,
                    query: anchor_query.to_string(),
                })
            } else {
                Some(LinkCompletionContext::Destination {
                    replace: start..offset,
                    query: query.to_string(),
                })
            }
        }
        LinkScanState::Text | LinkScanState::AfterLabel => None,
    }
}

fn closed_link_anchor_completion_context(
    text: &str,
    offset: usize,
) -> Option<LinkCompletionContext> {
    Parser::new(text)
        .into_offset_iter()
        .find_map(|(event, span)| match event {
            Event::End(Container::Link(dst, _)) if span.start <= offset && offset <= span.end => {
                closed_link_completion_from_end_span(text, span, dst.as_ref(), offset)
            }
            _ => None,
        })
}

fn closed_link_completion_from_end_span(
    text: &str,
    span: ByteRange<usize>,
    dst: &str,
    offset: usize,
) -> Option<LinkCompletionContext> {
    let syntax = &text[span.clone()];
    let dst_range = closed_link_destination_range(span.start, syntax, dst)?;
    let dst_start = dst_range.start;
    let dst_end = dst_range.end;

    if let Some(hash_in_dst) = dst.find('#') {
        let fragment_start = dst_start + hash_in_dst + '#'.len_utf8();
        if offset < fragment_start || offset > dst_end {
            return None;
        }

        return Some(LinkCompletionContext::Anchor {
            path: dst[..hash_in_dst].to_string(),
            replace: fragment_start..offset,
            query: text[fragment_start..offset].to_string(),
        });
    }

    if offset < dst_start || offset > dst_end {
        return None;
    }

    Some(LinkCompletionContext::Destination {
        replace: dst_start..offset,
        query: text[dst_start..offset].to_string(),
    })
}

fn closed_link_destination_range(
    span_start: usize,
    syntax: &str,
    dst: &str,
) -> Option<ByteRange<usize>> {
    if dst.is_empty() {
        let open = syntax.find('(')?;
        let close = syntax[open + '('.len_utf8()..].find(')')? + open + '('.len_utf8();
        if close == open + '('.len_utf8() {
            let cursor = span_start + close;
            return Some(cursor..cursor);
        }
    }

    let dst_in_syntax = syntax.find(dst)?;
    let dst_start = span_start + dst_in_syntax;
    Some(dst_start..dst_start + dst.len())
}

fn label_completion_replace_end(text: &str, offset: usize, limit: usize) -> usize {
    if offset < limit && text[offset..].starts_with(']') && !is_escaped(text, offset) {
        offset + ']'.len_utf8()
    } else {
        offset
    }
}

fn str_event_touching_cursor(text: &str, offset: usize) -> Option<ByteRange<usize>> {
    let mut ignored_depth = 0usize;
    for (event, span) in Parser::new(text).into_offset_iter() {
        match event {
            Event::Start(container, _) => {
                if ignored_depth > 0 || ignores_completion_str(&container) {
                    ignored_depth += 1;
                }
            }
            Event::End(container) => {
                let _ = container;
                ignored_depth = ignored_depth.saturating_sub(1);
            }
            Event::Str(_) if ignored_depth == 0 && span.start <= offset && offset <= span.end => {
                return Some(span);
            }
            _ => {}
        }
    }
    None
}

fn ignores_completion_str(container: &Container<'_>) -> bool {
    matches!(
        container,
        Container::Verbatim
            | Container::CodeBlock { .. }
            | Container::Math { .. }
            | Container::RawInline { .. }
            | Container::RawBlock { .. }
            | Container::Link(_, _)
            | Container::Image(_, _)
    )
}

fn is_escaped(text: &str, byte_index: usize) -> bool {
    let mut backslashes = 0;
    for b in text[..byte_index].bytes().rev() {
        if b == b'\\' {
            backslashes += 1;
        } else {
            break;
        }
    }
    backslashes % 2 == 1
}

pub(crate) fn completion_item(
    label: String,
    detail: Option<String>,
    new_text: String,
    source_text: &str,
    replace: &ByteRange<usize>,
    kind: CompletionItemKind,
) -> CompletionItem {
    CompletionItem {
        label,
        kind: Some(kind),
        detail,
        text_edit: Some(CompletionTextEdit::Edit(TextEdit::new(
            byte_range_to_lsp(source_text, replace),
            new_text,
        ))),
        ..CompletionItem::default()
    }
}

pub(crate) fn fuzzy_match(query: &str, candidate: &str) -> bool {
    if query.is_empty() {
        return true;
    }

    let mut chars = query.chars().flat_map(char::to_lowercase);
    let Some(mut needle) = chars.next() else {
        return true;
    };

    for c in candidate.chars().flat_map(char::to_lowercase) {
        if c == needle {
            if let Some(next) = chars.next() {
                needle = next;
            } else {
                return true;
            }
        }
    }
    false
}

pub(crate) fn escape_link_label(value: &str) -> String {
    value.replace('\\', "\\\\").replace(']', "\\]")
}

pub(crate) fn escape_link_destination(value: &str) -> String {
    value.replace('\\', "\\\\").replace(')', "\\)")
}
