use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::PathBuf;

use chrono::{DateTime, Datelike, Duration, FixedOffset, SecondsFormat, TimeZone, Timelike};
use iso8601_duration::Duration as IsoDuration;
use jotdown::{Container, Event, Parser};

use crate::{analyze, AnalysisDiagnostic, Anchor, DiagnosticKind, RefTarget, TextEdit};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    pub range: Range<usize>,
    pub title_range: Option<Range<usize>>,
    pub title: String,
    pub depth: usize,
    pub id: Option<String>,
    pub created: Option<String>,
    pub done: Option<String>,
    pub canceled: Option<String>,
    pub due: Option<String>,
    pub wait: Option<String>,
    pub recur: Option<String>,
    pub prev: Option<String>,
    pub depends: Vec<TaskDependency>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskDependency {
    pub source: String,
    pub range: Range<usize>,
    pub target: RefTarget,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskStatusEdit {
    pub edits: Vec<TextEdit>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatus {
    Done,
    Canceled,
}

impl TaskStatus {
    fn attribute(self) -> &'static str {
        match self {
            Self::Done => "done",
            Self::Canceled => "canceled",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskEditError {
    TaskIdNotFound { id: String },
    TaskAlreadyDone { id: String },
    TaskCanceled { id: String },
    CannotBuildEdit { id: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TaskRef {
    pub path: PathBuf,
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedTaskDependency {
    pub source: String,
    pub target: TaskRef,
    pub task: Task,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepeatRule {
    Days(i64),
    Weeks(i64),
    Months(i32),
    Years(i32),
}

pub fn task_list_item_conversion_edit(
    text: &str,
    offset: usize,
    created: &str,
) -> Option<TextEdit> {
    let (line_start, line_end) = line_bounds(text, offset)?;
    let line = text.get(line_start..line_end)?;
    let content = line.strip_suffix('\r').unwrap_or(line);
    let indent = leading_indent(content);
    let rest = &content[indent.len()..];
    let title = rest.strip_prefix("- [ ] ")?.trim();
    if title.is_empty() {
        return None;
    }

    Some(TextEdit {
        range: line_start..line_end,
        new_text: format!(
            "{indent}- {{created=\"{created}\"}}\n{indent}  ::: task\n{indent}  {title}\n{indent}  :::"
        ),
    })
}

pub fn task_status_edits_at(
    text: &str,
    offset: usize,
    status: TaskStatus,
    timestamp: &str,
) -> Option<TaskStatusEdit> {
    let analysis = analyze(text);
    let task = analysis.tasks.iter().find(|task| {
        task.done.is_none()
            && task.canceled.is_none()
            && task.range.start <= offset
            && offset <= task.range.end
    })?;
    task_status_edits_for_task(text, task, status, timestamp, true)
}

pub fn task_done_edits_by_id(
    text: &str,
    id: &str,
    done: &str,
) -> Result<Vec<TextEdit>, TaskEditError> {
    let analysis = analyze(text);
    let task = analysis
        .tasks
        .iter()
        .find(|task| task.id.as_deref() == Some(id))
        .ok_or_else(|| TaskEditError::TaskIdNotFound { id: id.to_string() })?;
    if task.done.is_some() {
        return Err(TaskEditError::TaskAlreadyDone { id: id.to_string() });
    }
    if task.canceled.is_some() {
        return Err(TaskEditError::TaskCanceled { id: id.to_string() });
    }

    task_status_edits_for_task(text, task, TaskStatus::Done, done, false)
        .map(|edit| edit.edits)
        .ok_or_else(|| TaskEditError::CannotBuildEdit { id: id.to_string() })
}

fn task_status_edits_for_task(
    text: &str,
    task: &Task,
    status: TaskStatus,
    timestamp: &str,
    allow_generated_current_id: bool,
) -> Option<TaskStatusEdit> {
    if task.recur.is_some() && task.due.is_some() {
        recurring_task_status_edits(text, task, status, timestamp, allow_generated_current_id)
    } else {
        simple_task_status_edits(text, task, status, timestamp)
    }
}

fn simple_task_status_edits(
    text: &str,
    task: &Task,
    status: TaskStatus,
    timestamp: &str,
) -> Option<TaskStatusEdit> {
    let attribute = status.attribute();
    let opening = task_opening_fence(text, &task.range)?;
    Some(TaskStatusEdit {
        edits: vec![TextEdit {
            range: opening.attribute_insert.clone(),
            new_text: format!(
                "{}{{{attribute}=\"{timestamp}\"}}\n{}",
                opening.attribute_prefix, opening.fence_prefix
            ),
        }],
    })
}

fn recurring_task_status_edits(
    text: &str,
    task: &Task,
    status: TaskStatus,
    timestamp: &str,
    allow_generated_current_id: bool,
) -> Option<TaskStatusEdit> {
    let attribute = status.attribute();
    let due = DateTime::parse_from_rfc3339(task.due.as_deref()?).ok()?;
    let recur = task.recur.as_deref()?;
    let next_due = next_recur_due(due, recur)?;
    let next_wait = task
        .wait
        .as_deref()
        .and_then(|wait| DateTime::parse_from_rfc3339(wait).ok())
        .and_then(|wait| next_recur_due(wait, recur));
    let opening = task_opening_fence(text, &task.range)?;
    let indent = opening.task_indent.as_str();

    let anchors = analyze(text).index.anchors;
    let mut reserved = HashSet::new();
    let current_id = match task.id.clone() {
        Some(id) => id,
        None if allow_generated_current_id => {
            let id = task_instance_id(&task.title, due, &anchors, &reserved)?;
            reserved.insert(id.clone());
            id
        }
        None => return None,
    };
    let next_id = task_instance_id(&task.title, next_due, &anchors, &reserved)?;
    let next_insert = line_bounds(text, task.range.end)?.1;
    let recur = escape_attribute_value(recur);
    let next_due_text = next_due.to_rfc3339_opts(SecondsFormat::Secs, true);
    let next_wait_text = next_wait.map(|wait| wait.to_rfc3339_opts(SecondsFormat::Secs, true));
    let next_wait_attribute = next_wait_text
        .as_deref()
        .map(|wait| format!(" wait=\"{}\"", escape_attribute_value(wait)))
        .unwrap_or_default();
    let current_id_text = escape_attribute_value(&current_id);
    let current_id_attribute = anchor_attribute(&current_id);
    let next_id_attribute = anchor_attribute(&next_id);
    let div = inherited_task_source(text.get(task.range.clone())?, indent);
    let list_item = single_task_list_item_context(text, opening.line_start, task.range.end, indent);

    let mut status_text = String::new();
    let mut attribute_prefix = opening.attribute_prefix.as_str();
    if task.id.is_none() {
        status_text.push_str(&format!("{attribute_prefix}{current_id_attribute}\n"));
        attribute_prefix = opening.continued_attribute_prefix.as_str();
    }
    status_text.push_str(&format!(
        "{attribute_prefix}{{{attribute}=\"{timestamp}\"}}\n{}",
        opening.fence_prefix
    ));

    let next_edit = match list_item {
        Some(context) => TextEdit {
            range: context.insert..context.insert,
            new_text: format!(
                "\n{list_indent}- {next_id_attribute}\n{indent}{{created=\"{timestamp}\" due=\"{next_due_text}\"{next_wait_attribute} recur=\"{recur}\" prev=\"#{current_id_text}\"}}\n{div}",
                list_indent = context.list_indent,
            ),
        },
        None => TextEdit {
            range: next_insert..next_insert,
            new_text: format!(
                "\n\n{indent}{next_id_attribute}\n{indent}{{created=\"{timestamp}\" due=\"{next_due_text}\"{next_wait_attribute} recur=\"{recur}\" prev=\"#{current_id_text}\"}}\n{div}"
            ),
        },
    };

    Some(TaskStatusEdit {
        edits: vec![
            TextEdit {
                range: opening.attribute_insert,
                new_text: status_text,
            },
            next_edit,
        ],
    })
}

pub(crate) fn document_local_task_diagnostics(tasks: &[Task]) -> Vec<AnalysisDiagnostic> {
    let mut diagnostics = Vec::new();

    for task in tasks {
        if task.done.is_some() && task.canceled.is_some() {
            diagnostics.push(AnalysisDiagnostic {
                range: task.range.clone(),
                kind: DiagnosticKind::ConflictingTaskClosedState,
            });
        }

        let Some(recur) = task.recur.as_deref() else {
            continue;
        };

        if parse_repeat_rule(recur).is_none() {
            diagnostics.push(AnalysisDiagnostic {
                range: task.range.clone(),
                kind: DiagnosticKind::InvalidTaskRecur {
                    recur: recur.to_string(),
                },
            });
        }

        if task.due.is_none() {
            diagnostics.push(AnalysisDiagnostic {
                range: task.range.clone(),
                kind: DiagnosticKind::MissingTaskDueForRecur,
            });
        }
    }

    diagnostics
}

struct ListTaskContext<'a> {
    list_indent: &'a str,
    insert: usize,
}

fn single_task_list_item_context<'a>(
    text: &str,
    task_line_start: usize,
    task_range_end: usize,
    task_indent: &'a str,
) -> Option<ListTaskContext<'a>> {
    let list_indent = task_indent
        .strip_suffix("  ")
        .or_else(|| task_indent.strip_suffix('\t'))?;
    let list_start = containing_list_item_start(text, task_line_start, list_indent, task_indent)?;
    let list_end = list_item_end(text, list_start, list_indent)?;
    let task_end_line_offset = task_range_end.saturating_sub(1);
    if list_end != line_bounds(text, task_end_line_offset).map(|(_, end)| end)? {
        return None;
    }
    if has_indented_content_after(text, list_end, list_indent) {
        return None;
    }
    if count_task_fences(text.get(list_start..list_end)?) != 1 {
        return None;
    }

    Some(ListTaskContext {
        list_indent,
        insert: list_end,
    })
}

fn containing_list_item_start(
    text: &str,
    task_line_start: usize,
    list_indent: &str,
    task_indent: &str,
) -> Option<usize> {
    let (_, current_line_end) = line_bounds(text, task_line_start)?;
    let current_line = text
        .get(task_line_start..current_line_end)?
        .strip_suffix('\r')
        .unwrap_or(text.get(task_line_start..current_line_end)?);
    let current_indent = leading_indent(current_line);
    let current_trimmed = current_line.trim_start();
    if current_indent == list_indent && current_trimmed.starts_with("- ") {
        return Some(task_line_start);
    }

    let mut line_start = task_line_start;
    while let Some(start) = previous_line_start(text, line_start) {
        let (_, line_end) = line_bounds(text, start)?;
        let line = text
            .get(start..line_end)?
            .strip_suffix('\r')
            .unwrap_or(text.get(start..line_end)?);
        if line.trim().is_empty() {
            line_start = start;
            continue;
        }
        let indent = leading_indent(line);
        let trimmed = line.trim_start();
        if indent == list_indent && trimmed.starts_with("- ") {
            return Some(start);
        }
        if indent.len() < task_indent.len() {
            return None;
        }
        line_start = start;
    }
    None
}

fn list_item_end(text: &str, list_start: usize, list_indent: &str) -> Option<usize> {
    let (_, first_end) = line_bounds(text, list_start)?;
    let mut line_start = next_line_start(text, first_end)?;
    let mut last_end = first_end;

    while line_start < text.len() {
        let (_, line_end) = line_bounds(text, line_start)?;
        let line = text
            .get(line_start..line_end)?
            .strip_suffix('\r')
            .unwrap_or(text.get(line_start..line_end)?);
        if !line.trim().is_empty() {
            let indent = leading_indent(line);
            let trimmed = line.trim_start();
            if indent.len() <= list_indent.len()
                && (trimmed.starts_with("- ") || trimmed.starts_with("+ "))
            {
                break;
            }
        }
        last_end = line_end;
        let Some(next) = next_line_start(text, line_end) else {
            break;
        };
        line_start = next;
    }

    Some(last_end)
}

fn has_indented_content_after(text: &str, line_end: usize, list_indent: &str) -> bool {
    let Some(mut line_start) = next_line_start(text, line_end) else {
        return false;
    };
    while line_start < text.len() {
        let Some((_, line_end)) = line_bounds(text, line_start) else {
            return false;
        };
        let Some(line) = text.get(line_start..line_end) else {
            return false;
        };
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.trim().is_empty() {
            line_start = match next_line_start(text, line_end) {
                Some(start) => start,
                None => return false,
            };
            continue;
        }
        let indent = leading_indent(line);
        if indent.len() <= list_indent.len() {
            return false;
        }
        return true;
    }

    false
}

fn count_task_fences(text: &str) -> usize {
    text.lines()
        .filter(|line| {
            let trimmed = line.trim_start();
            trimmed
                .strip_prefix("- ")
                .unwrap_or(trimmed)
                .starts_with("::: task")
        })
        .count()
}

fn previous_line_start(text: &str, line_start: usize) -> Option<usize> {
    if line_start == 0 {
        return None;
    }
    let previous_end = line_start.checked_sub('\n'.len_utf8())?;
    Some(text[..previous_end].rfind('\n').map_or(0, |i| i + 1))
}

fn next_line_start(text: &str, line_end: usize) -> Option<usize> {
    if line_end >= text.len() {
        None
    } else {
        Some(line_end + '\n'.len_utf8())
    }
}

fn ensure_block_indent(block: &str, indent: &str) -> String {
    if indent.is_empty() {
        return block.to_string();
    }

    let mut out = String::new();
    for line in block.split_inclusive('\n') {
        let content = line.trim_end_matches(['\r', '\n']);
        if content.is_empty() || line.starts_with(indent) {
            out.push_str(line);
        } else {
            out.push_str(indent);
            out.push_str(line);
        }
    }
    out
}

fn inherited_task_source(source: &str, indent: &str) -> String {
    filter_recurring_instance_attributes(&ensure_block_indent(source, indent))
}

pub(crate) fn filter_recurring_instance_attributes(source: &str) -> String {
    let mut out = String::new();
    for line in source.split_inclusive('\n') {
        match filter_recurring_attribute_line(line) {
            AttributeLineFilter::Keep(line) => out.push_str(line),
            AttributeLineFilter::Replace(line) => out.push_str(&line),
            AttributeLineFilter::Drop => {}
        }
    }
    out
}

enum AttributeLineFilter<'a> {
    Keep(&'a str),
    Replace(String),
    Drop,
}

fn filter_recurring_attribute_line(line: &str) -> AttributeLineFilter<'_> {
    let line_without_newline = line.trim_end_matches(['\r', '\n']);
    let newline = &line[line_without_newline.len()..];
    let indent = leading_indent(line_without_newline);
    let content = &line_without_newline[indent.len()..];
    let Some(inner) = content.strip_prefix('{').and_then(|s| s.strip_suffix('}')) else {
        return AttributeLineFilter::Keep(line);
    };

    let Some(tokens) = attribute_tokens(inner) else {
        return AttributeLineFilter::Keep(line);
    };
    if tokens.is_empty() {
        return AttributeLineFilter::Keep(line);
    }

    let kept = tokens
        .iter()
        .filter(|token| !is_recurring_instance_attribute(token))
        .collect::<Vec<_>>();
    if kept.len() == tokens.len() {
        return AttributeLineFilter::Keep(line);
    }
    if kept.is_empty() {
        return AttributeLineFilter::Drop;
    }

    let mut replacement = String::new();
    replacement.push_str(indent);
    replacement.push('{');
    for (idx, token) in kept.iter().enumerate() {
        if idx > 0 {
            replacement.push(' ');
        }
        replacement.push_str(token);
    }
    replacement.push('}');
    replacement.push_str(newline);
    AttributeLineFilter::Replace(replacement)
}

fn attribute_tokens(inner: &str) -> Option<Vec<&str>> {
    let mut tokens = Vec::new();
    let mut start = None;
    let mut quote = None;
    let mut escaped = false;

    for (idx, ch) in inner.char_indices() {
        if start.is_none() {
            if ch.is_whitespace() {
                continue;
            }
            start = Some(idx);
        }

        if let Some(quoted) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == quoted {
                quote = None;
            }
            continue;
        }

        if ch == '"' || ch == '\'' {
            quote = Some(ch);
        } else if ch.is_whitespace() {
            if let Some(token_start) = start.take() {
                tokens.push(inner[token_start..idx].trim());
            }
        }
    }

    if quote.is_some() {
        return None;
    }
    if let Some(token_start) = start {
        tokens.push(inner[token_start..].trim());
    }

    Some(
        tokens
            .into_iter()
            .filter(|token| !token.is_empty())
            .collect(),
    )
}

fn is_recurring_instance_attribute(token: &str) -> bool {
    if token.starts_with('#') {
        return true;
    }
    let key = token.split_once('=').map_or(token, |(key, _)| key);
    matches!(
        key,
        "id" | "created" | "done" | "canceled" | "due" | "wait" | "recur" | "prev"
    )
}

pub fn next_recur_due(due: DateTime<FixedOffset>, recur: &str) -> Option<DateTime<FixedOffset>> {
    let rule = parse_repeat_rule(recur)?;
    match rule {
        RepeatRule::Days(days) => Some(due + Duration::days(days)),
        RepeatRule::Weeks(weeks) => Some(due + Duration::weeks(weeks)),
        RepeatRule::Months(months) => add_months(due, months),
        RepeatRule::Years(years) => add_months(due, years.checked_mul(12)?),
    }
}

fn add_months(due: DateTime<FixedOffset>, months: i32) -> Option<DateTime<FixedOffset>> {
    let month0 = due.month0() as i32 + months;
    let year = due.year() + month0.div_euclid(12);
    let month0 = month0.rem_euclid(12);
    let month = (month0 + 1) as u32;
    let day = due.day().min(last_day_of_month(year, month)?);
    due.timezone()
        .with_ymd_and_hms(year, month, day, due.hour(), due.minute(), due.second())
        .single()
}

fn last_day_of_month(year: i32, month: u32) -> Option<u32> {
    let (next_year, next_month) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    let first_next = chrono::NaiveDate::from_ymd_opt(next_year, next_month, 1)?;
    Some((first_next - Duration::days(1)).day())
}

fn task_instance_id(
    title: &str,
    due: DateTime<FixedOffset>,
    anchors: &HashMap<String, Anchor>,
    reserved: &HashSet<String>,
) -> Option<String> {
    let base = djot_heading_id(title)?;
    let date = due.format("%Y-%m-%d");
    let candidate = format!("{base}-{date}");
    Some(unique_anchor_id(candidate, anchors, reserved))
}

fn djot_heading_id(title: &str) -> Option<String> {
    let source = format!("# {}\n", title.trim());
    Parser::new(&source).find_map(|event| match event {
        Event::Start(Container::Heading { id, .. }, _) => Some(id.into_owned()),
        _ => None,
    })
}

fn unique_anchor_id(
    candidate: String,
    anchors: &HashMap<String, Anchor>,
    reserved: &HashSet<String>,
) -> String {
    if !anchors.contains_key(&candidate) && !reserved.contains(&candidate) {
        return candidate;
    }
    let mut count = 2;
    loop {
        let id = format!("{candidate}-{count}");
        if !anchors.contains_key(&id) && !reserved.contains(&id) {
            return id;
        }
        count += 1;
    }
}

pub(crate) fn leading_indent(line: &str) -> &str {
    let indent_len = line
        .char_indices()
        .find(|(_, c)| *c != ' ' && *c != '\t')
        .map(|(i, _)| i)
        .unwrap_or(line.len());
    &line[..indent_len]
}

fn escape_attribute_value(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

pub(crate) fn anchor_attribute(id: &str) -> String {
    if is_shorthand_anchor_id(id) {
        format!("{{#{id}}}")
    } else {
        format!("{{id=\"{}\"}}", escape_attribute_value(id))
    }
}

fn is_shorthand_anchor_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b':' | b'_' | b'-'))
}

struct TaskOpeningFence {
    line_start: usize,
    attribute_insert: Range<usize>,
    attribute_prefix: String,
    continued_attribute_prefix: String,
    fence_prefix: String,
    task_indent: String,
}

fn task_opening_fence(text: &str, range: &Range<usize>) -> Option<TaskOpeningFence> {
    let mut offset = range.start;
    while offset <= range.end {
        let (line_start, line_end) = line_bounds(text, offset)?;
        let line = text.get(line_start..line_end)?;
        if let Some(opening) = task_opening_fence_from_line(line_start, line) {
            return Some(opening);
        }
        if line_end >= range.end || line_end == text.len() {
            break;
        }
        offset = line_end + '\n'.len_utf8();
    }
    None
}

fn task_opening_fence_from_line(line_start: usize, line: &str) -> Option<TaskOpeningFence> {
    let line = line.strip_suffix('\r').unwrap_or(line);
    let indent = leading_indent(line);
    let rest = &line[indent.len()..];
    if rest.starts_with("::: task") {
        return Some(TaskOpeningFence {
            line_start,
            attribute_insert: line_start..line_start,
            attribute_prefix: indent.to_string(),
            continued_attribute_prefix: indent.to_string(),
            fence_prefix: String::new(),
            task_indent: indent.to_string(),
        });
    }

    let fence = rest.strip_prefix("- ")?;
    if !fence.starts_with("::: task") {
        return None;
    }
    Some(TaskOpeningFence {
        line_start,
        attribute_insert: line_start..line_start + indent.len() + "- ".len(),
        attribute_prefix: format!("{indent}- "),
        continued_attribute_prefix: format!("{indent}  "),
        fence_prefix: format!("{indent}  "),
        task_indent: format!("{indent}  "),
    })
}

pub(crate) fn line_bounds(text: &str, offset: usize) -> Option<(usize, usize)> {
    if offset > text.len() {
        return None;
    }
    let start = text[..offset].rfind('\n').map_or(0, |i| i + 1);
    let end = text[offset..].find('\n').map_or(text.len(), |i| offset + i);
    Some((start, end))
}

pub fn parse_repeat_rule(recur: &str) -> Option<RepeatRule> {
    let duration: IsoDuration = recur.parse().ok()?;
    let units = [
        duration.year,
        duration.month,
        duration.day,
        duration.hour,
        duration.minute,
        duration.second,
    ];
    if units.iter().filter(|value| **value > 0.0).count() != 1 {
        return None;
    }
    if duration.hour > 0.0 || duration.minute > 0.0 || duration.second > 0.0 {
        return None;
    }
    if duration.year > 0.0 {
        return integer_f32(duration.year).and_then(|years| {
            i32::try_from(years)
                .ok()
                .filter(|years| *years > 0)
                .map(RepeatRule::Years)
        });
    }
    if duration.month > 0.0 {
        return integer_f32(duration.month).and_then(|months| {
            i32::try_from(months)
                .ok()
                .filter(|months| *months > 0)
                .map(RepeatRule::Months)
        });
    }
    integer_f32(duration.day).and_then(|days| {
        if days > 0 && days % 7 == 0 {
            Some(RepeatRule::Weeks(days / 7))
        } else if days > 0 {
            Some(RepeatRule::Days(days))
        } else {
            None
        }
    })
}

fn integer_f32(value: f32) -> Option<i64> {
    if value.fract() == 0.0 && value <= i64::MAX as f32 {
        Some(value as i64)
    } else {
        None
    }
}
