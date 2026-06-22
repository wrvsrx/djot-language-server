use crate::lsp_utils::offset_to_position;

pub(crate) fn anchor_hover_markdown(
    display_path: String,
    id: &str,
    text: &str,
    range: &std::ops::Range<usize>,
) -> String {
    let kind = if text[range.clone()].trim_start().starts_with('#') {
        "Heading"
    } else {
        "Anchor"
    };
    let line = offset_to_position(text, range.start).line + 1;
    let preview = preview_from_offset(text, range.start, 5);
    format!(
        "**{kind}** `{}`\n\n`{}:{line}`\n\n```djot\n{}\n```",
        escape_markdown_code(id),
        escape_markdown_code(&display_path),
        preview
    )
}

pub(crate) fn file_hover_markdown(display_path: String, text: &str) -> String {
    let (line, offset) = first_preview_offset(text);
    let preview = preview_from_offset(text, offset, 5);
    if preview.is_empty() {
        format!(
            "**File**\n\n`{}:{line}`",
            escape_markdown_code(&display_path)
        )
    } else {
        format!(
            "**File**\n\n`{}:{line}`\n\n```djot\n{}\n```",
            escape_markdown_code(&display_path),
            preview
        )
    }
}

fn first_preview_offset(text: &str) -> (usize, usize) {
    text.lines()
        .scan(0usize, |offset, line| {
            let current = *offset;
            *offset += line.len() + 1;
            Some((current, line))
        })
        .enumerate()
        .find(|(_, (_, line))| !line.trim().is_empty())
        .map(|(line, (offset, _))| (line + 1, offset))
        .unwrap_or((1, 0))
}

fn preview_from_offset(text: &str, offset: usize, max_lines: usize) -> String {
    let start = text[..offset].rfind('\n').map_or(0, |i| i + 1);
    text[start..]
        .lines()
        .take(max_lines)
        .collect::<Vec<_>>()
        .join("\n")
        .trim_end()
        .to_string()
}

fn escape_markdown_code(value: &str) -> String {
    value.replace('`', "\\`")
}
