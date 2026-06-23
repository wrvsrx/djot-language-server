use crate::position::offset_to_position;

pub(crate) struct TaskHover<'a> {
    pub title: &'a str,
    pub id: Option<&'a str>,
    pub created: Option<&'a str>,
    pub due: Option<&'a str>,
    pub wait: Option<&'a str>,
    pub done: Option<&'a str>,
    pub canceled: Option<&'a str>,
    pub recur: Option<&'a str>,
    pub prev: Option<&'a str>,
    pub depends: Vec<String>,
    pub blockers: Vec<String>,
}

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

pub(crate) fn task_hover_markdown(task: TaskHover<'_>) -> String {
    let status = if task.canceled.is_some() {
        "canceled"
    } else if task.done.is_some() {
        "done"
    } else if !task.blockers.is_empty() {
        "blocked"
    } else {
        "open"
    };
    let title = if task.title.trim().is_empty() {
        "Task"
    } else {
        task.title.trim()
    };
    let mut lines = vec![
        format!("**Task** `{}`", escape_markdown_code(title)),
        format!("status: `{status}`"),
    ];

    push_field(&mut lines, "id", task.id);
    push_field(&mut lines, "created", task.created);
    push_field(&mut lines, "due", task.due);
    push_field(&mut lines, "wait", task.wait);
    push_field(&mut lines, "done", task.done);
    push_field(&mut lines, "canceled", task.canceled);
    push_field(&mut lines, "recur", task.recur);
    push_field(&mut lines, "prev", task.prev);

    if !task.depends.is_empty() {
        lines.push(format!("depends: {}", inline_code_list(&task.depends)));
    }
    if !task.blockers.is_empty() {
        lines.push(format!("blocked by: {}", inline_code_list(&task.blockers)));
    }

    lines.join("\n\n")
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

fn push_field(lines: &mut Vec<String>, label: &str, value: Option<&str>) {
    if let Some(value) = value {
        lines.push(format!("{label}: `{}`", escape_markdown_code(value)));
    }
}

fn inline_code_list(values: &[String]) -> String {
    values
        .iter()
        .map(|value| format!("`{}`", escape_markdown_code(value)))
        .collect::<Vec<_>>()
        .join(", ")
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
