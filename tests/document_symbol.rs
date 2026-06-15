//! End-to-end test: spawn the built `djot-ls` binary and drive a real
//! JSON-RPC session over stdio, asserting on the `textDocument/documentSymbol`
//! response.

use std::io::{Read, Write};
use std::process::{Command, Stdio};

use serde_json::{json, Value};

/// Wrap a JSON value in an LSP `Content-Length` frame.
fn frame(v: &Value) -> Vec<u8> {
    let body = serde_json::to_vec(v).unwrap();
    let mut out = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
    out.extend_from_slice(&body);
    out
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Split a stream of `Content-Length`-framed messages into JSON values.
fn parse_frames(mut data: &[u8]) -> Vec<Value> {
    let mut msgs = Vec::new();
    while let Some(pos) = find(data, b"\r\n\r\n") {
        let header = std::str::from_utf8(&data[..pos]).unwrap();
        let len: usize = header
            .lines()
            .find_map(|l| l.strip_prefix("Content-Length: "))
            .expect("missing Content-Length")
            .trim()
            .parse()
            .unwrap();
        let start = pos + 4;
        let body = &data[start..start + len];
        msgs.push(serde_json::from_slice(body).unwrap());
        data = &data[start + len..];
    }
    msgs
}

#[test]
fn document_symbol_returns_headings() {
    let doc = "# Title\n\nsome text\n\n## Section A\n\nmore\n\n### Sub\n";
    let msgs = [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{},"processId":null,"rootUri":null}}),
        json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":"file:///t.dj","languageId":"djot","version":1,"text":doc}}}),
        json!({"jsonrpc":"2.0","id":2,"method":"textDocument/documentSymbol","params":{"textDocument":{"uri":"file:///t.dj"}}}),
        json!({"jsonrpc":"2.0","id":3,"method":"shutdown","params":null}),
        json!({"jsonrpc":"2.0","method":"exit","params":null}),
    ];

    let mut payload = Vec::new();
    for m in &msgs {
        payload.extend_from_slice(&frame(m));
    }

    let mut child = Command::new(env!("CARGO_BIN_EXE_djot-ls"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    // The payload is tiny, so writing it all before reading cannot deadlock.
    child.stdin.take().unwrap().write_all(&payload).unwrap();
    let mut out = Vec::new();
    child.stdout.take().unwrap().read_to_end(&mut out).unwrap();
    child.wait().unwrap();

    let responses = parse_frames(&out);
    let result = responses
        .iter()
        .find(|m| m["id"] == json!(2))
        .expect("no documentSymbol response")["result"]
        .as_array()
        .expect("result is not an array")
        .clone();

    let names: Vec<&str> = result.iter().map(|s| s["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Title", "Section A", "Sub"]);

    let details: Vec<&str> = result.iter().map(|s| s["detail"].as_str().unwrap()).collect();
    assert_eq!(details, ["H1", "H2", "H3"]);

    // Spot-check the range of the first heading (line 0).
    assert_eq!(result[0]["range"]["start"]["line"], json!(0));
}
