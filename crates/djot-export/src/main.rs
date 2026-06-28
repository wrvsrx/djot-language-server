//! `djot-export`: convert a djot document to a [pandoc] JSON AST on stdout, so
//! it can be piped into pandoc (`djot-export doc.dj | pandoc -f json -o doc.pdf`).
//!
//! Pandoc's native djot reader owns the syntax conversion. This binary applies
//! `djot-tools` export semantics on top of the resulting Pandoc AST:
//!
//! - the first `{.metadata}` TOML code block is folded into Pandoc metadata and
//!   removed from the rendered body;
//! - every `[X]{.cite}` span is rewritten into a Pandoc `Cite` node, where `X`
//!   is treated exactly as the body of a pandoc-markdown citation bracket
//!   (`[X]`). The parsing is delegated back to pandoc so the supported forms
//!   (`[@k]`, `[-@k]`, `[@k, p. 3]`, `[see @k]`, `[@a; @b]`) stay identical to
//!   pandoc-markdown. A downstream `pandoc --citeproc` then resolves them.
//!
//! [pandoc]: https://pandoc.org

use std::collections::HashMap;
use std::io::{Read, Write};
use std::process::{Command, ExitCode, Stdio};

use pandoc_types::definition::{Attr, Block, MetaValue, Pandoc};
use serde_json::Value;

/// Span class that marks a citation, e.g. `[@smith2004]{.cite}`. Export-only.
const CITE_CLASS: &str = "cite";

fn main() -> ExitCode {
    let input = match read_input() {
        Ok(input) => input,
        Err(err) => {
            eprintln!("djot-export: {err}");
            return ExitCode::FAILURE;
        }
    };

    match to_pandoc_json(&input) {
        Ok(json) => {
            println!("{json}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("djot-export: {err}");
            ExitCode::FAILURE
        }
    }
}

fn read_input() -> Result<String, String> {
    match std::env::args().nth(1) {
        Some(path) => {
            std::fs::read_to_string(&path).map_err(|err| format!("cannot read {path}: {err}"))
        }
        None => {
            let mut buf = String::new();
            std::io::stdin()
                .read_to_string(&mut buf)
                .map_err(|err| format!("cannot read stdin: {err}"))?;
            Ok(buf)
        }
    }
}

/// Convert djot `text` into a Pandoc JSON AST document.
fn to_pandoc_json(text: &str) -> Result<String, String> {
    let json = run_pandoc(&["-f", "djot", "-t", "json"], text)?;
    let mut value: Value =
        serde_json::from_str(&json).map_err(|err| format!("cannot parse pandoc JSON: {err}"))?;

    let mut texts = Vec::new();
    collect_cite_texts(&value, &mut texts);
    if !texts.is_empty() {
        let cites = parse_citations_via_pandoc(&texts)?;
        let mut idx = 0;
        replace_cite_spans(&mut value, &cites, &mut idx);
    }

    let mut document: Pandoc = serde_json::from_value(value)
        .map_err(|err| format!("cannot parse pandoc JSON: {err}"))?;
    fold_metadata_block(&mut document);
    serde_json::to_string(&document).map_err(|err| format!("cannot write pandoc JSON: {err}"))
}

/// Run `pandoc` with `args`, feeding `input` on stdin, and return its stdout.
fn run_pandoc(args: &[&str], input: &str) -> Result<String, String> {
    let mut child = Command::new("pandoc")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("cannot run pandoc: {err}"))?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "cannot open pandoc stdin".to_string())?;
    stdin
        .write_all(input.as_bytes())
        .map_err(|err| format!("cannot write to pandoc: {err}"))?;
    drop(stdin);

    let output = child
        .wait_with_output()
        .map_err(|err| format!("cannot wait for pandoc: {err}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let message = stderr.trim();
        return Err(if message.is_empty() {
            format!("pandoc exited with {}", output.status)
        } else {
            format!("pandoc exited with {}: {message}", output.status)
        });
    }

    String::from_utf8(output.stdout).map_err(|err| format!("pandoc wrote non-UTF-8 JSON: {err}"))
}

/// If `value` is a `[X]{.cite}` span, return its inline-content `Value` (`X`).
fn cite_span_content(value: &Value) -> Option<&Value> {
    let object = value.as_object()?;
    if object.get("t")?.as_str()? != "Span" {
        return None;
    }
    let content = object.get("c")?.as_array()?;
    let classes = content.first()?.as_array()?.get(1)?.as_array()?;
    if classes.iter().any(|class| class.as_str() == Some(CITE_CLASS)) {
        content.get(1)
    } else {
        None
    }
}

/// Collect the citation body text of every `.cite` span, in document order.
fn collect_cite_texts(value: &Value, out: &mut Vec<String>) {
    if let Some(content) = cite_span_content(value) {
        let mut text = String::new();
        inline_text(content, &mut text);
        out.push(text.trim().to_string());
        return;
    }
    match value {
        Value::Array(items) => items.iter().for_each(|item| collect_cite_texts(item, out)),
        Value::Object(map) => map.values().for_each(|item| collect_cite_texts(item, out)),
        _ => {}
    }
}

/// Flatten inline `Value`s to their plain text, joining words with spaces.
fn inline_text(value: &Value, out: &mut String) {
    match value {
        Value::Array(items) => items.iter().for_each(|item| inline_text(item, out)),
        Value::Object(map) => match map.get("t").and_then(Value::as_str) {
            Some("Str") => {
                if let Some(text) = map.get("c").and_then(Value::as_str) {
                    out.push_str(text);
                }
            }
            Some("Space" | "SoftBreak" | "LineBreak") => out.push(' '),
            _ => {
                if let Some(child) = map.get("c") {
                    inline_text(child, out);
                }
            }
        },
        _ => {}
    }
}

/// Replace each `.cite` span with the matching parsed `Cite` node, in order.
/// A `None` entry (body was not a valid citation) leaves the span unchanged.
fn replace_cite_spans(value: &mut Value, cites: &[Option<Value>], idx: &mut usize) {
    if cite_span_content(value).is_some() {
        if let Some(Some(cite)) = cites.get(*idx) {
            *value = cite.clone();
        }
        *idx += 1;
        return;
    }
    match value {
        Value::Array(items) => items
            .iter_mut()
            .for_each(|item| replace_cite_spans(item, cites, idx)),
        Value::Object(map) => map
            .values_mut()
            .for_each(|item| replace_cite_spans(item, cites, idx)),
        _ => {}
    }
}

/// Find the first `Cite` inline anywhere inside a block `Value`.
fn extract_cite_from_block(block: &Value) -> Option<Value> {
    fn find(value: &Value) -> Option<Value> {
        if let Value::Object(map) = value {
            if map.get("t").and_then(Value::as_str) == Some("Cite") {
                return Some(value.clone());
            }
        }
        match value {
            Value::Array(items) => items.iter().find_map(find),
            Value::Object(map) => map.values().find_map(find),
            _ => None,
        }
    }
    find(block)
}

/// Parse each citation body `X` by handing `[X]` back to pandoc's markdown
/// reader, returning one `Cite` `Value` per input (or `None` if `X` is not a
/// citation). Order matches `texts`.
fn parse_citations_via_pandoc(texts: &[String]) -> Result<Vec<Option<Value>>, String> {
    let markdown = texts
        .iter()
        .map(|text| format!("[{}]", text.replace('\n', " ")))
        .collect::<Vec<_>>()
        .join("\n\n");
    let json = run_pandoc(&["-f", "markdown", "-t", "json"], &markdown)?;
    let document: Value = serde_json::from_str(&json)
        .map_err(|err| format!("cannot parse pandoc citation JSON: {err}"))?;
    let blocks = document
        .get("blocks")
        .and_then(Value::as_array)
        .ok_or_else(|| "pandoc citation output has no blocks".to_string())?;
    if blocks.len() != texts.len() {
        return Err(format!(
            "expected {} citation blocks from pandoc, got {}",
            texts.len(),
            blocks.len()
        ));
    }
    let cites: Vec<Option<Value>> = blocks.iter().map(extract_cite_from_block).collect();
    for (text, cite) in texts.iter().zip(&cites) {
        if cite.is_none() {
            eprintln!("djot-export: warning: .cite span is not a valid citation: [{text}]");
        }
    }
    Ok(cites)
}

fn fold_metadata_block(document: &mut Pandoc) {
    let mut found = None;
    document.blocks.retain(|block| {
        if found.is_none() {
            if let Block::CodeBlock(attr, text) = block {
                if has_class(attr, djot_core::METADATA_CLASS) {
                    found = Some(text.clone());
                    return false;
                }
            }
        }
        true
    });

    let Some(metadata) = found else {
        return;
    };
    let Ok(table) = toml::from_str::<toml::Table>(&metadata) else {
        return;
    };
    for (key, value) in table {
        document.meta.insert(key, toml_to_meta(value));
    }
}

fn has_class(attr: &Attr, class: &str) -> bool {
    attr.classes.iter().any(|candidate| candidate == class)
}

fn toml_to_meta(value: toml::Value) -> MetaValue {
    match value {
        toml::Value::String(s) => MetaValue::MetaString(s),
        toml::Value::Boolean(b) => MetaValue::MetaBool(b),
        toml::Value::Integer(n) => MetaValue::MetaString(n.to_string()),
        toml::Value::Float(n) => MetaValue::MetaString(n.to_string()),
        toml::Value::Datetime(d) => MetaValue::MetaString(d.to_string()),
        toml::Value::Array(items) => {
            MetaValue::MetaList(items.into_iter().map(toml_to_meta).collect())
        }
        toml::Value::Table(table) => MetaValue::MetaMap(
            table
                .into_iter()
                .map(|(key, value)| (key, toml_to_meta(value)))
                .collect::<HashMap<_, _>>(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pandoc_types::definition::Inline;

    #[test]
    fn metadata_is_folded_into_meta_and_removed_from_body() {
        let mut document = Pandoc {
            meta: HashMap::new(),
            blocks: vec![
                Block::CodeBlock(
                    Attr {
                        identifier: String::new(),
                        classes: vec!["metadata".to_string(), "toml".to_string()],
                        attributes: Vec::new(),
                    },
                    "title = \"X\"\ndraft = true\n".to_string(),
                ),
                Block::Header(1, Attr::default(), vec![Inline::Str("Heading".to_string())]),
            ],
        };

        fold_metadata_block(&mut document);

        assert_eq!(
            document.meta.get("title"),
            Some(&MetaValue::MetaString("X".to_string()))
        );
        assert_eq!(document.meta.get("draft"), Some(&MetaValue::MetaBool(true)));
        assert!(matches!(document.blocks.as_slice(), [Block::Header(..)]));
    }

    #[test]
    fn invalid_metadata_is_removed_without_failing() {
        let mut document = Pandoc {
            meta: HashMap::new(),
            blocks: vec![Block::CodeBlock(
                Attr {
                    identifier: String::new(),
                    classes: vec!["metadata".to_string()],
                    attributes: Vec::new(),
                },
                "not = = toml\n".to_string(),
            )],
        };

        fold_metadata_block(&mut document);

        assert!(document.meta.is_empty());
        assert!(document.blocks.is_empty());
    }

    #[test]
    fn non_metadata_code_block_is_kept() {
        let mut document = Pandoc {
            meta: HashMap::new(),
            blocks: vec![Block::CodeBlock(
                Attr {
                    identifier: String::new(),
                    classes: vec!["toml".to_string()],
                    attributes: Vec::new(),
                },
                "title = \"X\"\n".to_string(),
            )],
        };

        fold_metadata_block(&mut document);

        assert!(document.meta.is_empty());
        assert!(matches!(document.blocks.as_slice(), [Block::CodeBlock(..)]));
    }

    use serde_json::json;

    /// A `[X]{.cite}` span `Value` whose inline content is a single `Str`.
    fn cite_span(text: &str) -> Value {
        json!({"t": "Span", "c": [["", ["cite"], []], [{"t": "Str", "c": text}]]})
    }

    #[test]
    fn collect_finds_nested_cite_text_in_order() {
        // Two cite spans, the first nested inside an Emph, plus a plain span.
        let document = json!({
            "blocks": [{"t": "Para", "c": [
                {"t": "Emph", "c": [cite_span("@smith2004")]},
                {"t": "Str", "c": "and"},
                cite_span("@doe2010"),
            ]}]
        });

        let mut texts = Vec::new();
        collect_cite_texts(&document, &mut texts);

        assert_eq!(texts, vec!["@smith2004".to_string(), "@doe2010".to_string()]);
    }

    #[test]
    fn span_without_cite_class_is_not_a_citation() {
        let span = json!({"t": "Span", "c": [["", ["aside"], []], [{"t": "Str", "c": "x"}]]});
        assert!(cite_span_content(&span).is_none());
    }

    #[test]
    fn replace_swaps_cites_and_leaves_invalid_spans() {
        let mut document = json!({
            "blocks": [{"t": "Para", "c": [
                cite_span("@smith2004"),
                {"t": "Str", "c": "between"},
                cite_span("not a cite"),
            ]}]
        });
        let cite = json!({"t": "Cite", "c": [[], [{"t": "Str", "c": "(Smith 2004)"}]]});
        let cites = vec![Some(cite.clone()), None];

        let mut idx = 0;
        replace_cite_spans(&mut document, &cites, &mut idx);

        let inlines = &document["blocks"][0]["c"];
        assert_eq!(idx, 2);
        assert_eq!(inlines[0], cite); // first span became the Cite
        assert_eq!(inlines[1], json!({"t": "Str", "c": "between"})); // untouched
        assert_eq!(inlines[2], cite_span("not a cite")); // None left as-is
    }

    #[test]
    fn extract_cite_pulls_cite_out_of_a_paragraph() {
        let cite = json!({"t": "Cite", "c": [[], [{"t": "Str", "c": "(Smith 2004)"}]]});
        let para = json!({"t": "Para", "c": [cite.clone()]});
        assert_eq!(extract_cite_from_block(&para), Some(cite));

        let plain = json!({"t": "Para", "c": [{"t": "Str", "c": "[foo]"}]});
        assert_eq!(extract_cite_from_block(&plain), None);
    }
}
