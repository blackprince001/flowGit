//! Compare two branches and open a pull request from them: base and head
//! pickers, the commits and diff `head` adds, and a title and body to send.
use super::pulls::segmented;
use super::ui::*;
use super::*;
use crate::{
    icons::Glyph,
    menu::{Entry, Item},
    util,
};
use gpui::prelude::FluentBuilder;
use gpui_component::{
    Disableable, IconName, Sizable,
    button::{Button, ButtonVariants},
    input::Input,
    resizable::*,
    switch::Switch,
};
use std::sync::atomic::{AtomicU64, Ordering};

actions!(gitflow, [NewPullRequest]);

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ComparePage {
    #[default]
    Commits,
    Files,
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Side {
    Base,
    Head,
}

/// The "new pull request" flow in one tab. Creating it is a `mutate`, so the
/// busy label says when it is being sent; there is no separate submitting state.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum NewPr {
    #[default]
    Closed,
    Open {
        base: String,
        /// Not chosen yet.
        head: Option<String>,
        draft: bool,
        page: ComparePage,
        /// The title and body were filled from this pair of branches.
        prefilled: bool,
    },
}
impl NewPr {
    pub fn open(base: String, head: Option<String>) -> Self {
        Self::Open {
            base,
            head,
            draft: false,
            page: ComparePage::Commits,
            prefilled: false,
        }
    }
    pub fn is_open(&self) -> bool {
        matches!(self, Self::Open { .. })
    }
    /// The branches being compared, once both are chosen.
    pub fn range(&self) -> Option<(&str, &str)> {
        match self {
            Self::Open { base, head: Some(head), .. } => Some((base, head)),
            _ => None,
        }
    }
    /// Choosing a different branch means the title and body no longer describe it.
    pub fn pick(&mut self, side: Side, name: String) {
        if let Self::Open { base, head, prefilled, .. } = self {
            let changed = match side {
                Side::Base => {
                    let changed = *base != name;
                    *base = name;
                    changed
                }
                Side::Head => {
                    let changed = head.as_deref() != Some(name.as_str());
                    *head = Some(name);
                    changed
                }
            };
            if changed {
                *prefilled = false;
            }
        }
    }
    pub fn set_draft(&mut self, on: bool) {
        if let Self::Open { draft, .. } = self {
            *draft = on;
        }
    }
    pub fn set_page(&mut self, to: ComparePage) {
        if let Self::Open { page, .. } = self {
            *page = to;
        }
    }
    /// The branches whose commits should fill the title and body, if that
    /// hasn't happened yet.
    pub fn needs_prefill(&self) -> Option<(&str, &str)> {
        match self {
            Self::Open { prefilled: false, .. } => self.range(),
            _ => None,
        }
    }
    pub fn mark_prefilled(&mut self) {
        if let Self::Open { prefilled, .. } = self {
            *prefilled = true;
        }
    }
}

impl Workspace {
    /// Open the compare page on the active tab. The base is the default branch;
    /// the head is the branch the tab is already looking at, when it isn't that.
    pub(super) fn new_pull_request(&mut self, _: &NewPullRequest, window: &mut Window, cx: &mut Context<Self>) {
        let Some(t) = self.current() else { return };
        let Some(base) = self.default_branch(&t.repo) else { return };
        let head = t.branch.clone().filter(|b| *b != base);
        let (title, body) = (t.pr_title.clone(), t.pr_body.clone());
        if let Some(t) = self.current_mut() {
            t.view = View::Pulls;
            t.pr = None;
            t.line_target = None;
            t.new_pr = NewPr::open(base, head);
        }
        title.update(cx, |s, cx| s.set_value("", window, cx));
        body.update(cx, |s, cx| s.set_value("", window, cx));
        cx.notify();
    }

    /// Fetch what the compare page shows. Called from `ensure`.
    pub(super) fn ensure_compare(&mut self, key: &str, slug: &str, force: bool, cx: &mut Context<Self>) {
        let Some(t) = self.current() else { return };
        if !t.new_pr.is_open() {
            return;
        }
        let range = t.new_pr.range().map(|(b, h)| (b.to_string(), h.to_string()));
        let s = slug.to_string();
        self.fetch(key, |d| &mut d.branches, force, move || gh::branches(&s), cx);
        let Some((base, head)) = range else { return };
        let (s, b, h) = (slug.to_string(), base.clone(), head.clone());
        let k = (base.clone(), head.clone());
        self.fetch(key, move |d| d.compare.entry(k.clone()).or_default(), force, move || Ok(Arc::new(gh::compare(&s, &b, &h)?)), cx);
        let (s, b, h) = (slug.to_string(), base.clone(), head.clone());
        let k = (base, head);
        self.fetch(
            key,
            move |d| d.compare_diff.entry(k.clone()).or_default(),
            force,
            move || Ok(Arc::new(DiffView::new(gh::compare_diff(&s, &b, &h)?))),
            cx,
        );
    }

    /// Fill the title and body from the commits once they have loaded.
    fn prefill_new_pr(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(t) = self.current() else { return };
        let Some((base, head)) = t.new_pr.needs_prefill().map(|(b, h)| (b.to_string(), h.to_string())) else {
            return;
        };
        let Some(compare) = self.data(&t.repo).and_then(|d| d.compare.get(&(base, head.clone()))).and_then(|l| l.data.clone()) else {
            return;
        };
        let (title, body) = gh::prefill(&head, &compare.commits);
        let (title_input, body_input) = (t.pr_title.clone(), t.pr_body.clone());
        title_input.update(cx, |s, cx| s.set_value(title, window, cx));
        body_input.update(cx, |s, cx| s.set_value(body, window, cx));
        if let Some(t) = self.current_mut() {
            t.new_pr.mark_prefilled();
        }
        cx.notify();
    }

    /// `gh pr create` with the page's branches, title and body, then open the result.
    pub(super) fn create_pull(&mut self, cx: &mut Context<Self>) {
        let Some(t) = self.current() else { return };
        let Some(repo) = self.repo(&t.repo).cloned() else { return };
        let NewPr::Open { base, head: Some(head), draft, .. } = &t.new_pr else { return };
        let (base, head, draft) = (base.clone(), head.clone(), *draft);
        let title = t.pr_title.read(cx).value().trim().to_string();
        let body = t.pr_body.read(cx).value().to_string();
        if title.is_empty() {
            self.notice = Notice::warn("Give the pull request a title.");
            cx.notify();
            return;
        }
        let (key, slug, tab) = (repo.key(), repo.slug(), t.id);
        let number = Arc::new(AtomicU64::new(0));
        let sent = number.clone();
        let label = format!("Creating pull request from {head}");
        self.mutate(
            label,
            Report::Notice,
            move || {
                let n = gh::create_pr(&slug, &base, &head, &title, &body, draft)?;
                sent.store(n, Ordering::SeqCst);
                Ok(format!("Created #{n}"))
            },
            move |s, cx| {
                if let Some(d) = s.data.get_mut(&key) {
                    d.pulls.clear();
                    d.compare.clear();
                    d.compare_diff.clear();
                }
                let n = number.load(Ordering::SeqCst);
                // Only follow it if the tab is still on the compare page.
                if let Some(t) = s.tabs.iter_mut().find(|t| t.id == tab && t.new_pr.is_open()) {
                    t.new_pr = NewPr::Closed;
                    t.pr = Some(n);
                    t.pr_page = PrPage::Conversation;
                    t.line_target = None;
                }
                s.ensure(false, cx);
            },
            cx,
        );
    }

    fn branch_picker(&self, side: Side, current: Option<String>, cx: &mut Context<Self>) -> Div {
        let colors = palette(cx);
        let (id, trigger, tip) = match side {
            Side::Base => ("compare-base", "compare-base-trigger", "Branch the changes go into"),
            Side::Head => ("compare-head", "compare-head-trigger", "Branch with the changes"),
        };
        self.popover_button(
            id,
            button(trigger, current.clone().unwrap_or_else(|| "Choose a branch".into()))
                .icon(icon(Glyph::Branch).text_color(rgb(colors.muted)))
                .dropdown_caret(true)
                .tooltip(tip),
            Some("Find a branch…"),
            320.,
            move |s, weak, _| {
                let Some(t) = s.current() else { return vec![] };
                let default = s.default_branch(&t.repo);
                let chosen = |side| match (&t.new_pr, side) {
                    (NewPr::Open { base, .. }, Side::Base) => Some(base.clone()),
                    (NewPr::Open { head, .. }, Side::Head) => head.clone(),
                    _ => None,
                };
                let current = chosen(side);
                let Some(branches) = s.data(&t.repo).and_then(|d| d.branches.data.as_ref()) else {
                    return vec![Entry::Item(Item::new("Loading branches…").disabled(true))];
                };
                let mut sorted = branches.iter().collect::<Vec<_>>();
                sorted.sort_by_key(|b| (Some(&b.name) != default.as_ref(), b.name.to_lowercase()));
                let mut out = vec![Entry::Label(format!("Branches · {}", branches.len()).into())];
                for b in sorted {
                    let weak = weak.clone();
                    let name = b.name.clone();
                    out.push(Entry::Item(
                        Item::new(b.name.clone())
                            .icon(if b.protected { Glyph::Lock } else { Glyph::Branch })
                            .checked(Some(&b.name) == current.as_ref())
                            .detail(if Some(&b.name) == default.as_ref() { "default" } else { "" })
                            .on_click(move |_, cx| {
                                let name = name.clone();
                                let _ = weak.update(cx, |s, cx| {
                                    if let Some(t) = s.current_mut() {
                                        t.new_pr.pick(side, name);
                                    }
                                    cx.notify();
                                });
                            }),
                    ));
                }
                out
            },
            cx,
        )
    }

    pub(super) fn compare_view(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let colors = palette(cx);
        let Some(t) = self.current() else { return div().into_any_element() };
        let NewPr::Open { base, head, draft, page, .. } = t.new_pr.clone() else { return div().into_any_element() };
        let data = self.data(&t.repo);
        let range = t.new_pr.range().map(|(b, h)| (b.to_string(), h.to_string()));
        let compare = range.as_ref().and_then(|k| data?.compare.get(k));
        let diff = range.as_ref().and_then(|k| data?.compare_diff.get(k));
        let summary = compare.and_then(|l| l.data.clone());
        let busy = self.busy.is_some();
        if t.new_pr.needs_prefill().is_some() && summary.is_some() {
            cx.defer_in(window, |s, w, cx| s.prefill_new_pr(w, cx));
        }

        let crumb = row()
            .h(px(BAR))
            .flex_shrink_0()
            .px_3()
            .gap_1()
            .text_size(px(13.))
            .child(
                row()
                    .id("crumb-compare")
                    .gap(px(4.))
                    .cursor_pointer()
                    .text_color(rgb(colors.blue))
                    .child(icon(IconName::ChevronLeft).size(px(13.)))
                    .child("Pull requests")
                    .on_click(cx.listener(|s, _, _, cx| {
                        if let Some(t) = s.current_mut() {
                            t.new_pr = NewPr::Closed;
                        }
                        cx.notify();
                    })),
            )
            .child(div().text_color(rgb(colors.quiet)).child("/"))
            .child(div().text_color(rgb(colors.muted)).child("Compare changes"));

        let can_create = !busy && summary.as_ref().is_some_and(|c| !c.commits.is_empty());
        let stats = match &summary {
            Some(c) if c.status == "identical" => Some("These branches are identical.".to_string()),
            Some(c) => Some(format!(
                "{} commit{} ahead · {} behind",
                c.ahead_by,
                if c.ahead_by == 1 { "" } else { "s" },
                c.behind_by
            )),
            None => None,
        };
        let head_row = row()
            .flex_shrink_0()
            .px_4()
            .pt_1()
            .pb_3()
            .gap_2()
            .child(icon(Glyph::Compare).size(px(16.)).text_color(rgb(colors.muted)))
            .child(label(colors, "into"))
            .child(self.branch_picker(Side::Base, Some(base), cx))
            .child(label(colors, "from"))
            .child(self.branch_picker(Side::Head, head.clone(), cx))
            .when_some(stats, |s, text| s.child(div().text_color(rgb(colors.quiet)).child("•")).child(label(colors, text)))
            .child(div().flex_1())
            .child(
                Button::new("create-pr")
                    .label("Create pull request")
                    .icon(Glyph::PullRequest)
                    .primary()
                    .small()
                    .loading(self.busy_with("Creating"))
                    .disabled(!can_create)
                    .on_click(cx.listener(|s, _, _, cx| s.create_pull(cx))),
            );

        let files = diff.and_then(|l| l.data.as_ref()).map(|d| d.files.len());
        let tabs = [
            (ComparePage::Commits, format!("Commits  {}", summary.as_ref().map(|c| c.total_commits.to_string()).unwrap_or_default())),
            (ComparePage::Files, format!("Files changed  {}", files.map(|n| n.to_string()).unwrap_or_default())),
        ];
        let sub = row()
            .h(px(BAR))
            .flex_shrink_0()
            .px_3()
            .gap_2()
            .border_b_1()
            .border_color(rgb(colors.border))
            .child(segmented(colors, "compare-page", &tabs, page, |s, to, _| {
                if let Some(t) = s.current_mut() {
                    t.new_pr.set_page(to);
                }
            }, cx))
            .child(div().flex_1())
            .when_some(diff.and_then(|l| l.data.as_ref()), |s, d| s.child(super::diff::counts(colors, d.additions, d.deletions)));

        let body = if head.is_none() {
            empty(colors, Glyph::Compare, "Choose a branch to compare", "Pick the branch with your changes to see its commits and diff.").into_any_element()
        } else {
            match page {
                ComparePage::Commits => match state(colors, compare, "Comparing branches…") {
                    Err(p) => p.into_any_element(),
                    Ok(()) => self.compare_commits(summary.as_ref().unwrap(), draft, cx),
                },
                ComparePage::Files => match state(colors, diff, "Loading diff…") {
                    Err(p) => p.into_any_element(),
                    Ok(()) => {
                        let diff = diff.and_then(|l| l.data.clone()).unwrap();
                        h_resizable(SharedString::from(format!("compare-files-{}", t.id)))
                            .child(
                                resizable_panel()
                                    .size(px(260.))
                                    .size_range(px(180.)..px(500.))
                                    .child(div().size_full().pt_1().border_r_1().border_color(rgb(colors.row_border)).child(self.diff_files(colors, &diff, t.diff_scroll.clone(), cx))),
                            )
                            .child(resizable_panel().child(self.diff_list(("compare-diff", t.id), diff, t.diff_scroll.clone(), None, cx)))
                            .into_any_element()
                    }
                },
            }
        };
        col()
            .size_full()
            .bg(rgb(colors.background))
            .child(crumb)
            .child(head_row)
            .child(sub)
            .child(div().flex_1().min_h_0().child(body))
            .into_any_element()
    }

    /// The pull request form above the commits it would contain.
    fn compare_commits(&self, c: &gh::Compare, draft: bool, cx: &mut Context<Self>) -> AnyElement {
        let colors = palette(cx);
        let mono = settings::mono(cx);
        let Some(t) = self.current() else { return div().into_any_element() };
        if c.commits.is_empty() {
            return empty(colors, Glyph::Commit, "Nothing to compare", "The head branch has no commits that the base branch doesn’t already have.").into_any_element();
        }
        let form = col()
            .gap_2()
            .child(Input::new(&t.pr_title))
            .child(Input::new(&t.pr_body))
            .child(
                row()
                    .gap_2()
                    .child(
                        Switch::new("new-pr-draft")
                            .checked(draft)
                            .on_click(cx.listener(|s, on: &bool, _, cx| {
                                let on = *on;
                                if let Some(t) = s.current_mut() {
                                    t.new_pr.set_draft(on);
                                }
                                cx.notify();
                            })),
                    )
                    .child(label(colors, "Create as draft"))
                    .child(div().flex_1())
                    .child(label(colors, "Markdown supported")),
            );
        let mut list = card(colors);
        for (i, commit) in c.commits.iter().enumerate() {
            let name = commit.author.clone();
            let sha = commit.sha.clone();
            list = list.child(
                row()
                    .h(px(40.))
                    .px_3()
                    .gap_3()
                    .when(i > 0, |s| s.border_t_1().border_color(rgb(colors.row_border)))
                    .child(icon(Glyph::Commit).text_color(rgb(colors.muted)))
                    .child(self.avatar(commit.login.as_deref(), &name, 18., cx))
                    .child(div().flex_1().min_w_0().truncate().child(commit.subject.clone()))
                    .child(label(colors, name))
                    .child(label(colors, self.when(commit.time, cx)))
                    .child(
                        Button::new(SharedString::from(format!("compare-commit-{i}")))
                            .label(util::short_sha(&commit.sha).to_string())
                            .ghost()
                            .xsmall()
                            .font_family(mono.clone())
                            .tooltip("Copy SHA")
                            .on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(sha.clone()))),
                    ),
            );
        }
        let more = (c.total_commits as usize).saturating_sub(c.commits.len());
        div()
            .id(("compare-commits", t.id))
            .size_full()
            .overflow_y_scroll()
            .child(
                col()
                    .p_4()
                    .gap_4()
                    .max_w(px(900.))
                    .w_full()
                    .mx_auto()
                    .child(form)
                    .child(col().child(section(colors, format!("Commits · {}", c.total_commits))).child(list))
                    .when(more > 0, |s| s.child(label(colors, format!("{more} more commits not shown")))),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{ComparePage, NewPr, Side};

    fn open() -> NewPr {
        NewPr::open("main".into(), None)
    }

    #[test]
    fn opens_without_a_range_until_a_head_is_chosen() {
        let mut n = open();
        assert!(n.is_open());
        assert_eq!(n.range(), None);
        assert_eq!(n.needs_prefill(), None);
        n.pick(Side::Head, "feat/x".into());
        assert_eq!(n.range(), Some(("main", "feat/x")));
        assert_eq!(n.needs_prefill(), Some(("main", "feat/x")));
    }
    #[test]
    fn prefill_happens_once_per_pair() {
        let mut n = NewPr::open("main".into(), Some("a".into()));
        n.mark_prefilled();
        assert_eq!(n.needs_prefill(), None);
        // Picking what is already chosen keeps the user's edits.
        n.pick(Side::Head, "a".into());
        n.pick(Side::Base, "main".into());
        assert_eq!(n.needs_prefill(), None);
        n.pick(Side::Head, "b".into());
        assert_eq!(n.needs_prefill(), Some(("main", "b")));
        n.mark_prefilled();
        n.pick(Side::Base, "dev".into());
        assert_eq!(n.needs_prefill(), Some(("dev", "b")));
    }
    #[test]
    fn closed_ignores_events() {
        let mut n = NewPr::Closed;
        n.pick(Side::Head, "a".into());
        n.set_draft(true);
        n.set_page(ComparePage::Files);
        n.mark_prefilled();
        assert_eq!(n, NewPr::Closed);
    }
    #[test]
    fn keeps_draft_and_page_when_branches_change() {
        let mut n = open();
        n.set_draft(true);
        n.set_page(ComparePage::Files);
        n.pick(Side::Head, "x".into());
        match n {
            NewPr::Open { draft, page, .. } => assert!(draft && page == ComparePage::Files),
            NewPr::Closed => panic!("closed"),
        }
    }
}
