use std::path::{Path, PathBuf};

use async_lsp::{ErrorCode, ResponseError};
use lsp_types::{InitializeParams, Position, Range, ResourceOperationKind};

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ClientWorkspaceEditCapabilities {
    pub(crate) document_changes: bool,
    pub(crate) rename_resource_operation: bool,
}

pub(crate) fn is_valid_anchor_id(id: &str) -> bool {
    !id.is_empty() && !id.contains('#') && !id.chars().any(char::is_whitespace)
}

pub(crate) fn is_valid_link_path_rename(path: &str) -> bool {
    !path.is_empty()
        && !path.contains('#')
        && !path.contains("://")
        && !path.starts_with("mailto:")
        && Path::new(path).is_relative()
}

pub(crate) fn implicit_heading_rename_error() -> ResponseError {
    ResponseError::new(
        ErrorCode::INVALID_REQUEST,
        "Renaming implicit heading anchors is not supported yet; add an explicit {#id} anchor or rename the heading text.",
    )
}

pub(crate) fn document_changes_capability_error() -> ResponseError {
    ResponseError::new(
        ErrorCode::INVALID_REQUEST,
        "Renaming link paths requires client support for workspace.workspaceEdit.documentChanges.",
    )
}

pub(crate) fn rename_resource_operation_capability_error() -> ResponseError {
    ResponseError::new(
        ErrorCode::INVALID_REQUEST,
        "Renaming link paths requires client support for the workspace.workspaceEdit.resourceOperations rename operation.",
    )
}

pub(crate) fn invalid_rename_path_error() -> ResponseError {
    ResponseError::new(
        ErrorCode::INVALID_PARAMS,
        "Rename path must be a relative Djot file path without a fragment.",
    )
}

pub(crate) fn non_djot_path_rename_error() -> ResponseError {
    ResponseError::new(
        ErrorCode::INVALID_REQUEST,
        "Only Djot file links can be renamed.",
    )
}

pub(crate) fn unindexed_path_rename_error() -> ResponseError {
    ResponseError::new(
        ErrorCode::INVALID_REQUEST,
        "Cannot rename a link path whose target is not indexed in the workspace.",
    )
}

pub(crate) fn rename_target_exists_error() -> ResponseError {
    ResponseError::new(
        ErrorCode::INVALID_REQUEST,
        "Cannot rename link path because the target path already exists.",
    )
}

pub(crate) fn rename_target_outside_workspace_error() -> ResponseError {
    ResponseError::new(
        ErrorCode::INVALID_REQUEST,
        "Cannot rename link path outside the workspace.",
    )
}

pub(crate) fn byte_range_to_lsp(text: &str, range: &std::ops::Range<usize>) -> Range {
    Range {
        start: offset_to_position(text, range.start),
        end: offset_to_position(text, range.end),
    }
}

pub(crate) fn position_to_offset(text: &str, pos: Position) -> usize {
    let mut line = 0u32;
    let mut character = 0u32;
    for (i, c) in text.char_indices() {
        if line == pos.line && character == pos.character {
            return i;
        }
        if c == '\n' {
            if line == pos.line {
                return i;
            }
            line += 1;
            character = 0;
        } else {
            character += c.len_utf16() as u32;
        }
    }
    text.len()
}

pub(crate) fn offset_to_position(text: &str, offset: usize) -> Position {
    let mut line = 0u32;
    let mut character = 0u32;
    for (i, c) in text.char_indices() {
        if i >= offset {
            break;
        }
        if c == '\n' {
            line += 1;
            character = 0;
        } else {
            character += c.len_utf16() as u32;
        }
    }
    Position { line, character }
}

pub(crate) fn workspace_roots(params: &InitializeParams) -> Vec<PathBuf> {
    if let Some(folders) = &params.workspace_folders {
        folders
            .iter()
            .filter_map(|folder| folder.uri.to_file_path().ok())
            .collect()
    } else {
        #[allow(deprecated)]
        params
            .root_uri
            .as_ref()
            .and_then(|uri| uri.to_file_path().ok())
            .into_iter()
            .collect()
    }
}

pub(crate) fn client_workspace_edit_capabilities(
    params: &InitializeParams,
) -> ClientWorkspaceEditCapabilities {
    let Some(workspace_edit) = params
        .capabilities
        .workspace
        .as_ref()
        .and_then(|workspace| workspace.workspace_edit.as_ref())
    else {
        return ClientWorkspaceEditCapabilities::default();
    };

    ClientWorkspaceEditCapabilities {
        document_changes: workspace_edit.document_changes == Some(true),
        rename_resource_operation: workspace_edit
            .resource_operations
            .as_ref()
            .is_some_and(|operations| operations.contains(&ResourceOperationKind::Rename)),
    }
}
