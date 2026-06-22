use std::collections::HashMap;

use chrono::{Local, SecondsFormat};
use djot_core::TextEdit as CoreTextEdit;
use lsp_types::{TextEdit, Url, WorkspaceEdit};

use crate::lsp_utils::byte_range_to_lsp;

pub(crate) struct EditContext {
    timestamp: String,
}

impl EditContext {
    pub(crate) fn now() -> Self {
        Self {
            timestamp: Local::now().to_rfc3339_opts(SecondsFormat::Secs, false),
        }
    }

    pub(crate) fn timestamp(&self) -> &str {
        &self.timestamp
    }

    pub(crate) fn lsp_text_edit(source_text: &str, edit: CoreTextEdit) -> TextEdit {
        TextEdit::new(byte_range_to_lsp(source_text, &edit.range), edit.new_text)
    }

    pub(crate) fn single_document_workspace_edit(
        uri: Url,
        source_text: &str,
        edits: Vec<CoreTextEdit>,
    ) -> WorkspaceEdit {
        WorkspaceEdit::new(HashMap::from([(
            uri,
            edits
                .into_iter()
                .map(|edit| Self::lsp_text_edit(source_text, edit))
                .collect(),
        )]))
    }
}
