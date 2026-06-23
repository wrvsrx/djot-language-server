use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

pub(crate) fn relative_link_path(from: &Path, target: &Path) -> Option<String> {
    let base = from.parent()?;
    Some(relative_path(base, target)?.display().to_string())
}

fn relative_path(base: &Path, target: &Path) -> Option<PathBuf> {
    let base_components = lexical_components(base)?;
    let target_components = lexical_components(target)?;

    if base_components.first() != target_components.first() {
        return None;
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

    Some(if out.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        out
    })
}

fn lexical_components(path: &Path) -> Option<Vec<OsString>> {
    let mut out = Vec::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop()?;
            }
            Component::Normal(part) => out.push(part.to_os_string()),
            Component::RootDir => out.push(OsString::from(std::path::MAIN_SEPARATOR.to_string())),
            Component::Prefix(prefix) => out.push(prefix.as_os_str().to_os_string()),
        }
    }
    Some(out)
}

pub(crate) fn is_djot_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext == "dj" || ext == "djot")
}
