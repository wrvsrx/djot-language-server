use std::ops::Range;
use std::path::{Path, PathBuf};

use crate::analysis::Anchor;
use crate::edits::{DocumentTextEdit, FileRenameEdit, TextEdit, WorkspaceEdit};
use crate::paths::{is_djot_file_path, normalize, relative_link_path};
use crate::references::resolve_target;
use crate::workspace::Workspace;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenameTarget {
    /// The document containing the anchor declaration.
    pub path: PathBuf,
    pub id: String,
    /// The source range under the cursor that should be selected before rename.
    pub range: Range<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenameTargetError {
    NotRenameable,
    ImplicitHeadingAnchor,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathRenameTarget {
    pub old_path: PathBuf,
    pub range: Range<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathRenameError {
    NotRenameable,
    NonDjotPath,
    TargetNotIndexed,
}

impl Workspace {
    pub fn rename_target_at(
        &self,
        path: &Path,
        offset: usize,
    ) -> Result<RenameTarget, RenameTargetError> {
        let path = normalize(path);
        if let Some((id, anchor)) = self.anchor_rename_at(&path, offset) {
            if !anchor.explicit {
                return Err(RenameTargetError::ImplicitHeadingAnchor);
            }
            return Ok(RenameTarget {
                path,
                id: id.to_string(),
                range: anchor.rename_range.clone(),
            });
        }

        let reference = self
            .reference_at(&path, offset)
            .ok_or(RenameTargetError::NotRenameable)?;
        let target_id_range = reference
            .target_id_range
            .clone()
            .ok_or(RenameTargetError::NotRenameable)?;
        if !contains_inclusive(&target_id_range, offset) {
            return Err(RenameTargetError::NotRenameable);
        }
        let target =
            resolve_target(&path, &reference.target).ok_or(RenameTargetError::NotRenameable)?;
        let id = target.id.ok_or(RenameTargetError::NotRenameable)?;
        let anchor = self
            .anchor(&target.path, &id)
            .ok_or(RenameTargetError::NotRenameable)?;
        if !anchor.explicit {
            return Err(RenameTargetError::ImplicitHeadingAnchor);
        }

        Ok(RenameTarget {
            path: target.path,
            id,
            range: target_id_range,
        })
    }

    fn anchor_rename_at(&self, path: &Path, offset: usize) -> Option<(&str, &Anchor)> {
        self.get(path)?
            .analysis
            .index
            .anchors
            .iter()
            .find(|(_, anchor)| contains_inclusive(&anchor.rename_range, offset))
            .map(|(id, anchor)| (id.as_str(), anchor))
    }

    /// Text edits for renaming an explicit anchor and all indexed references to
    /// it. Scans all loaded documents, so completeness requires the caller to
    /// have indexed the workspace first.
    pub fn anchor_rename_edits(
        &self,
        path: &Path,
        id: &str,
        replacement: &str,
    ) -> Vec<DocumentTextEdit> {
        let target = normalize(path);
        let mut edits = Vec::new();

        if let Some(anchor) = self.anchor(&target, id) {
            if !anchor.explicit {
                return Vec::new();
            }
            edits.push(DocumentTextEdit {
                path: target.clone(),
                edit: TextEdit {
                    range: anchor.rename_range.clone(),
                    new_text: replacement.to_string(),
                },
            });
        } else {
            return Vec::new();
        }

        for (src, entry) in &self.docs {
            for reference in &entry.analysis.index.references {
                let Some(range) = &reference.target_id_range else {
                    continue;
                };
                let Some(resolved) = resolve_target(src, &reference.target) else {
                    continue;
                };
                if resolved.path == target && resolved.id.as_deref() == Some(id) {
                    edits.push(DocumentTextEdit {
                        path: src.clone(),
                        edit: TextEdit {
                            range: range.clone(),
                            new_text: replacement.to_string(),
                        },
                    });
                }
            }
        }

        edits
    }

    /// Resolve a file path link under `offset` to the indexed document it
    /// targets. Only Djot file targets can be renamed this way.
    pub fn path_rename_target_at(
        &self,
        path: &Path,
        offset: usize,
    ) -> Result<PathRenameTarget, PathRenameError> {
        let path = normalize(path);
        let reference = self
            .reference_at(&path, offset)
            .ok_or(PathRenameError::NotRenameable)?;
        let range = reference
            .target_path_range
            .clone()
            .ok_or(PathRenameError::NotRenameable)?;
        if !contains_inclusive(&range, offset) {
            return Err(PathRenameError::NotRenameable);
        }

        let target =
            resolve_target(&path, &reference.target).ok_or(PathRenameError::NotRenameable)?;
        if !is_djot_file_path(&target.path) {
            return Err(PathRenameError::NonDjotPath);
        }
        if !self.contains(&target.path) {
            return Err(PathRenameError::TargetNotIndexed);
        }

        Ok(PathRenameTarget {
            old_path: target.path,
            range,
        })
    }

    /// Text edits for updating all indexed links when moving a document from
    /// `old_path` to `new_path`.
    pub fn path_rename_text_edits(
        &self,
        old_path: &Path,
        new_path: &Path,
    ) -> Vec<DocumentTextEdit> {
        let old_path = normalize(old_path);
        let new_path = normalize(new_path);
        let mut edits = Vec::new();

        for (src, entry) in &self.docs {
            for reference in &entry.analysis.index.references {
                let Some(range) = &reference.target_path_range else {
                    continue;
                };
                let Some(resolved) = resolve_target(src, &reference.target) else {
                    continue;
                };
                if resolved.path == old_path {
                    edits.push(DocumentTextEdit {
                        path: src.clone(),
                        edit: TextEdit {
                            range: range.clone(),
                            new_text: relative_link_path(src, &new_path),
                        },
                    });
                }
            }
        }

        edits
    }

    /// A protocol-agnostic workspace edit plan for moving a document and
    /// updating indexed links to point at the new path. This performs no file
    /// system checks; callers that own I/O must validate conflicts separately.
    pub fn path_rename_edit_plan(&self, old_path: &Path, new_path: &Path) -> Vec<WorkspaceEdit> {
        let old_path = normalize(old_path);
        let new_path = normalize(new_path);
        std::iter::once(WorkspaceEdit::RenameFile(FileRenameEdit {
            old_path: old_path.clone(),
            new_path: new_path.clone(),
        }))
        .chain(
            self.path_rename_text_edits(&old_path, &new_path)
                .into_iter()
                .map(WorkspaceEdit::Text),
        )
        .collect()
    }
}

fn contains_inclusive(range: &Range<usize>, offset: usize) -> bool {
    range.start <= offset && offset <= range.end
}
