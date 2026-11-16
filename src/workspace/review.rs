//! Batched reviews: line comments saved as pending drafts, then sent together
//! with a summary and a verdict in one call, as on GitHub's "Start a review".
use super::pulls::segmented;
use super::ui::*;
use super::*;
use crate::{gh::ReviewEvent, icons::Glyph};
use gpui_component::{
    Disableable, IconName, Sizable,
    button::{Button, ButtonVariants},
    input::Input,
};

actions!(gitflow, [FinishReview]);

/// A pending line comment, with an id the UI (and later other callers) can
/// delete it by.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Draft {
    pub id: u64,
    pub comment: gh::DraftComment,
}
impl Draft {
    /// The diff line this comment is attached to.
    pub fn target(&self) -> LineTarget {
        LineTarget {
            path: self.comment.path.clone(),
            line: self.comment.line,
            right: self.comment.right,
        }
    }
}

/// Every pending review, keyed by (repository key, pull request number). They
/// live on the workspace, not on a tab, so any tab on the same pull request
/// shows the same drafts and they outlive the tab.
#[derive(Default)]
pub struct ReviewDrafts {
    next_id: u64,
    by_pr: HashMap<(String, u64), Vec<Draft>>,
}
impl ReviewDrafts {
    pub fn add(&mut self, repo: &str, pr: u64, comment: gh::DraftComment) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        self.by_pr.entry((repo.to_string(), pr)).or_default().push(Draft { id, comment });
        id
    }
    /// Delete one draft; false if it was already gone.
    pub fn remove(&mut self, repo: &str, pr: u64, id: u64) -> bool {
        self.discard(repo, pr, |d| d.id == id) > 0
    }
    /// A review was sent: drop the drafts it carried. Drafts added while it was
    /// being sent stay for the next one.
    pub fn submitted(&mut self, repo: &str, pr: u64, sent: &[u64]) {
        self.discard(repo, pr, |d| sent.contains(&d.id));
    }
    fn discard(&mut self, repo: &str, pr: u64, gone: impl Fn(&Draft) -> bool) -> usize {
        let key = (repo.to_string(), pr);
        let Some(list) = self.by_pr.get_mut(&key) else { return 0 };
        let before = list.len();
        list.retain(|d| !gone(d));
        let removed = before - list.len();
        if list.is_empty() {
            self.by_pr.remove(&key);
        }
        removed
    }
    pub fn list(&self, repo: &str, pr: u64) -> &[Draft] {
        self.by_pr.get(&(repo.to_string(), pr)).map(Vec::as_slice).unwrap_or(&[])
    }
    pub fn count(&self, repo: &str, pr: u64) -> usize {
        self.list(repo, pr).len()
    }
    /// How many drafts sit on each line, for the diff gutter.
    pub fn per_line(&self, repo: &str, pr: u64) -> HashMap<LineTarget, usize> {
        let mut out = HashMap::new();
        for d in self.list(repo, pr) {
            *out.entry(d.target()).or_insert(0) += 1;
        }
        out
    }
}

/// Whether GitHub will accept a review with this verdict, summary and number of
/// line comments; otherwise what to tell the reviewer.
pub fn ready(event: ReviewEvent, summary: &str, comments: usize) -> Result<(), &'static str> {
    let empty = summary.trim().is_empty();
    match event {
        ReviewEvent::RequestChanges if empty => Err("Say what needs to change before requesting changes."),
        ReviewEvent::Comment if empty && comments == 0 => Err("Write a summary or add a line comment first."),
        _ => Ok(()),
    }
}

impl Workspace {
    /// The pull request the active tab is on, as its draft key.
    fn review_target(&self) -> Option<(String, u64)> {
        let t = self.current()?;
        Some((t.repo.clone(), t.pr?))
    }

    /// Keep the comment box's text as a pending line comment.
    pub(super) fn add_draft_line(&mut self, n: u64, cx: &mut Context<Self>) {
        let Some(t) = self.current() else { return };
        let Some(target) = t.line_target.clone() else { return };
        let body = t.line_composer.read(cx).value().trim().to_string();
        if body.is_empty() {
            return;
        }
        let comment = gh::DraftComment {
            start: t.line_start.as_ref().map(|s| (s.line, s.right)),
            path: target.path,
            line: target.line,
            right: target.right,
            body,
        };
        let (key, tab) = (t.repo.clone(), t.id);
        self.drafts.add(&key, n, comment);
        self.pending_line_clear = Some(tab);
        cx.notify();
    }
    pub(super) fn remove_draft(&mut self, n: u64, id: u64, cx: &mut Context<Self>) {
        if let Some(t) = self.current() {
            let key = t.repo.clone();
            self.drafts.remove(&key, n, id);
        }
        cx.notify();
    }

    pub(super) fn open_finish_review(&mut self, _: &FinishReview, window: &mut Window, cx: &mut Context<Self>) {
        if self.review_target().is_none() {
            return;
        }
        self.popup = None;
        self.overlay = Overlay::FinishReview { event: ReviewEvent::Comment };
        self.review_body.update(cx, |s, cx| s.focus(window, cx));
        cx.notify();
    }

    /// Send the pending comments with the summary and verdict from the dialog.
    pub(super) fn finish_review(&mut self, cx: &mut Context<Self>) {
        let Overlay::FinishReview { event } = self.overlay else { return };
        let Some((key, n)) = self.review_target() else { return };
        let Some(repo) = self.repo(&key).cloned() else { return };
        let slug = repo.slug();
        let commit = self
            .data(&key)
            .and_then(|d| d.pull.get(&n))
            .and_then(|l| l.data.as_ref())
            .map(|pr| pr.head_ref_oid.clone())
            .filter(|c| !c.is_empty());
        let Some(commit) = commit else {
            self.notice = Notice::warn("The pull request hasn’t finished loading.");
            cx.notify();
            return;
        };
        let body = self.review_body.read(cx).value().trim().to_string();
        let drafts = self.drafts.list(&key, n).to_vec();
        if let Err(why) = ready(event, &body, drafts.len()) {
            self.notice = Notice::warn(why);
            cx.notify();
            return;
        }
        let ids: Vec<u64> = drafts.iter().map(|d| d.id).collect();
        let action = gh::PrAction::SubmitReview {
            commit,
            event,
            body,
            comments: drafts.into_iter().map(|d| d.comment).collect(),
        };
        let tab = self.current().map(|t| t.id);
        self.mutate(
            format!("Submitting review on #{n}"),
            Report::Notice,
            move || gh::pr_action(&slug, n, &action),
            move |s, cx| {
                s.drafts.submitted(&key, n, &ids);
                if matches!(s.overlay, Overlay::FinishReview { .. }) {
                    s.overlay = Overlay::None;
                }
                s.pending_review_clear = true;
                if let Some(d) = s.data.get_mut(&key) {
                    // The review's comments arrive with the pull request; the diff is unchanged.
                    d.pull.remove(&n);
                    d.pulls.clear();
                }
                if let Some(t) = s.tabs.iter_mut().find(|t| Some(t.id) == tab) {
                    t.line_target = None;
                }
                s.ensure(false, cx);
            },
            cx,
        );
    }

    /// One pending comment: where it goes, its text, and a way to delete it.
    pub(super) fn pending_comment(&self, n: u64, d: &Draft, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let colors = palette(cx);
        let mono = settings::mono(cx);
        let c = &d.comment;
        let line = match c.start {
            Some((first, _)) if first != c.line => format!(":{first}–{}", c.line),
            _ => format!(":{}", c.line),
        };
        let id = d.id;
        col()
            .gap(px(6.))
            .child(
                row()
                    .gap(px(6.))
                    .text_size(px(13.))
                    .child(div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).child("You"))
                    .child(div().text_color(rgb(colors.muted)).child("commented on"))
                    .child(tinted_chip(colors.text, colors.hover, mono, format!("{}{line}", c.path)))
                    .child(pill("Pending", colors.amber, colors.amber_tint))
                    .child(div().flex_1())
                    .child(
                        icon_button(("delete-draft", id), IconName::Delete, "Delete pending comment")
                            .xsmall()
                            .on_click(cx.listener(move |s, _, _, cx| s.remove_draft(n, id, cx))),
                    ),
            )
            .child(markdown(format!("pr-{n}-draft-{id}"), &c.body, window, cx))
    }

    /// The pending count with the button that sends it, shown on every PR page.
    pub(super) fn pending_bar(&self, count: usize, cx: &mut Context<Self>) -> Div {
        let colors = palette(cx);
        row()
            .gap_2()
            .child(pill(format!("{count} pending comment{}", if count == 1 { "" } else { "s" }), colors.amber, colors.amber_tint))
            .child(
                Button::new("finish-review")
                    .label("Finish review")
                    .icon(Glyph::ListChecks)
                    .primary()
                    .small()
                    .disabled(self.busy.is_some())
                    .on_click(cx.listener(|s, _, w, cx| s.open_finish_review(&FinishReview, w, cx))),
            )
    }

    pub(super) fn finish_review_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = palette(cx);
        let Overlay::FinishReview { event } = self.overlay else { return div().into_any_element() };
        let pending = self.review_target().map(|(k, n)| self.drafts.count(&k, n)).unwrap_or(0);
        let options = ReviewEvent::ALL.map(|e| (e, e.title().to_string()));
        div()
            .absolute()
            .inset_0()
            .occlude()
            .flex()
            .justify_center()
            .items_start()
            .pt(px(120.))
            .bg(rgba(colors.scrim))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|s, _, _, cx| {
                    s.overlay = Overlay::None;
                    cx.notify();
                }),
            )
            .child(
                col()
                    .w(px(560.))
                    .h_auto()
                    .bg(rgb(colors.background))
                    .border_1()
                    .border_color(rgb(colors.border))
                    .rounded(px(10.))
                    .shadow_lg()
                    .overflow_hidden()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        row()
                            .h(px(BAR + 6.))
                            .px_4()
                            .gap_2()
                            .bg(rgb(colors.chrome))
                            .rounded_t(px(9.))
                            .border_b_1()
                            .border_color(rgb(colors.border))
                            .child(icon(Glyph::ListChecks).text_color(rgb(colors.muted)))
                            .child(div().flex_1().font_weight(FontWeight::MEDIUM).child("Finish your review"))
                            .child(label(colors, "esc")),
                    )
                    .child(
                        col()
                            .p_4()
                            .gap_3()
                            .child(Input::new(&self.review_body))
                            .child(segmented(colors, "review-event", &options, event, |s, e, _| s.overlay = Overlay::FinishReview { event: e }, cx))
                            .child(
                                row()
                                    .gap_2()
                                    .child(
                                        label(
                                            colors,
                                            match pending {
                                                0 => "No line comments. Markdown · ⌘ Return to send".to_string(),
                                                1 => "1 pending comment goes with it · ⌘ Return to send".to_string(),
                                                n => format!("{n} pending comments go with it · ⌘ Return to send"),
                                            },
                                        )
                                        .flex_1()
                                        .min_w_0(),
                                    )
                                    .child(Button::new("cancel-review").label("Cancel").ghost().small().on_click(cx.listener(|s, _, _, cx| {
                                        s.overlay = Overlay::None;
                                        cx.notify();
                                    })))
                                    .child(
                                        Button::new("submit-review")
                                            .label("Submit review")
                                            .primary()
                                            .small()
                                            .loading(self.busy_with("Submitting review"))
                                            .disabled(self.busy.is_some())
                                            .on_click(cx.listener(|s, _, _, cx| s.finish_review(cx))),
                                    ),
                            ),
                    ),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{ReviewDrafts, ready};
    use crate::gh::{DraftComment, ReviewEvent};

    fn comment(path: &str, line: u32) -> DraftComment {
        DraftComment {
            path: path.into(),
            line,
            right: true,
            start: None,
            body: format!("on {path}:{line}"),
        }
    }

    #[test]
    fn drafts_are_kept_per_pull_request() {
        let mut d = ReviewDrafts::default();
        d.add("o/r", 1, comment("a.rs", 1));
        d.add("o/r", 1, comment("a.rs", 2));
        d.add("o/r", 2, comment("b.rs", 1));
        d.add("x/y", 1, comment("c.rs", 1));
        assert_eq!(d.count("o/r", 1), 2);
        assert_eq!(d.count("o/r", 2), 1);
        assert_eq!(d.count("x/y", 1), 1);
        assert_eq!(d.count("o/r", 3), 0);
    }
    #[test]
    fn ids_are_unique_and_removal_targets_one_draft() {
        let mut d = ReviewDrafts::default();
        let a = d.add("o/r", 1, comment("a.rs", 1));
        let b = d.add("o/r", 1, comment("a.rs", 2));
        assert_ne!(a, b);
        assert!(d.remove("o/r", 1, a));
        assert!(!d.remove("o/r", 1, a));
        assert_eq!(d.list("o/r", 1).iter().map(|x| x.id).collect::<Vec<_>>(), vec![b]);
        // Another pull request's draft with the same id is never touched.
        assert!(!d.remove("o/r", 2, b));
        assert!(d.remove("o/r", 1, b));
        assert_eq!(d.count("o/r", 1), 0);
    }
    #[test]
    fn a_sent_review_leaves_drafts_added_since() {
        let mut d = ReviewDrafts::default();
        let a = d.add("o/r", 1, comment("a.rs", 1));
        let b = d.add("o/r", 1, comment("a.rs", 2));
        // The review was built from a and b; c arrives while it is in flight.
        let c = d.add("o/r", 1, comment("a.rs", 3));
        d.submitted("o/r", 1, &[a, b]);
        assert_eq!(d.list("o/r", 1).iter().map(|x| x.id).collect::<Vec<_>>(), vec![c]);
        d.submitted("o/r", 1, &[c]);
        assert_eq!(d.count("o/r", 1), 0);
    }
    #[test]
    fn counts_drafts_per_line() {
        let mut d = ReviewDrafts::default();
        d.add("o/r", 1, comment("a.rs", 4));
        d.add("o/r", 1, comment("a.rs", 4));
        let mut left = comment("a.rs", 4);
        left.right = false;
        d.add("o/r", 1, left);
        let lines = d.per_line("o/r", 1);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines.values().sum::<usize>(), 3);
        assert!(d.per_line("o/r", 9).is_empty());
    }
    #[test]
    fn github_needs_something_to_say() {
        assert!(ready(ReviewEvent::Approve, "", 0).is_ok());
        assert!(ready(ReviewEvent::Comment, "", 0).is_err());
        assert!(ready(ReviewEvent::Comment, "  ", 2).is_ok());
        assert!(ready(ReviewEvent::Comment, "looks good", 0).is_ok());
        assert!(ready(ReviewEvent::RequestChanges, "", 3).is_err());
        assert!(ready(ReviewEvent::RequestChanges, "fix it", 0).is_ok());
    }
}
