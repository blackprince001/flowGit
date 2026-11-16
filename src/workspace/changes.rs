//! Changes: a local clone's working tree. Stage and unstage files, read each
//! file's diff, commit, then push or pull. Only local clones have this view.
use super::ui::*;
use super::*;
use crate::{
    gh::{Area, Change, ChangeKind, ChangeRef},
    icons::Glyph,
    menu::{Entry, Item},
};
use gpui::prelude::FluentBuilder;
use gpui_component::{
    Disableable, Icon, IconName, Sizable,
    button::{Button, ButtonVariants},
    input::Input,
    resizable::*,
};
use std::path::{Path, PathBuf};

const FILE_ROW: f32 = 26.;
/// The staged list grows with its files up to this many rows, then scrolls.
const STAGED_ROWS: usize = 6;
/// How often the working tree is re-read while this view is open.
const POLL: Duration = Duration::from_secs(3);

/// What else a git write changes besides the working tree.
#[derive(Clone, Copy)]
enum Aftermath {
    /// Only the index moved.
    Index,
    /// Commits or branch tips moved, so the history graph is stale.
    History,
    /// A commit was made in this tab: its message box is spent.
    Committed(u64),
}

fn split_path(path: &str) -> (String, String) {
    match path.rsplit_once('/') {
        Some((dir, name)) => (name.to_string(), dir.to_string()),
        None => (path.to_string(), String::new()),
    }
}
fn files_label(n: usize) -> String {
    if n == 1 { "1 file".into() } else { format!("{n} files") }
}
/// The change at position `i` when `areas` are listed one after another.
fn nth<'a>(status: &'a gh::Status, areas: &[Area], mut i: usize) -> Option<(Area, &'a Change)> {
    for area in areas {
        let list = status.list(*area);
        if i < list.len() {
            return Some((*area, &list[i]));
        }
        i -= list.len();
    }
    None
}
fn change_glyph(colors: Palette, kind: ChangeKind) -> (Icon, u32) {
    match kind {
        ChangeKind::Added | ChangeKind::Untracked => (IconName::Plus.into(), colors.green),
        ChangeKind::Deleted => (IconName::Minus.into(), colors.red),
        ChangeKind::Renamed => (IconName::ArrowRight.into(), colors.blue),
        ChangeKind::Modified => (Glyph::FileDiff.into(), colors.amber),
        ChangeKind::Conflicted => (IconName::TriangleAlert.into(), colors.red),
    }
}

impl Workspace {
    // ── Data ────────────────────────────────────────────────────────────────

    /// Read the working tree and the selected file's diff. Staging moves a file
    /// between lists, so the selection is first pointed at where its file is now.
    pub(super) fn ensure_changes(&mut self, key: &str, path: Option<PathBuf>, force: bool, cx: &mut Context<Self>) {
        let Some(path) = path else { return };
        let picked = self.current().filter(|t| t.repo == key).and_then(|t| t.change.clone());
        if let Some(pick) = &picked
            && let Some(status) = self.data(key).and_then(|d| d.status.data.as_ref())
        {
            let now = status.locate(pick);
            if now.as_ref() != Some(pick)
                && let Some(t) = self.current_mut()
            {
                t.change = now;
            }
        }
        let p = path.clone();
        self.fetch(key, |d| &mut d.status, force, move || Ok(Arc::new(gh::status(&p)?)), cx);
        if let Some(pick) = self.current().filter(|t| t.repo == key).and_then(|t| t.change.clone()) {
            let k = pick.clone();
            self.fetch(
                key,
                move |d| d.change_diff.entry(k.clone()).or_default(),
                force,
                move || Ok(Arc::new(DiffView::new(gh::file_diff(&path, &pick.path, pick.area)?))),
                cx,
            );
        }
    }
    /// From `tick`: re-read the working tree every few seconds while it is on screen.
    pub(super) fn poll_changes(&mut self, cx: &mut Context<Self>) {
        let Some(t) = self.current().filter(|t| t.view == View::Changes) else { return };
        if self.last_changes.is_some_and(|at| at.elapsed() < POLL) || self.busy.is_some() {
            return;
        }
        let key = t.repo.clone();
        let Some(path) = self.repo(&key).and_then(|r| r.path.clone()) else { return };
        self.last_changes = Some(Instant::now());
        self.ensure_changes(&key, Some(path), true, cx);
    }
    /// Empty the commit message of the tab that just committed (needs a window).
    pub(super) fn clear_committed_message(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.pending_commit_clear.take()
            && let Some(t) = self.tabs.iter().find(|t| t.id == id)
        {
            t.commit_message.update(cx, |s, cx| s.set_value("", window, cx));
        }
    }
    fn status_of(&self, key: &str) -> Option<Arc<gh::Status>> {
        self.data(key)?.status.data.clone()
    }

    // ── Actions ─────────────────────────────────────────────────────────────

    /// Run one git write on the current tab's clone, then re-read its status.
    fn git_write(&mut self, label: String, after: Aftermath, job: impl FnOnce(&Path) -> anyhow::Result<String> + Send + 'static, cx: &mut Context<Self>) {
        let Some(t) = self.current() else { return };
        let Some(repo) = self.repo(&t.repo).cloned() else { return };
        let Some(path) = repo.path.clone() else { return };
        let key = repo.key();
        let at = path.clone();
        self.mutate(
            label,
            move || job(&at),
            move |s, cx| {
                if !matches!(after, Aftermath::Index)
                    && let Some(d) = s.data.get_mut(&key)
                {
                    d.commits.clear();
                }
                if let Aftermath::Committed(tab) = after {
                    s.pending_commit_clear = Some(tab);
                }
                s.last_changes = Some(Instant::now());
                s.ensure_changes(&key, Some(path), true, cx);
            },
            cx,
        );
    }
    fn stage_files(&mut self, files: Vec<String>, cx: &mut Context<Self>) {
        if files.is_empty() {
            return;
        }
        let label = format!("Staging {}", files_label(files.len()));
        self.git_write(label, Aftermath::Index, move |p| gh::stage(p, &files), cx);
    }
    fn unstage_files(&mut self, files: Vec<String>, cx: &mut Context<Self>) {
        if files.is_empty() {
            return;
        }
        let label = format!("Unstaging {}", files_label(files.len()));
        self.git_write(label, Aftermath::Index, move |p| gh::unstage(p, &files), cx);
    }
    fn stage_all(&mut self, cx: &mut Context<Self>) {
        let Some(status) = self.current().and_then(|t| self.status_of(&t.repo)) else { return };
        let files = status.unstaged.iter().chain(&status.untracked).flat_map(Change::paths).collect();
        self.stage_files(files, cx);
    }
    fn unstage_all(&mut self, cx: &mut Context<Self>) {
        let Some(status) = self.current().and_then(|t| self.status_of(&t.repo)) else { return };
        self.unstage_files(status.staged.iter().flat_map(Change::paths).collect(), cx);
    }
    /// Commit the staged files with the message box's text (⌘ Return and the button).
    pub(super) fn commit_staged(&mut self, cx: &mut Context<Self>) {
        let Some(t) = self.current() else { return };
        let message = t.commit_message.read(cx).value().trim().to_string();
        let tab = t.id;
        let staged = self.status_of(&t.repo).is_some_and(|s| !s.staged.is_empty());
        if message.is_empty() {
            self.notice = Notice::warn("Write a commit message first.");
        } else if !staged {
            self.notice = Notice::warn("Stage some files to commit first.");
        } else {
            self.git_write("Committing".into(), Aftermath::Committed(tab), move |p| gh::commit(p, &message), cx);
        }
        cx.notify();
    }
    fn push_branch(&mut self, cx: &mut Context<Self>) {
        let publish = self.current().and_then(|t| self.status_of(&t.repo)).is_some_and(|s| s.upstream.is_none());
        let label = if publish { "Publishing branch" } else { "Pushing" };
        self.git_write(label.into(), Aftermath::History, gh::push_branch, cx);
    }
    fn pull_branch(&mut self, cx: &mut Context<Self>) {
        self.git_write("Pulling".into(), Aftermath::History, gh::pull_branch, cx);
    }
    /// Show a file's diff, freshly read: a diff kept from earlier may be out of date.
    fn pick_change(&mut self, pick: ChangeRef, cx: &mut Context<Self>) {
        let Some(t) = self.current_mut() else { return };
        t.diff_scroll.scroll_to_item(0, ScrollStrategy::Top);
        let key = t.repo.clone();
        t.change = Some(pick.clone());
        if let Some(d) = self.data.get_mut(&key) {
            d.change_diff.remove(&pick);
        }
        cx.notify();
    }

    /// Palette commands for the working tree, offered while the Changes view is open.
    pub(super) fn changes_entries(&self, weak: &WeakEntity<Self>) -> Vec<Entry> {
        let Some(t) = self.current().filter(|t| t.view == View::Changes) else { return vec![] };
        let Some(status) = self.status_of(&t.repo) else { return vec![] };
        let idle = self.busy.is_none();
        let item = |title: &'static str, icon: Icon, enabled: bool, run: fn(&mut Workspace, &mut Context<Workspace>)| {
            let w = weak.clone();
            Entry::Item(Item::new(title).icon(icon).disabled(!enabled || !idle).on_click(move |_, cx| {
                let _ = w.update(cx, |s, cx| run(s, cx));
            }))
        };
        vec![
            Entry::Label("Changes".into()),
            item("Stage all", IconName::Plus.into(), !status.unstaged.is_empty() || !status.untracked.is_empty(), |s, cx| s.stage_all(cx)),
            item("Unstage all", IconName::Minus.into(), !status.staged.is_empty(), |s, cx| s.unstage_all(cx)),
            item("Commit", Glyph::Commit.into(), !status.staged.is_empty(), |s, cx| s.commit_staged(cx)),
            item(if status.upstream.is_none() { "Publish branch" } else { "Push" }, IconName::ArrowUp.into(), status.can_push(), |s, cx| s.push_branch(cx)),
            item("Pull", IconName::ArrowDown.into(), status.can_pull(), |s, cx| s.pull_branch(cx)),
        ]
    }

    // ── View ────────────────────────────────────────────────────────────────

    pub(super) fn changes_view(&self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let colors = palette(cx);
        let Some(t) = self.current() else { return div().into_any_element() };
        if !self.is_local(&t.repo) {
            return empty(colors, Glyph::FileDiff, "No local clone", "Changes are read from a folder on disk. Add this repository with Open local repository.").into_any_element();
        }
        let l = self.data(&t.repo).map(|d| &d.status);
        if let Err(p) = state(colors, l, "Reading changes…") {
            return p.into_any_element();
        }
        let Some(status) = self.status_of(&t.repo) else { return div().into_any_element() };

        let right = match &t.change {
            None => empty(colors, Glyph::FileDiff, "No file selected", "Pick a file to see what changed.").into_any_element(),
            Some(pick) => {
                let l = self.data(&t.repo).and_then(|d| d.change_diff.get(pick));
                match state(colors, l, "Loading changes…") {
                    Err(p) => p.into_any_element(),
                    Ok(()) => {
                        let diff = l.and_then(|l| l.data.clone()).unwrap();
                        self.diff_list(("changes-diff", t.id), diff, t.diff_scroll.clone(), None, cx)
                    }
                }
            }
        };
        let body = h_resizable(SharedString::from(format!("changes-files-{}", t.id)))
            .child(
                resizable_panel()
                    .size(px(320.))
                    .size_range(px(240.)..px(560.))
                    .child(self.change_lists(t, status.clone(), cx)),
            )
            .child(resizable_panel().child(right));
        col()
            .size_full()
            .bg(rgb(colors.background))
            .child(self.changes_header(&status, cx))
            .child(div().flex_1().min_h_0().child(body))
            .into_any_element()
    }

    /// Branch, upstream and how far apart they are, with Pull and Push.
    fn changes_header(&self, status: &gh::Status, cx: &mut Context<Self>) -> Div {
        let colors = palette(cx);
        let mono = settings::mono(cx);
        let busy = self.busy.is_some();
        let branch = status.branch.clone().unwrap_or_else(|| "detached HEAD".into());
        let sync = |glyph: IconName, n: u32, color: u32| {
            row()
                .gap(px(2.))
                .text_size(px(12.))
                .text_color(rgb(color))
                .child(icon(glyph).size(px(12.)))
                .child(n.to_string())
        };
        let (push_label, push_tip) = if status.upstream.is_none() {
            ("Publish branch", "Push this branch to origin and track it")
        } else {
            ("Push", "Push commits to the upstream branch")
        };
        row()
            .h(px(BAR + 6.))
            .flex_shrink_0()
            .px_2()
            .gap_2()
            .border_b_1()
            .border_color(rgb(colors.row_border))
            .child(icon(Glyph::Branch).text_color(rgb(colors.muted)))
            .child(chip(colors, mono.clone(), branch))
            .child(match &status.upstream {
                Some(up) => row()
                    .gap_2()
                    .child(icon(IconName::ArrowRight).size(px(12.)).text_color(rgb(colors.quiet)))
                    .child(tinted_chip(colors.muted, colors.hover, mono, up.clone()))
                    .into_any_element(),
                None => label(colors, "No upstream branch").into_any_element(),
            })
            .when(status.ahead > 0, |s| s.child(sync(IconName::ArrowUp, status.ahead, colors.green)))
            .when(status.behind > 0, |s| s.child(sync(IconName::ArrowDown, status.behind, colors.amber)))
            .child(div().flex_1())
            .child(
                Button::new("changes-pull")
                    .label("Pull")
                    .icon(IconName::ArrowDown)
                    .outline()
                    .small()
                    .tooltip("Fast-forward from the upstream branch")
                    .loading(self.busy_with("Pulling"))
                    .disabled(busy || !status.can_pull())
                    .on_click(cx.listener(|s, _, _, cx| s.pull_branch(cx))),
            )
            .child(
                Button::new("changes-push")
                    .label(push_label)
                    .icon(IconName::ArrowUp)
                    .outline()
                    .small()
                    .tooltip(push_tip)
                    .loading(self.busy_with("Pushing") || self.busy_with("Publishing"))
                    .disabled(busy || !status.can_push())
                    .on_click(cx.listener(|s, _, _, cx| s.push_branch(cx))),
            )
    }

    /// The left column: staged files, the commit box, then the rest.
    fn change_lists(&self, t: &Tab, status: Arc<gh::Status>, cx: &mut Context<Self>) -> AnyElement {
        let colors = palette(cx);
        let busy = self.busy.is_some();
        let message = t.commit_message.read(cx).value().trim().to_string();
        let can_commit = !status.staged.is_empty() && !message.is_empty() && !busy;
        let loose = status.unstaged.len() + status.untracked.len();

        let staged_body = if status.staged.is_empty() {
            row().h(px(FILE_ROW)).px_3().child(label(colors, "Nothing staged")).into_any_element()
        } else {
            div()
                .h(px(status.staged.len().min(STAGED_ROWS) as f32 * FILE_ROW))
                .px_1()
                .child(self.change_list(("staged-files", t.id), status.clone(), &[Area::Staged], t.change.clone(), cx))
                .into_any_element()
        };
        let loose_body = if loose == 0 {
            empty(colors, Glyph::Commit, "No changes", "The working tree matches the last commit.").into_any_element()
        } else {
            div()
                .size_full()
                .px_1()
                .child(self.change_list(("loose-files", t.id), status.clone(), &[Area::Unstaged, Area::Untracked], t.change.clone(), cx))
                .into_any_element()
        };
        let commit = col()
            .flex_shrink_0()
            .p_2()
            .gap_2()
            .border_y_1()
            .border_color(rgb(colors.row_border))
            .child(Input::new(&t.commit_message))
            .child(
                row()
                    .gap_2()
                    .child(label(colors, "⌘ Return to commit"))
                    .child(div().flex_1())
                    .child(
                        Button::new("commit")
                            .label("Commit")
                            .icon(Glyph::Commit)
                            .primary()
                            .small()
                            .loading(self.busy_with("Committing"))
                            .disabled(!can_commit)
                            .on_click(cx.listener(|s, _, _, cx| s.commit_staged(cx))),
                    ),
            );
        col()
            .size_full()
            .child(
                section(colors, "Staged")
                    .px_3()
                    .flex_shrink_0()
                    .child(count_badge(colors, status.staged.len()))
                    .child(div().flex_1())
                    .child(
                        button("unstage-all", "Unstage all")
                            .xsmall()
                            .disabled(busy || status.staged.is_empty())
                            .on_click(cx.listener(|s, _, _, cx| s.unstage_all(cx))),
                    ),
            )
            .child(staged_body)
            .child(commit)
            .child(
                section(colors, "Changes")
                    .px_3()
                    .flex_shrink_0()
                    .child(count_badge(colors, loose))
                    .child(div().flex_1())
                    .child(
                        button("stage-all", "Stage all")
                            .xsmall()
                            .disabled(busy || loose == 0)
                            .on_click(cx.listener(|s, _, _, cx| s.stage_all(cx))),
                    ),
            )
            .child(div().flex_1().min_h_0().child(loose_body))
            .into_any_element()
    }

    /// Files from `areas`, listed one after another, as a virtualized list.
    fn change_list(&self, id: (&'static str, u64), status: Arc<gh::Status>, areas: &'static [Area], picked: Option<ChangeRef>, cx: &mut Context<Self>) -> AnyElement {
        let total = areas.iter().map(|a| status.list(*a).len()).sum();
        uniform_list(
            id,
            total,
            cx.processor(move |s, range: std::ops::Range<usize>, _, cx| {
                range
                    .filter_map(|i| {
                        let (area, c) = nth(&status, areas, i)?;
                        let on = picked.as_ref().is_some_and(|p| p.area == area && p.path == c.path);
                        Some(s.change_row(id.0, i, area, c, on, cx))
                    })
                    .collect()
            }),
        )
        .size_full()
        .into_any_element()
    }

    /// One file: its change, name and folder, and a button to stage or unstage it.
    fn change_row(&self, list: &'static str, i: usize, area: Area, c: &Change, on: bool, cx: &mut Context<Self>) -> Stateful<Div> {
        let colors = palette(cx);
        let (glyph, color) = change_glyph(colors, c.kind);
        let (name, dir) = split_path(&c.path);
        let staged = area == Area::Staged;
        let pick = ChangeRef { path: c.path.clone(), area };
        let files = c.paths();
        let tip = match &c.old_path {
            Some(old) => format!("{old} → {}", c.path),
            None => c.path.clone(),
        };
        row()
            .id((list, i))
            .w_full()
            .h(px(FILE_ROW))
            .px_2()
            .gap_2()
            .cursor_pointer()
            .rounded(px(4.))
            .when(on, |s| s.bg(rgb(colors.selected)))
            .when(!on, |s| s.hover(|s| s.bg(rgb(colors.hover))))
            .child(icon(glyph).size(px(12.)).text_color(rgb(color)))
            .child(
                row()
                    .id(("change-name", i))
                    .flex_1()
                    .min_w_0()
                    .gap(px(5.))
                    .child(div().flex_shrink_0().max_w(relative(0.8)).truncate().child(name))
                    .child(div().min_w_0().truncate().text_size(px(11.)).text_color(rgb(colors.quiet)).child(dir))
                    .tooltip(move |w, cx| gpui_component::tooltip::Tooltip::new(tip.clone()).build(w, cx)),
            )
            .child(
                icon_button(
                    (if staged { "unstage-file" } else { "stage-file" }, i),
                    if staged { IconName::Minus } else { IconName::Plus },
                    if staged { "Unstage" } else { "Stage" },
                )
                .xsmall()
                .disabled(self.busy.is_some())
                .on_click(cx.listener(move |s, _, _, cx| {
                    cx.stop_propagation();
                    if staged {
                        s.unstage_files(files.clone(), cx);
                    } else {
                        s.stage_files(files.clone(), cx);
                    }
                })),
            )
            .on_click(cx.listener(move |s, _, _, cx| s.pick_change(pick.clone(), cx)))
    }
}
