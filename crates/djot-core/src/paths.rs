use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

/// Lexically normalize a path (resolve `.`/`..` without touching the
/// filesystem), so equal logical paths compare equal as index keys.
pub(crate) fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

pub(crate) fn is_djot_path(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext == "dj" || ext == "djot")
}

pub(crate) fn is_djot_file_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext == "dj" || ext == "djot")
}

pub(crate) fn relative_link_path(from_file: &Path, target: &Path) -> String {
    relative_path(from_file.parent().unwrap_or_else(|| Path::new("")), target)
        .display()
        .to_string()
}

fn relative_path(base: &Path, target: &Path) -> PathBuf {
    let base = normalize(base);
    let target = normalize(target);
    let base_components = path_components(&base);
    let target_components = path_components(&target);

    if base_components.first() != target_components.first() {
        return target;
    }

    let common_len = base_components
        .iter()
        .zip(target_components.iter())
        .take_while(|(base, target)| base == target)
        .count();

    let mut out = PathBuf::new();
    for _ in common_len..base_components.len() {
        out.push("..");
    }
    for component in &target_components[common_len..] {
        out.push(component);
    }

    if out.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        out
    }
}

fn path_components(path: &Path) -> Vec<OsString> {
    path.components()
        .filter_map(|component| match component {
            Component::CurDir => None,
            Component::ParentDir => Some(OsString::from("..")),
            Component::Normal(part) => Some(part.to_os_string()),
            Component::RootDir => Some(OsString::from(std::path::MAIN_SEPARATOR.to_string())),
            Component::Prefix(prefix) => Some(prefix.as_os_str().to_os_string()),
        })
        .collect()
}

pub(crate) fn percent_decode_path(path: &str) -> String {
    let mut decoded = Vec::with_capacity(path.len());
    let bytes = path.as_bytes();
    let mut cursor = 0;
    while cursor < bytes.len() {
        if bytes[cursor] == b'%' && cursor + 2 < bytes.len() {
            if let Some(byte) = hex_byte(bytes[cursor + 1], bytes[cursor + 2]) {
                decoded.push(byte);
                cursor += 3;
                continue;
            }
        }
        decoded.push(bytes[cursor]);
        cursor += 1;
    }
    String::from_utf8(decoded).unwrap_or_else(|_| path.to_string())
}

fn hex_byte(high: u8, low: u8) -> Option<u8> {
    Some(hex_digit(high)? * 16 + hex_digit(low)?)
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}
