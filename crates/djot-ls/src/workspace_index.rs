use std::path::{Path, PathBuf};

use lsp_types::{
    NumberOrString, ProgressParams, ProgressParamsValue, WorkDoneProgress, WorkDoneProgressBegin,
    WorkDoneProgressEnd, WorkDoneProgressReport,
};

use crate::path_utils::is_djot_file;
use crate::ServerState;

impl ServerState {
    pub(crate) fn index_workspace_root(&mut self, root: &Path) -> usize {
        index_djot_files(root, &mut |path, text| {
            self.workspace.insert(path, text);
        })
    }

    pub(crate) fn is_in_workspace(&self, path: &Path) -> bool {
        self.workspace_roots
            .iter()
            .any(|root| path.starts_with(root))
    }

    pub(crate) fn index_workspace_roots_with_progress(&mut self) {
        if self.workspace_roots.is_empty() {
            return;
        }

        self.notify_index_progress(WorkDoneProgress::Begin(WorkDoneProgressBegin {
            title: "Indexing Djot workspace".to_string(),
            cancellable: Some(false),
            message: Some("Scanning .dj/.djot files".to_string()),
            percentage: None,
        }));

        let mut indexed = 0usize;
        for root in self.workspace_roots.clone() {
            indexed += self.index_workspace_root(&root);
        }

        self.notify_index_progress(WorkDoneProgress::Report(WorkDoneProgressReport {
            cancellable: Some(false),
            message: Some(format!("Indexed {indexed} files")),
            percentage: None,
        }));
        self.notify_index_progress(WorkDoneProgress::End(WorkDoneProgressEnd {
            message: Some(format!("Indexed {indexed} Djot files")),
        }));
    }

    fn notify_index_progress(&self, progress: WorkDoneProgress) {
        let _ = self
            .client
            .notify::<lsp_types::notification::Progress>(ProgressParams {
                token: NumberOrString::String("djot-ls-index".to_string()),
                value: ProgressParamsValue::WorkDone(progress),
            });
    }

    pub(crate) fn display_path(&self, path: &Path) -> String {
        self.workspace_roots
            .iter()
            .find_map(|root| path.strip_prefix(root).ok())
            .unwrap_or(path)
            .display()
            .to_string()
    }
}

fn index_djot_files(root: &Path, insert: &mut impl FnMut(PathBuf, String)) -> usize {
    let Ok(entries) = std::fs::read_dir(root) else {
        return 0;
    };

    let mut indexed = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };

        if file_type.is_dir() {
            indexed += index_djot_files(&path, insert);
        } else if file_type.is_file() && is_djot_file(&path) {
            if let Ok(text) = std::fs::read_to_string(&path) {
                insert(path, text);
                indexed += 1;
            }
        }
    }
    indexed
}
