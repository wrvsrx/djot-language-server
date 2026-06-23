use std::ops::Range;

use djot_core::{NativeTaskListItem, Task};
use lsp_types::{
    SemanticToken, SemanticTokenModifier, SemanticTokenType, SemanticTokens,
    SemanticTokensFullOptions, SemanticTokensLegend, SemanticTokensOptions,
    SemanticTokensServerCapabilities, WorkDoneProgressOptions,
};

use crate::position::offset_to_position;

const TASK_TOKEN_TYPE_INDEX: u32 = 0;
const COMPLETED_MODIFIER_BITSET: u32 = 1;

pub(crate) fn semantic_tokens_provider() -> SemanticTokensServerCapabilities {
    SemanticTokensOptions {
        work_done_progress_options: WorkDoneProgressOptions::default(),
        legend: SemanticTokensLegend {
            token_types: vec![SemanticTokenType::new("task")],
            token_modifiers: vec![SemanticTokenModifier::new("completed")],
        },
        range: None,
        full: Some(SemanticTokensFullOptions::Bool(true)),
    }
    .into()
}

pub(crate) fn task_semantic_tokens(
    text: &str,
    tasks: &[Task],
    native_task_list_items: &[NativeTaskListItem],
) -> SemanticTokens {
    let ranges = completed_task_ranges(tasks, native_task_list_items);
    let mut absolute = ranges
        .iter()
        .flat_map(|range| token_ranges_for_byte_range(text, range))
        .collect::<Vec<_>>();
    absolute.sort_unstable();

    let mut previous_line = 0;
    let mut previous_start = 0;
    let data = absolute
        .into_iter()
        .map(|(line, start, length)| {
            let delta_line = line - previous_line;
            let delta_start = if delta_line == 0 {
                start - previous_start
            } else {
                start
            };
            previous_line = line;
            previous_start = start;
            SemanticToken {
                delta_line,
                delta_start,
                length,
                token_type: TASK_TOKEN_TYPE_INDEX,
                token_modifiers_bitset: COMPLETED_MODIFIER_BITSET,
            }
        })
        .collect();

    SemanticTokens {
        result_id: None,
        data,
    }
}

fn completed_task_ranges(
    tasks: &[Task],
    native_task_list_items: &[NativeTaskListItem],
) -> Vec<Range<usize>> {
    let mut ranges = tasks
        .iter()
        .filter(|task| task.done.is_some() || task.canceled.is_some())
        .map(|task| task.range.clone())
        .chain(
            native_task_list_items
                .iter()
                .filter(|item| item.checked)
                .map(|item| item.range.clone()),
        )
        .collect::<Vec<_>>();
    ranges.sort_by_key(|range| range.start);

    let mut merged: Vec<Range<usize>> = Vec::new();
    for range in ranges {
        if let Some(previous) = merged.last_mut() {
            if range.start <= previous.end {
                previous.end = previous.end.max(range.end);
                continue;
            }
        }
        merged.push(range);
    }
    merged
}

fn token_ranges_for_byte_range(text: &str, range: &Range<usize>) -> Vec<(u32, u32, u32)> {
    let start = offset_to_position(text, range.start);
    let end = offset_to_position(text, range.end);
    if start.line == end.line {
        return non_empty_token(start.line, start.character, end.character)
            .into_iter()
            .collect();
    }

    let line_lengths = line_lengths_utf16(text);
    let mut tokens = Vec::new();
    for line in start.line..=end.line {
        let start_character = if line == start.line {
            start.character
        } else {
            0
        };
        let end_character = if line == end.line {
            end.character
        } else {
            *line_lengths.get(line as usize).unwrap_or(&0)
        };
        if let Some(token) = non_empty_token(line, start_character, end_character) {
            tokens.push(token);
        }
    }
    tokens
}

fn non_empty_token(line: u32, start_character: u32, end_character: u32) -> Option<(u32, u32, u32)> {
    if start_character == end_character {
        return None;
    }
    Some((line, start_character, end_character - start_character))
}

fn line_lengths_utf16(text: &str) -> Vec<u32> {
    let mut lengths = vec![0];
    for c in text.chars() {
        if c == '\n' {
            lengths.push(0);
        } else if let Some(length) = lengths.last_mut() {
            *length += c.len_utf16() as u32;
        }
    }
    lengths
}
