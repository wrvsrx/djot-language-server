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
