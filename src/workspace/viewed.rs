//! Marking a pull request's files as viewed. A viewed file collapses to its
//! header; clicking the header opens it again. Read state comes from GraphQL
//! (`gh::viewed_files`) and is written through `Workspace::mutate`.
use super::*;
use std::collections::HashMap;

/// How one file of a pull request diff is shown.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FileView {
    Unviewed,
    /// Marked viewed, but opened again by the reader.
    Open,
    /// Marked viewed: only the header shows.
    Collapsed,
}
impl FileView {
    pub fn of(viewed: bool, reopened: bool) -> Self {
        match (viewed, reopened) {
            (false, _) => Self::Unviewed,
            (true, true) => Self::Open,
            (true, false) => Self::Collapsed,
        }
    }
}

/// Every file's state in a pull request diff, by file index, with its number.
pub struct ViewedInfo {
    pub pr: u64,
    pub files: Vec<FileView>,
}

/// Each path's state, given what GitHub reports and which viewed files were reopened.
pub fn file_views(paths: &[&str], viewed: &HashMap<String, bool>, reopened: impl Fn(&str) -> bool) -> Vec<FileView> {
    paths.iter().map(|p| FileView::of(viewed.get(*p).copied().unwrap_or(false), reopened(p))).collect()
}

impl Workspace {
    /// The pull request diff as shown, with viewed files collapsed, and each
    /// file's state. Without viewed state (still loading, or it failed) the diff
    /// is shown whole.
    pub(super) fn shown_pr_diff(&self, t: &Tab, n: u64, full: &Arc<DiffView>) -> (Arc<DiffView>, Option<ViewedInfo>) {
        let Some(viewed) = self.data(&t.repo).and_then(|d| d.viewed.get(&n)).and_then(|l| l.data.as_ref()) else {
            return (full.clone(), None);
        };
        let paths: Vec<&str> = full.files.iter().map(|f| f.path.as_str()).collect();
        let files = file_views(&paths, &viewed.files, |p| t.viewed_open.contains(&(n, p.to_string())));
        let diff = if files.contains(&FileView::Collapsed) {
            Arc::new(full.collapsed(|f| files[f] == FileView::Collapsed))
        } else {
            full.clone()
        };
        (diff, Some(ViewedInfo { pr: n, files }))
    }

    /// Mark a file viewed, or not viewed if it already is.
    pub(super) fn toggle_viewed(&mut self, n: u64, path: String, cx: &mut Context<Self>) {
        let Some(t) = self.current() else { return };
        let Some(repo) = self.repo(&t.repo).cloned() else { return };
        let (key, tab) = (repo.key(), t.id);
        let Some(viewed) = self.data(&key).and_then(|d| d.viewed.get(&n)).and_then(|l| l.data.as_ref()) else {
            return;
        };
        let target = !viewed.files.get(&path).copied().unwrap_or(false);
        let id = viewed.pr_id.clone();
        let label = if target { format!("Marking {path} as viewed") } else { format!("Marking {path} as not viewed") };
        let (job_path, done_path) = (path.clone(), path);
        // Ticking a file off is frequent and visible in place; only failures speak.
        self.mutate(
            label,
            Report::Quiet,
            move || gh::set_file_viewed(&id, &job_path, target),
            move |s, cx| {
                // GitHub accepted it; mirror that rather than fetch everything again.
                if let Some(v) = s.data.get_mut(&key).and_then(|d| d.viewed.get_mut(&n)).and_then(|l| l.data.as_mut()) {
                    v.files.insert(done_path.clone(), target);
                }
                if let Some(t) = s.tabs.iter_mut().find(|t| t.id == tab) {
                    t.viewed_open.remove(&(n, done_path));
                    t.rows_changed();
                }
                cx.notify();
            },
            cx,
        );
    }

    /// Open or collapse a viewed file by clicking its header.
    pub(super) fn toggle_open(&mut self, n: u64, path: String, cx: &mut Context<Self>) {
        if let Some(t) = self.current_mut() {
            let key = (n, path);
            if !t.viewed_open.remove(&key) {
                t.viewed_open.insert(key);
            }
            t.rows_changed();
        }
        cx.notify();
    }
}

impl Tab {
    /// The diff's rows moved (a file collapsed or opened), so row numbers kept
    /// for the comment box and keyboard stepping no longer point at the same lines.
    fn rows_changed(&mut self) {
        self.selection = None;
        self.diff_nav = None;
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: gpui's own `test` attribute would shadow the built-in one.
    use super::{FileView, file_views};
    use crate::{gh, workspace::DiffView};
    use std::{collections::HashMap, sync::Arc};

    #[test]
    fn viewed_files_collapse_unless_reopened() {
        let viewed: HashMap<String, bool> = [("a".to_string(), true), ("b".to_string(), false), ("c".to_string(), true)].into();
        let views = file_views(&["a", "b", "c", "d"], &viewed, |p| p == "c");
        assert_eq!(views, vec![FileView::Collapsed, FileView::Unviewed, FileView::Open, FileView::Unviewed]);
    }
    #[test]
    fn collapsing_keeps_headers_and_drops_lines() {
        let text = "diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -1 +1 @@\n-x\n+y\ndiff --git a/b.txt b/b.txt\n--- a/b.txt\n+++ b/b.txt\n@@ -1 +1 @@\n-m\n+n\n";
        let d = DiffView::new(gh::parse_diff(text));
        // Rows: 0 header a, 1-3 its lines, 4 header b, 5-7 its lines.
        assert_eq!(d.rows.len(), 8);
        let c = d.collapsed(|f| f == 0);
        assert_eq!(c.rows, vec![(0, None), (1, None), (1, Some(0)), (1, Some(1)), (1, Some(2))]);
        assert_eq!(c.header_row(1), Some(1));
        assert!(Arc::ptr_eq(&c.files, &d.files));
    }
}
