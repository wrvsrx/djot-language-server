use djot_core::{metadata_block, Heading};
use lsp_types::{DocumentSymbol, SymbolKind};

use crate::position::byte_range_to_lsp;

pub(crate) fn document_title(text: &str) -> Option<String> {
    let metadata = metadata_block(text)?;
    let value: toml::Value = toml::from_str(&metadata).ok()?;
    value
        .get("title")
        .and_then(|title| title.as_str())
        .map(str::to_string)
}

/// Convert a core [`Heading`] (byte offsets) into an LSP `DocumentSymbol`.
pub(crate) fn to_document_symbol(text: &str, heading: &Heading) -> DocumentSymbol {
    let children: Vec<_> = heading
        .children
        .iter()
        .map(|child| to_document_symbol(text, child))
        .collect();
    #[allow(deprecated)]
    DocumentSymbol {
        name: if heading.name.is_empty() {
            format!("H{}", heading.level)
        } else {
            heading.name.clone()
        },
        detail: Some(format!("H{}", heading.level)),
        kind: SymbolKind::STRING,
        tags: None,
        deprecated: None,
        range: byte_range_to_lsp(text, &heading.range),
        selection_range: byte_range_to_lsp(text, &heading.selection_range),
        children: if children.is_empty() {
            None
        } else {
            Some(children)
        },
    }
}
