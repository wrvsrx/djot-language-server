use crate::cst::leading_indent;

pub(crate) fn escape_attribute_value(value: &str) -> String {
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
    if !(content.starts_with('{') && content.ends_with('}')) {
        return AttributeLineFilter::Keep(line);
    }

    let brace_range = indent.len()..line_without_newline.len();
    let tokens = crate::cst::attribute_block(line_without_newline, &brace_range);
    if tokens.is_empty() {
        return AttributeLineFilter::Keep(line);
    }

    let kept = tokens
        .iter()
        .map(|token| &line_without_newline[token.token_range.clone()])
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
