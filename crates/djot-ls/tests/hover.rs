//! End-to-end tests for `textDocument/hover`.

mod support;

use serde_json::{json, Value};

use support::{dir_uri, file_uri, response_result, run_session, temp_dir};

#[test]
fn hover_shows_link_target_heading() {
    let dir = temp_dir("djot-ls-hover-heading-test");
    let a = dir.join("a.dj");
    let b = dir.join("b.dj");
    let doc_a = "# A\n\nsee [topic](b.dj#Topic)\n";
    std::fs::write(&a, doc_a).unwrap();
    std::fs::write(&b, "# Intro\n\n## Topic\n\nbody\nmore body\n").unwrap();

    let root_uri = dir_uri(&dir);
    let a_uri = file_uri(&a);
    let link_col = doc_a.lines().nth(2).unwrap().find("b.dj").unwrap() as i64;
    let msgs = [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{},"processId":null,"rootUri":root_uri}}),
        json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
        json!({"jsonrpc":"2.0","id":2,"method":"textDocument/hover",
               "params":{"textDocument":{"uri":a_uri},"position":{"line":2,"character":link_col}}}),
        json!({"jsonrpc":"2.0","id":99,"method":"shutdown","params":null}),
        json!({"jsonrpc":"2.0","method":"exit","params":null}),
    ];

    let responses = run_session(&msgs);
    let value = hover_value(&responses, 2);

    assert!(value.contains("**Heading** `Topic`"));
    assert!(value.contains("`b.dj:3`"));
    assert!(value.contains("## Topic"));
    assert!(value.contains("body\nmore body"));
}

#[test]
fn hover_shows_link_target_file() {
    let dir = temp_dir("djot-ls-hover-file-test");
    let a = dir.join("a.dj");
    let b = dir.join("b.dj");
    let doc_a = "# A\n\nsee [file](b.dj)\n";
    std::fs::write(&a, doc_a).unwrap();
    std::fs::write(&b, "\n# Target File\n\nbody\nmore body\n").unwrap();

    let root_uri = dir_uri(&dir);
    let a_uri = file_uri(&a);
    let link_col = doc_a.lines().nth(2).unwrap().find("b.dj").unwrap() as i64;
    let msgs = [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{},"processId":null,"rootUri":root_uri}}),
        json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
        json!({"jsonrpc":"2.0","id":2,"method":"textDocument/hover",
               "params":{"textDocument":{"uri":a_uri},"position":{"line":2,"character":link_col}}}),
        json!({"jsonrpc":"2.0","id":99,"method":"shutdown","params":null}),
        json!({"jsonrpc":"2.0","method":"exit","params":null}),
    ];

    let responses = run_session(&msgs);
    let value = hover_value(&responses, 2);

    assert!(value.contains("**File**"));
    assert!(value.contains("`b.dj:2`"));
    assert!(value.contains("# Target File"));
    assert!(value.contains("body\nmore body"));
}

#[test]
fn hover_shows_explicit_anchor_target() {
    let doc =
        "# A\n\nsee [note](#important-note)\n\n{#important-note}\nImportant text.\nMore text.\n";
    let msgs = [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{},"processId":null,"rootUri":null}}),
        json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":"file:///a.dj","languageId":"djot","version":1,"text":doc}}}),
        json!({"jsonrpc":"2.0","id":2,"method":"textDocument/hover",
               "params":{"textDocument":{"uri":"file:///a.dj"},"position":{"line":2,"character":12}}}),
        json!({"jsonrpc":"2.0","id":99,"method":"shutdown","params":null}),
        json!({"jsonrpc":"2.0","method":"exit","params":null}),
    ];

    let responses = run_session(&msgs);
    let value = hover_value(&responses, 2);

    assert!(value.contains("**Anchor** `important-note`"));
    assert!(value.contains("`/a.dj:5`"));
    assert!(value.contains("{#important-note}"));
    assert!(value.contains("Important text.\nMore text."));
}

#[test]
fn hover_shows_task_summary() {
    let doc = "{#draft}\n::: task\nDraft.\n:::\n\n{#review created=\"2026-06-21T09:00:00Z\" due=\"2026-06-22T09:00:00Z\" wait=\"2026-06-21T12:00:00Z\" depends=\"#draft\"}\n::: task\nReview draft.\n:::\n";
    let msgs = [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{},"processId":null,"rootUri":null}}),
        json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":"file:///tasks.dj","languageId":"djot","version":1,"text":doc}}}),
        json!({"jsonrpc":"2.0","id":2,"method":"textDocument/hover",
               "params":{"textDocument":{"uri":"file:///tasks.dj"},"position":{"line":6,"character":3}}}),
        json!({"jsonrpc":"2.0","id":99,"method":"shutdown","params":null}),
        json!({"jsonrpc":"2.0","method":"exit","params":null}),
    ];

    let responses = run_session(&msgs);
    let value = hover_value(&responses, 2);

    assert!(value.contains("**Task** `Review draft.`"));
    assert!(value.contains("status: `blocked`"));
    assert!(value.contains("id: `review`"));
    assert!(value.contains("created: `2026-06-21T09:00:00Z`"));
    assert!(value.contains("due: `2026-06-22T09:00:00Z`"));
    assert!(value.contains("wait: `2026-06-21T12:00:00Z`"));
    assert!(value.contains("depends: `#draft`"));
    assert!(value.contains("blocked by: `tasks.dj#draft`"));
}

fn hover_value(responses: &[Value], id: i64) -> String {
    response_result(responses, id)["contents"]["value"]
        .as_str()
        .expect("hover value is not a string")
        .to_string()
}
