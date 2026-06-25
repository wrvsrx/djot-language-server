//! Layer A: semantics-agnostic djot *syntax* analysis layered on jotdown.
//!
//! jotdown yields parsed values but not the source byte ranges that edits need
//! — which colons open a fenced div, where in a line a new attribute block
//! belongs. This module recovers those ranges from jotdown's spans without
//! knowing what the syntax *means* (tasks, metadata, references); the semantic
//! layers read the ranges instead of re-scanning the source. Keeping it free of
//! project semantics is deliberate, so it can later be lifted into a standalone
//! djot syntax crate.

use std::ops::Range;

/// Syntactic anchors of a fenced div's opening fence that edits use to place a
/// new attribute block.
///
/// Recovered from the div's own span, so it is independent of the fence's colon
/// count or list nesting: a nested `:::: task` resolves to itself rather than to
/// an inner `::: task`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DivFence {
    /// Byte span of the opening fence line itself (e.g. `:::: task`).
    pub fence_range: Range<usize>,
    /// Range to replace when inserting a new `{...}` attribute line. Empty (an
    /// insertion point) for a bare fence; the `<indent>- ` marker for the
    /// compact `- ::: task` list form, which `attribute_prefix` re-emits.
    pub attribute_insert: Range<usize>,
    /// Prefix emitted before the first inserted attribute line.
    pub attribute_prefix: String,
    /// Prefix emitted before any subsequent inserted attribute line.
    pub continued_attribute_prefix: String,
    /// Prefix re-emitted before the fence after the inserted line(s).
    pub fence_prefix: String,
    /// Indent of the div's content (fence indent plus any list-marker width).
    pub indent: String,
}

/// Resolve the opening fence of the div whose source range is `div_span`.
///
/// `div_span` is jotdown's span for the div, which also covers the block
/// attribute lines preceding the fence, so we scan forward to the first fence
/// line rather than assuming `div_span.start` is the fence. Fence detection is
/// colon-count agnostic, which is what keeps a 4-colon outer fence from being
/// skipped in favour of a 3-colon inner one.
pub fn div_fence(text: &str, div_span: &Range<usize>) -> Option<DivFence> {
    let mut offset = div_span.start;
    while offset <= div_span.end {
        let (line_start, line_end) = line_bounds(text, offset)?;
        let line = text.get(line_start..line_end)?;
        if let Some(fence) = div_fence_from_line(line_start, line) {
            return Some(fence);
        }
        if line_end >= div_span.end || line_end == text.len() {
            break;
        }
        offset = next_line_start(text, line_end)?;
    }
    None
}

fn div_fence_from_line(line_start: usize, line: &str) -> Option<DivFence> {
    let line = line.strip_suffix('\r').unwrap_or(line);
    let fence_range = line_start..line_start + line.len();
    let indent = leading_indent(line);
    let rest = &line[indent.len()..];

    if is_div_fence(rest) {
        return Some(DivFence {
            fence_range,
            attribute_insert: line_start..line_start,
            attribute_prefix: indent.to_string(),
            continued_attribute_prefix: indent.to_string(),
            fence_prefix: String::new(),
            indent: indent.to_string(),
        });
    }

    if !is_div_fence(rest.strip_prefix("- ")?) {
        return None;
    }
    let marker_end = line_start + indent.len() + "- ".len();
    Some(DivFence {
        fence_range,
        attribute_insert: line_start..marker_end,
        attribute_prefix: format!("{indent}- "),
        continued_attribute_prefix: format!("{indent}  "),
        fence_prefix: format!("{indent}  "),
        indent: format!("{indent}  "),
    })
}

/// Whether `rest` (a line with its indent and any list marker stripped) opens a
/// djot fenced div: three or more colons.
fn is_div_fence(rest: &str) -> bool {
    rest.bytes().take_while(|&byte| byte == b':').count() >= 3
}

pub(crate) fn leading_indent(line: &str) -> &str {
    let indent_len = line
        .char_indices()
        .find(|(_, c)| *c != ' ' && *c != '\t')
        .map(|(i, _)| i)
        .unwrap_or(line.len());
    &line[..indent_len]
}

pub(crate) fn line_bounds(text: &str, offset: usize) -> Option<(usize, usize)> {
    if offset > text.len() {
        return None;
    }
    let start = text[..offset].rfind('\n').map_or(0, |i| i + 1);
    let end = text[offset..].find('\n').map_or(text.len(), |i| offset + i);
    Some((start, end))
}

pub(crate) fn previous_line_start(text: &str, line_start: usize) -> Option<usize> {
    if line_start == 0 {
        return None;
    }
    let previous_end = line_start.checked_sub('\n'.len_utf8())?;
    Some(text[..previous_end].rfind('\n').map_or(0, |i| i + 1))
}

pub(crate) fn next_line_start(text: &str, line_end: usize) -> Option<usize> {
    if line_end >= text.len() {
        None
    } else {
        Some(line_end + '\n'.len_utf8())
    }
}
