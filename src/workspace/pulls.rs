//! Pull requests: a filterable list, and a detail page with the conversation,
//! commits, checks and files, plus review, merge and comment actions.
use super::ui::*;
use super::*;
use crate::{
    gh::{PrAction, PrFilter},
    icons::Glyph,
    menu::{Entry, Item},
    settings::MergeMethod,
    util,
};
use gpui::prelude::FluentBuilder;
use gpui_component::{
    Disableable,
    IconName, Sizable,
    button::{Button, ButtonVariants},
    input::Input,
    resizable::*,
    tooltip::Tooltip,
};

const PR_ROW: f32 = 56.;
/// Conversation layout: avatar column, gap to the text, reply geometry.
pub(super) const AVATAR: f32 = 34.;
pub(super) const RAIL_INSET: f32 = AVATAR + 12.;
const REPLY_AVATAR: f32 = 24.;
const REPLY_TOP: f32 = 14.;

/// The conversation rail: visible on both themes without competing with text.
pub(super) fn rail_color(colors: Palette) -> u32 {
    mix(colors.border, colors.quiet, 0.45)
}
/// "Nov 11 at 10:43 AM", or "Today at …" / "Yesterday at …".
pub(super) fn stamp(ts: i64) -> String {
    format!("{} at {}", day(ts), clock(ts))
}

pub(super) fn segmented<T: Copy + PartialEq + 'static>(
    colors: Palette,
    id: &'static str,
    options: &[(T, String)],
    current: T,
    on: impl Fn(&mut Workspace, T, &mut Context<Workspace>) + 'static + Clone,
    cx: &mut Context<Workspace>,
) -> Div {
    let mut seg = row()
        .p(px(2.))
        .gap(px(2.))
        .rounded(px(6.))
        .bg(rgb(colors.chrome))
        .border_1()
        .border_color(rgb(colors.row_border));
    for (i, (value, title)) in options.iter().enumerate() {
        let active = *value == current;
        let value = *value;
        let on = on.clone();
        seg = seg.child(
            div()
                .id((id, i))
                .px(px(9.))
                .h(px(22.))
                .flex()
                .items_center()
                .rounded(px(4.))
                .text_size(px(12.5))
                .cursor_pointer()
                .map(|s| {
                    if active {
                        s.bg(rgb(colors.background))
                            .text_color(rgb(colors.text))
                            .font_weight(FontWeight::MEDIUM)
                    } else {
                        s.text_color(rgb(colors.muted)).hover(|s| s.text_color(rgb(colors.text)))
                    }
                })
                .child(title.clone())
                .on_click(cx.listener(move |s, _, _, cx| {
                    on(s, value, cx);
                    cx.notify();
                })),
        );
    }
    seg
}

impl Workspace {
    pub(super) fn pulls_view(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        match self.current().and_then(|t| t.pr) {
            Some(n) => self.pull_page(n, window, cx),
            None => self.pull_list(cx),
        }
    }

    fn pull_list(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = palette(cx);
        let Some(t) = self.current() else { return div().into_any_element() };
        let Some(repo) = self.repo(&t.repo).cloned() else { return div().into_any_element() };
        let l = self.data(&t.repo).and_then(|d| d.pulls.get(&t.pr_filter));
        let q = t.search.read(cx).value().trim().to_lowercase();
        let compare = format!("https://github.com/{}/compare", repo.slug());
        let options = PrFilter::ALL.map(|f| (f, f.title().to_string()));
        let header = row()
            .h(px(BAR + 6.))
            .flex_shrink_0()
            .px_2()
            .gap_2()
            .border_b_1()
            .border_color(rgb(colors.row_border))
            .child(segmented(colors, "pr-filter", &options, t.pr_filter, |s, f, _| {
                if let Some(t) = s.current_mut() {
                    t.pr_filter = f;
                }
            }, cx))
            .child(
                div().w(px(260.)).child(
                    Input::new(&t.search)
                        .small()
                        .prefix(icon(IconName::Search).text_color(rgb(colors.muted)))
                        .cleanable(true),
                ),
            )
            .child(div().flex_1())
            .child(
                Button::new("new-pr")
                    .label("New pull request")
                    .icon(Glyph::Compare)
                    .outline()
                    .small()
                    .tooltip("Compare branches on GitHub")
                    .on_click(move |_, _, cx| cx.open_url(&compare)),
            );
        let body = match state(colors, l, "Loading pull requests…") {
            Err(p) => p.into_any_element(),
            Ok(()) => {
                let pulls = l.and_then(|l| l.data.as_ref()).unwrap();
                let shown = pulls
                    .iter()
                    .filter(|p| {
                        q.is_empty()
                            || p.title.to_lowercase().contains(&q)
                            || p.author.login.to_lowercase().contains(&q)
                            || p.number.to_string() == q.trim_start_matches('#')
                            || p.head_ref_name.to_lowercase().contains(&q)
                    })
                    .collect::<Vec<_>>();
                if shown.is_empty() {
                    empty(
                        colors,
                        Glyph::PullRequest,
                        if q.is_empty() { format!("No {} pull requests", t.pr_filter.title().to_lowercase()) } else { "No matches".into() },
                        if q.is_empty() { "Nothing to review here." } else { "Try another search." },
                    )
                    .into_any_element()
                } else {
                    let shown = Arc::new(shown.into_iter().cloned().collect::<Vec<_>>());
                    uniform_list(
                        ("pr-list", t.id),
                        shown.len(),
                        cx.processor(move |s, range: std::ops::Range<usize>, _, cx| {
                            range.map(|i| s.pr_row(i, &shown[i], cx)).collect()
                        }),
                    )
                    .track_scroll(t.list_scroll.clone())
                    .size_full()
                    .into_any_element()
                }
            }
        };
        col()
            .size_full()
            .bg(rgb(colors.background))
            .child(header)
            .child(div().flex_1().min_h_0().child(body))
            .into_any_element()
    }

    /// One pull request row; fixed height so the list can virtualize.
    fn pr_row(&self, i: usize, p: &gh::Pull, cx: &mut Context<Self>) -> Stateful<Div> {
        let colors = palette(cx);
        let mono = settings::mono(cx);
        let (glyph, color, _, _) = pr_glyph(colors, &p.state, p.is_draft);
        let n = p.number;
        let checks = gh::rollup(&p.status_check_rollup);
        let review = p.review_decision.as_deref().unwrap_or("");
        let tip = format!("{} → {}", p.head_ref_name, p.base_ref_name);
        row()
            .id(("pr", n))
            .w_full()
            .h(px(PR_ROW))
            .px_3()
            .py_2()
            .gap_3()
            .items_start()
            .cursor_pointer()
            .when(i > 0, |s| s.border_t_1().border_color(rgb(colors.row_border)))
            .hover(|s| s.bg(rgb(colors.hover)))
            .child(div().pt(px(2.)).child(icon(glyph).size(px(16.)).text_color(rgb(color))))
            .child(
                col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(4.))
                    .child(
                        row()
                            .w_full()
                            .gap(px(6.))
                            .overflow_hidden()
                            .child(div().min_w_0().truncate().font_weight(FontWeight::MEDIUM).child(p.title.clone()))
                            .children(p.labels.iter().take(3).map(|l| label_pill(colors, l))),
                    )
                    .child(
                        row()
                            .w_full()
                            .gap(px(6.))
                            .overflow_hidden()
                            .text_size(px(12.))
                            .text_color(rgb(colors.muted))
                            .child(format!("#{n}"))
                            .child(format!("opened {} by {}", self.when(util::parse_time(&p.created_at), cx), p.author.login))
                            .child(div().text_color(rgb(colors.quiet)).child("·"))
                            .child(
                                div()
                                    .id(("pr-branch", n))
                                    .min_w_0()
                                    .truncate()
                                    .font_family(mono.clone())
                                    .text_size(px(11.5))
                                    .child(p.head_ref_name.clone())
                                    .tooltip(move |w, cx| Tooltip::new(tip.clone()).build(w, cx)),
                            ),
                    ),
            )
            .child(
                row()
                    .flex_shrink_0()
                    .gap_3()
                    .pt(px(2.))
                    .when_some(review_pill(colors, review), |s, p| s.child(p))
                    .when_some(checks, |s, c| s.child(check_icon(colors, c)))
                    .child(super::diff::counts(colors, p.additions as u32, p.deletions as u32))
                    .child(self.avatar(Some(&p.author.login), &p.author.login, 20., cx)),
            )
            .on_click(cx.listener(move |s, _, _, cx| {
                if let Some(t) = s.current_mut() {
                    t.pr = Some(n);
                    t.line_target = None;
                    t.pr_page = PrPage::Conversation;
                }
                cx.notify();
            }))
    }

    #[allow(clippy::too_many_arguments)]
    fn confirm_pr(
        &mut self,
        number: u64,
        title: String,
        detail: String,
        answer: &'static str,
        action: PrAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let prompt = window.prompt(PromptLevel::Warning, &title, Some(&detail), &[answer, "Cancel"], cx);
        cx.spawn(async move |this, cx| {
            if prompt.await == Ok(0) {
                let _ = this.update(cx, |s, cx| s.pr_action(number, action, cx));
            }
        })
        .detach();
    }

    fn pull_page(&self, n: u64, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let colors = palette(cx);
        let mono = settings::mono(cx);
        let Some(t) = self.current() else { return div().into_any_element() };
        let repo = self.repo(&t.repo).cloned();
        let l = self.data(&t.repo).and_then(|d| d.pull.get(&n));
        let crumb = row()
            .h(px(BAR))
            .flex_shrink_0()
            .px_3()
            .gap_1()
            .text_size(px(13.))
            .child(
                row()
                    .id("crumb-pulls")
                    .gap(px(4.))
                    .cursor_pointer()
                    .text_color(rgb(colors.blue))
                    .child(icon(IconName::ChevronLeft).size(px(13.)))
                    .child("Pull requests")
                    .on_click(cx.listener(|s, _, _, cx| {
                        if let Some(t) = s.current_mut() {
                            t.pr = None;
                            t.line_target = None;
                        }
                        cx.notify();
                    })),
            )
            .child(div().text_color(rgb(colors.quiet)).child("/"))
            .child(div().text_color(rgb(colors.muted)).child(format!("#{n}")))
;
        let Some(pr) = l.and_then(|l| l.data.as_ref()) else {
            let body = match l.and_then(|l| l.error.clone()) {
                Some(e) => failed(colors, &e),
                None => loading(colors, "Loading pull request…"),
            };
            return col().size_full().child(crumb).child(body).into_any_element();
        };
        let p = &pr.summary;
        let (glyph, fg, bg, state) = pr_glyph(colors, &p.state, p.is_draft);
        let open = p.state == "OPEN";
        let busy = self.busy.is_some();
        let method = settings::get(cx).merge_method;

        // Merge: the default method as the main button, the others in its menu.
        let merge = self.popover_button(
            "merge-menu",
            Button::new("merge-trigger")
                .label(method.short())
                .icon(Glyph::Merge)
                .primary()
                .small()
                .dropdown_caret(true)
                .loading(self.busy_with("Merging"))
                .disabled(busy || !open || p.is_draft),
            None,
            280.,
            move |_, weak, cx| {
                let prefs = settings::get(cx);
                let mut out = vec![Entry::Label(format!("Merge #{n}").into())];
                for m in MergeMethod::ALL {
                    let weak = weak.clone();
                    out.push(Entry::Item(
                        Item::new(m.title())
                            .icon(Glyph::Merge)
                            .checked(m == prefs.merge_method)
                            .on_click(move |w, cx| {
                                let _ = weak.update(cx, |s, cx| {
                                    let delete = settings::get(cx).delete_branch;
                                    s.confirm_pr(
                                        n,
                                        format!("{} #{n}?", m.short()),
                                        if delete {
                                            "The head branch will be deleted afterwards. This can’t be undone from here.".into()
                                        } else {
                                            "This can’t be undone from here.".into()
                                        },
                                        m.short(),
                                        PrAction::Merge { method: m, delete_branch: delete },
                                        w,
                                        cx,
                                    )
                                });
                            }),
                    ));
                }
                let weak = weak.clone();
                out.push(Entry::Separator);
                out.push(Entry::Item(
                    Item::new("Delete branch after merge")
                        .checked(prefs.delete_branch)
                        .keep_open()
                        .on_click(move |_, cx| {
                            let _ = weak.update(cx, |s, cx| s.update_settings(cx, |p| p.delete_branch = !p.delete_branch));
                        }),
                ));
                out
            },
            cx,
        );
        let review = self.popover_button(
            "review-menu",
            Button::new("review-trigger")
                .label("Review")
                .icon(Glyph::ListChecks)
                .outline()
                .small()
                .dropdown_caret(true)
                .disabled(busy || !open),
            None,
            300.,
            move |s, weak, cx| {
                let body = s
                    .current()
                    .map(|t| t.composer.read(cx).value().trim().to_string())
                    .unwrap_or_default();
                let (a, b) = (weak.clone(), weak.clone());
                let body2 = body.clone();
                vec![
                    Entry::Item(Item::new("Approve").icon(IconName::CircleCheck).on_click(move |_, cx| {
                        let _ = a.update(cx, |s, cx| s.pr_action(n, PrAction::Approve, cx));
                    })),
                    Entry::Item(
                        Item::new("Request changes")
                            .icon(IconName::TriangleAlert)
                            .detail(if body.is_empty() { "write a comment first" } else { "uses your comment" })
                            .disabled(body.is_empty())
                            .on_click(move |_, cx| {
                                let body = body2.clone();
                                let _ = b.update(cx, |s, cx| s.pr_action(n, PrAction::RequestChanges(body), cx));
                            }),
                    ),
                ]
            },
            cx,
        );
        let local = repo.as_ref().and_then(|r| r.path.clone());
        let url = pr.url.clone();
        let more = self.popover_button(
            "pr-more",
            icon_button("pr-more-trigger", IconName::Ellipsis, "More actions").disabled(busy),
            None,
            260.,
            move |s, weak, _| {
                let Some(t) = s.current() else { return vec![] };
                let Some(pr) = s.data(&t.repo).and_then(|d| d.pull.get(&n)).and_then(|l| l.data.as_ref()) else {
                    return vec![];
                };
                let p = &pr.summary;
                let mut out = vec![];
                if let Some(path) = local.clone() {
                    let w = weak.clone();
                    out.push(Entry::Item(Item::new("Check out locally").icon(Glyph::Branch).detail("gh pr checkout").on_click(move |_, cx| {
                        let path = path.clone();
                        let _ = w.update(cx, |s, cx| s.pr_action(n, PrAction::Checkout(path), cx));
                    })));
                }
                if p.state == "OPEN" {
                    let w = weak.clone();
                    let (title, action) = if p.is_draft {
                        ("Mark ready for review", PrAction::Ready)
                    } else {
                        ("Convert to draft", PrAction::Draft)
                    };
                    out.push(Entry::Item(Item::new(title).icon(Glyph::PullRequestDraft).on_click(move |_, cx| {
                        let action = action.clone();
                        let _ = w.update(cx, |s, cx| s.pr_action(n, action, cx));
                    })));
                    let w = weak.clone();
                    out.push(Entry::Item(Item::new("Close pull request").icon(Glyph::PullRequestClosed).on_click(move |win, cx| {
                        let _ = w.update(cx, |s, cx| {
                            s.confirm_pr(n, format!("Close #{n}?"), "You can reopen it later.".into(), "Close", PrAction::Close, win, cx)
                        });
                    })));
                } else if p.state == "CLOSED" {
                    let w = weak.clone();
                    out.push(Entry::Item(Item::new("Reopen pull request").icon(Glyph::PullRequest).on_click(move |_, cx| {
                        let _ = w.update(cx, |s, cx| s.pr_action(n, PrAction::Reopen, cx));
                    })));
                }
                out.push(Entry::Separator);
                let (u1, u2) = (pr.url.clone(), pr.url.clone());
                out.push(Entry::Item(Item::new("Copy link").icon(IconName::Copy).on_click(move |_, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(u1.clone()))
                })));
                out.push(Entry::Item(Item::new("Open on GitHub").icon(IconName::ExternalLink).on_click(move |_, cx| cx.open_url(&u2))));
                out
            },
            cx,
        );
        let _ = url;
        let head = col()
            .flex_shrink_0()
            .px_4()
            .pt_1()
            .pb_3()
            .gap_2()
            .child(
                row()
                    .gap_3()
                    .items_start()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(19.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .line_height(relative(1.3))
                            .child(format!("{}  ", p.title))
                            .child(div().text_color(rgb(colors.quiet)).font_weight(FontWeight::NORMAL).child(format!("#{n}"))),
                    )
                    .child(
                        row()
                            .gap_1()
                            .flex_shrink_0()
                            .child(review)
                            .child(merge)
                            .child(more),
                    ),
            )
            .child(
                row()
                    .gap_2()
                    .text_size(px(13.))
                    .text_color(rgb(colors.muted))
                    .child(
                        row()
                            .h(px(24.))
                            .px_2()
                            .gap(px(5.))
                            .rounded(px(12.))
                            .bg(rgb(bg))
                            .text_color(rgb(fg))
                            .font_weight(FontWeight::MEDIUM)
                            .text_size(px(12.))
                            .child(icon(glyph).size(px(13.)))
                            .child(state),
                    )
                    .child(self.avatar(Some(&p.author.login), &p.author.login, 18., cx))
                    .child(div().text_color(rgb(colors.text)).child(p.author.login.clone()))
                    .child(match p.state.as_str() {
                        "MERGED" => "merged",
                        "CLOSED" => "wanted to merge",
                        _ => "wants to merge",
                    })
                    .child(format!("{} commit{} into", pr.commits.len(), if pr.commits.len() == 1 { "" } else { "s" }))
                    .child(chip(colors, mono.clone(), p.base_ref_name.clone()))
                    .child("from")
                    .child(chip(colors, mono.clone(), p.head_ref_name.clone()))
                    .map(|s| {
                        // When it ended, or when it was opened while still open.
                        let (what, at) = match p.state.as_str() {
                            "MERGED" => ("merged", pr.merged_at.as_deref()),
                            "CLOSED" => ("closed", pr.closed_at.as_deref()),
                            _ => ("opened", Some(p.created_at.as_str())),
                        };
                        match at.map(util::parse_time).filter(|t| *t > 0) {
                            Some(at) => s
                                .child(div().text_color(rgb(colors.quiet)).child("•"))
                                .child(format!("{what} {}", stamp(at))),
                            None => s,
                        }
                    }),
            );
        let rollup = gh::rollup(&p.status_check_rollup);
        let tabs = [
            (PrPage::Conversation, format!("Conversation  {}", pr.comments.len() + pr.reviews.iter().filter(|r| !r.body.is_empty()).count() + pr.threads.iter().map(|t| 1 + t.replies.len()).sum::<usize>())),
            (PrPage::Commits, format!("Commits  {}", pr.commits.len())),
            (PrPage::Checks, format!("Checks  {}", p.status_check_rollup.len())),
            (PrPage::Files, format!("Files changed  {}", pr.changed_files)),
        ];
        let sub = row()
            .h(px(BAR))
            .flex_shrink_0()
            .px_3()
            .gap_2()
            .border_b_1()
            .border_color(rgb(colors.border))
            .child(segmented(colors, "pr-page", &tabs, t.pr_page, |s, page, _| {
                if let Some(t) = s.current_mut() {
                    t.pr_page = page;
                }
            }, cx))
            .when_some(rollup, |s, r| s.child(check_icon(colors, r)))
            .child(div().flex_1())
            .child(super::diff::counts(colors, p.additions as u32, p.deletions as u32));
        let body = match t.pr_page {
            PrPage::Conversation => self.conversation(pr, window, cx),
            PrPage::Commits => self.pr_commits(pr, cx),
            PrPage::Checks => self.pr_checks(pr, cx),
            PrPage::Files => {
                let l = self.data(&t.repo).and_then(|d| d.pull_diff.get(&n));
                match (l.and_then(|l| l.data.clone()), l.and_then(|l| l.error.clone())) {
                    (Some(diff), _) => h_resizable(SharedString::from(format!("pr-files-{}", t.id)))
                        .child(
                            resizable_panel()
                                .size(px(260.))
                                .size_range(px(180.)..px(500.))
                                .child(div().size_full().pt_1().border_r_1().border_color(rgb(colors.row_border)).child(self.diff_files(colors, &diff, t.diff_scroll.clone(), cx))),
                        )
                        .child(
                            resizable_panel().child(
                                col()
                                    .size_full()
                                    .child(div().flex_1().min_h_0().child(self.diff_list(
                                        ("pr-diff", t.id),
                                        diff,
                                        t.diff_scroll.clone(),
                                        Some(super::diff::Review::new(&pr.threads, t.line_target.as_ref().map(|_| t.line_rows))),
                                        cx,
                                    )))
                                    .when_some(t.line_target.clone(), |c, target| c.child(self.line_panel(n, pr, target, window, cx))),
                            ),
                        )
                        .into_any_element(),
                    (None, Some(e)) => failed(colors, &e).into_any_element(),
                    _ => loading(colors, "Loading diff…").into_any_element(),
                }
            }
        };
        col()
            .size_full()
            .bg(rgb(colors.background))
            .child(crumb)
            .child(head)
            .child(sub)
            .child(div().flex_1().min_h_0().child(body))
            .into_any_element()
    }

    /// The comment box under a pull request diff: the line's existing threads,
    /// then a composer that starts a thread or replies to the last one.
    fn line_panel(&self, n: u64, pr: &gh::PullDetail, target: LineTarget, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let colors = palette(cx);
        let mono = settings::mono(cx);
        let Some(t) = self.current() else { return div() };
        let threads: Vec<&gh::Thread> = pr.threads.iter().filter(|th| super::diff::thread_target(th).as_ref() == Some(&target)).collect();
        let side = if target.right { "" } else { " (removed)" };
        let lines = match &t.line_start {
            Some(start) if start.right == target.right => format!("{}–{}", start.line, target.line),
            Some(start) => format!("{}{}–{}", start.line, if start.right { "" } else { " (removed)" }, target.line),
            None => target.line.to_string(),
        };
        let header = row()
            .gap_2()
            .text_size(px(12.5))
            .child(icon(Glyph::Message).size(px(14.)).text_color(rgb(colors.blue)))
            .child(div().text_color(rgb(colors.muted)).child("Comment on"))
            .child(tinted_chip(colors.text, colors.hover, mono, format!("{}:{lines}{side}", target.path.rsplit('/').next().unwrap_or(&target.path))).max_w(px(520.)))
            .child(div().flex_1())
            .child(icon_button("close-line-comment", IconName::Close, "Close").on_click(cx.listener(|s, _, _, cx| {
                if let Some(t) = s.current_mut() {
                    t.line_target = None;
                }
                cx.notify();
            })));
        let mut list = col().gap_3();
        for (k, th) in threads.iter().enumerate() {
            for (j, c) in std::iter::once(&th.root).chain(th.replies.iter()).enumerate() {
                let at = util::parse_time(&c.created_at);
                list = list.child(
                    div()
                        .flex()
                        .items_start()
                        .gap_2()
                        .when(j > 0, |d| d.pl(px(28.)))
                        .child(self.avatar(Some(&c.login()), &c.login(), 20., cx))
                        .child(
                            col()
                                .flex_1()
                                .min_w_0()
                                .gap(px(2.))
                                .child(
                                    row()
                                        .gap(px(6.))
                                        .text_size(px(12.5))
                                        .child(div().font_weight(FontWeight::SEMIBOLD).child(c.login()))
                                        .child(div().text_color(rgb(colors.quiet)).child("•"))
                                        .child(div().text_color(rgb(colors.muted)).child(stamp(at))),
                                )
                                .child(markdown(format!("pr-{n}-line-{k}-{j}-{}", c.id), &c.body, window, cx)),
                        ),
                );
            }
        }
        let replying = !threads.is_empty();
        col()
            .flex_shrink_0()
            .max_h(px(420.))
            .gap_3()
            .p_3()
            .border_t_1()
            .border_color(rgb(colors.border))
            .bg(rgb(colors.panel))
            .child(header)
            .when(replying, |c| {
                c.child(
                    div()
                        .id(("line-threads", t.id))
                        .flex_shrink()
                        .min_h_0()
                        .max_h(px(220.))
                        .overflow_y_scroll()
                        .child(list),
                )
            })
            .child(Input::new(&t.line_composer))
            .child(
                row()
                    .gap_2()
                    .child(label(colors, if replying { "Markdown · ⌘ Return to send" } else { "Markdown · ⌘ Return to send · ⇧-click a line for a range" }))
                    .child(div().flex_1())
                    .child(Button::new("cancel-line-comment").label("Cancel").ghost().small().on_click(cx.listener(|s, _, _, cx| {
                        if let Some(t) = s.current_mut() {
                            t.line_target = None;
                        }
                        cx.notify();
                    })))
                    .child(
                        Button::new("send-line-comment")
                            .label(if replying { "Reply" } else { "Comment" })
                            .icon(Glyph::Message)
                            .primary()
                            .small()
                            .loading(self.busy_with("Commenting"))
                            .disabled(self.busy.is_some())
                            .on_click(cx.listener(|s, _, w, cx| s.submit(&Submit, w, cx))),
                    ),
            )
    }

    /// A message's header and Markdown body; the avatar goes on the rail.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn message(&self, id: String, login: &str, verb: Option<Div>, when: i64, body: &str, accent: Option<(u32, u32, &'static str)>, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let colors = palette(cx);
        col()
            .gap(px(6.))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(6.))
                    .text_size(px(13.))
                    .child(div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).child(login.to_string()))
                    .children(verb)
                    .child(div().text_color(rgb(colors.quiet)).child("•"))
                    .child(div().text_color(rgb(colors.muted)).child(stamp(when)))
                    .when_some(accent, |s, (fg, bg, text)| s.child(pill(text, fg, bg))),
            )
            .child(if body.trim().is_empty() {
                div().text_color(rgb(colors.muted)).child("No description provided.").into_any_element()
            } else {
                markdown(id, body, window, cx)
            })
    }
    /// A round badge holding an event's glyph, in the avatar column.
    pub(super) fn badge(&self, glyph: impl Into<gpui_component::Icon>, color: u32, cx: &App) -> AnyElement {
        let colors = palette(cx);
        div()
            .size(px(AVATAR))
            .flex_shrink_0()
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(colors.chrome))
            .border_1()
            .border_color(rgb(colors.row_border))
            .child(icon(glyph).size(px(15.)).text_color(rgb(color)))
            .into_any_element()
    }
    /// An inline review comment with its replies. Each reply branches off the
    /// rail on a curve, so the thread reads as one conversation.
    fn review_thread(&self, n: u64, k: usize, thread: &gh::Thread, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let colors = palette(cx);
        let mono = settings::mono(cx);
        let root = &thread.root;
        let line = match (root.start_line, root.line.or(root.original_line)) {
            (Some(a), Some(b)) if a != b => format!(":{a}–{b}"),
            (_, Some(l)) => format!(":{l}"),
            _ => String::new(),
        };
        let verb = row()
            .gap(px(6.))
            .child(div().text_color(rgb(colors.muted)).child("commented on"))
            .child(tinted_chip(colors.text, colors.hover, mono, format!("{}{line}", root.path)));
        let mut out = self.message(
            format!("pr-{n}-t{k}"),
            &root.login(),
            Some(verb),
            util::parse_time(&root.created_at),
            &root.body,
            None,
            window,
            cx,
        );
        let rail = rail_color(colors);
        for (j, reply) in thread.replies.iter().enumerate() {
            // From the rail (centre of the avatar column) down and round into the
            // reply's avatar. Painted outside the reply's own box, to its left.
            let bend = canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    let o = bounds.origin;
                    let cy = o.y + px(REPLY_TOP + REPLY_AVATAR / 2.);
                    let end = o.x + bounds.size.width;
                    let mut b = PathBuilder::stroke(px(2.));
                    b.move_to(point(o.x, cy - px(14.)));
                    b.curve_to(point(o.x + px(12.), cy), point(o.x, cy));
                    b.line_to(point(end, cy));
                    if let Ok(path) = b.build() {
                        window.paint_path(path, rgb(rail));
                    }
                },
            )
            .absolute()
            .top_0()
            .left(px(-(RAIL_INSET - AVATAR / 2.)))
            .w(px(RAIL_INSET - AVATAR / 2. - 4.))
            .h(px(REPLY_TOP + REPLY_AVATAR));
            let at = util::parse_time(&reply.created_at);
            out = out.child(
                div()
                    .relative()
                    .flex()
                    .items_start()
                    .gap_2()
                    .pt(px(REPLY_TOP))
                    .child(bend)
                    .child(self.avatar(Some(&reply.login()), &reply.login(), REPLY_AVATAR, cx))
                    .child(
                        col()
                            .flex_1()
                            .min_w_0()
                            .gap(px(4.))
                            .child(
                                row()
                                    .gap(px(6.))
                                    .text_size(px(12.5))
                                    .child(div().font_weight(FontWeight::SEMIBOLD).child(reply.login()))
                                    .child(div().text_color(rgb(colors.quiet)).child("•"))
                                    .child(div().text_color(rgb(colors.muted)).child(stamp(at))),
                            )
                            .child(markdown(format!("pr-{n}-t{k}-r{j}"), &reply.body, window, cx)),
                    ),
            );
        }
        out
    }

    fn conversation(&self, pr: &gh::PullDetail, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let colors = palette(cx);
        let Some(t) = self.current() else { return div().into_any_element() };
        let n = pr.summary.number;
        let p = &pr.summary;
        // Everything on the rail, oldest first: (time, avatar or badge, content).
        let mut items: Vec<(i64, AnyElement, Div)> = vec![];
        let opened = util::parse_time(&p.created_at);
        let verb = |text: &str| div().text_color(rgb(colors.muted)).child(text.to_string());
        items.push((
            opened,
            self.avatar(Some(&p.author.login), &p.author.login, AVATAR, cx),
            self.message(format!("pr-{n}-body"), &p.author.login, Some(verb("opened this")), opened, &pr.body, None, window, cx),
        ));
        for (i, c) in pr.comments.iter().enumerate() {
            let at = util::parse_time(&c.created_at);
            items.push((
                at,
                self.avatar(Some(&c.author.login), &c.author.login, AVATAR, cx),
                self.message(format!("pr-{n}-c{i}"), &c.author.login, None, at, &c.body, None, window, cx),
            ));
        }
        for (i, r) in pr.reviews.iter().enumerate() {
            let at = util::parse_time(r.submitted_at.as_deref().unwrap_or(""));
            let accent = match r.state.as_str() {
                "APPROVED" => Some((colors.green, colors.green_tint, "Approved")),
                "CHANGES_REQUESTED" => Some((colors.red, colors.red_tint, "Changes requested")),
                "DISMISSED" => Some((colors.muted, colors.hover, "Dismissed")),
                _ => None,
            };
            if r.body.trim().is_empty() {
                // A bare approval reads as a one-line event.
                let Some((fg, _, text)) = accent else { continue };
                let glyph = if r.state == "APPROVED" { IconName::CircleCheck } else { IconName::TriangleAlert };
                let line = row()
                    .min_h(px(AVATAR))
                    .gap(px(6.))
                    .text_size(px(13.))
                    .child(self.avatar(Some(&r.author.login), &r.author.login, 18., cx))
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(r.author.login.clone()))
                    .child(div().text_color(rgb(fg)).child(match r.state.as_str() {
                        "APPROVED" => "approved these changes".to_string(),
                        "CHANGES_REQUESTED" => "requested changes".to_string(),
                        _ => text.to_lowercase(),
                    }))
                    .child(div().text_color(rgb(colors.quiet)).child("•"))
                    .child(div().text_color(rgb(colors.muted)).child(stamp(at)));
                items.push((at, self.badge(glyph, fg, cx), line));
            } else {
                items.push((
                    at,
                    self.avatar(Some(&r.author.login), &r.author.login, AVATAR, cx),
                    self.message(format!("pr-{n}-r{i}"), &r.author.login, Some(verb("reviewed")), at, &r.body, accent, window, cx),
                ));
            }
        }
        for (k, thread) in pr.threads.iter().enumerate() {
            let login = thread.root.login();
            items.push((
                util::parse_time(&thread.root.created_at),
                self.avatar(Some(&login), &login, AVATAR, cx),
                self.review_thread(n, k, thread, window, cx),
            ));
        }
        items.sort_by_key(|(at, _, _)| *at);

        // Where the pull request stands, with when it merged or closed.
        let ended = match p.state.as_str() {
            "MERGED" => pr.merged_at.as_deref(),
            "CLOSED" => pr.closed_at.as_deref(),
            _ => None,
        }
        .map(util::parse_time)
        .filter(|t| *t > 0);
        let (glyph, color, title, detail): (gpui_component::Icon, u32, &str, String) = match (p.state.as_str(), pr.mergeable.as_str(), pr.merge_state_status.as_str()) {
            ("MERGED", _, _) => (Glyph::Merge.into(), colors.purple, "Merged", format!("into {}", p.base_ref_name)),
            ("CLOSED", _, _) => (Glyph::PullRequestClosed.into(), colors.red, "Closed", "without merging".to_string()),
            (_, "CONFLICTING", _) => (IconName::TriangleAlert.into(), colors.red, "Conflicts", "Resolve conflicts with the base branch before merging.".into()),
            (_, _, "BLOCKED") => (Glyph::Lock.into(), colors.amber, "Blocked", "Required reviews or checks haven’t passed yet.".into()),
            (_, _, "BEHIND") => (Glyph::Fetch.into(), colors.amber, "Behind base", "The head branch is out of date with the base branch.".into()),
            (_, _, "UNSTABLE") => (IconName::TriangleAlert.into(), colors.amber, "Checks failing", "Some non-required checks are failing.".into()),
            (_, _, "DRAFT") => (Glyph::PullRequestDraft.into(), colors.muted, "Draft", "Mark it ready for review to merge.".into()),
            (_, "MERGEABLE", _) => (IconName::CircleCheck.into(), colors.green, "Ready to merge", "No conflicts with the base branch.".into()),
            _ => (Glyph::CircleDashed.into(), colors.muted, "Checking mergeability", "GitHub is still computing whether this can merge.".into()),
        };
        let status = row()
            .min_h(px(AVATAR))
            .gap(px(6.))
            .text_size(px(13.))
            .child(div().flex_shrink_0().font_weight(FontWeight::SEMIBOLD).text_color(rgb(color)).child(title))
            .child(div().min_w_0().truncate().text_color(rgb(colors.muted)).child(detail))
            .when_some(ended, |s, at| {
                s.child(div().text_color(rgb(colors.quiet)).child("•"))
                    .child(div().flex_shrink_0().text_color(rgb(colors.muted)).child(stamp(at)))
            });
        items.push((i64::MAX, self.badge(glyph, color, cx), status));

        let composer = col()
            .gap_2()
            .child(Input::new(&t.composer))
            .child(
                row()
                    .gap_2()
                    .child(label(colors, "Markdown · ⌘ Return to send"))
                    .child(div().flex_1())
                    .child(
                        Button::new("send-comment")
                            .label("Comment")
                            .icon(Glyph::Message)
                            .primary()
                            .small()
                            .loading(self.busy_with("Commenting"))
                            .disabled(self.busy.is_some())
                            .on_click(cx.listener(|s, _, w, cx| s.submit(&Submit, w, cx))),
                    ),
            );
        let me = self.user.data.as_ref().map(|u| u.login.clone());
        items.push((i64::MAX, self.avatar(me.as_deref(), "you", AVATAR, cx), composer));

        // The rail: each item's avatar column carries the line down to the next.
        let last = items.len() - 1;
        let mut thread = col();
        for (i, (_, avatar, content)) in items.into_iter().enumerate() {
            thread = thread.child(
                div()
                    .relative()
                    .flex()
                    .w_full()
                    // Absolute so it spans the item's full height, through to the next avatar.
                    .when(i < last, |d| {
                        d.child(
                            div()
                                .absolute()
                                .left(px(AVATAR / 2. - 1.))
                                .top(px(AVATAR))
                                .bottom_0()
                                .w(px(2.))
                                .bg(rgb(rail_color(colors))),
                        )
                    })
                    .child(col().w(px(AVATAR)).flex_shrink_0().items_center().child(avatar))
                    .child(
                        content
                            .flex_1()
                            .min_w_0()
                            .pl(px(RAIL_INSET - AVATAR))
                            .pt(px(7.))
                            .pb(px(if i < last { 26. } else { 0. })),
                    ),
            );
        }
        div()
            .id(("conversation", t.id))
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&t.page_scroll)
            .child(col().p_4().max_w(px(900.)).w_full().mx_auto().child(thread))
            .into_any_element()
    }
    fn pr_commits(&self, pr: &gh::PullDetail, cx: &mut Context<Self>) -> AnyElement {
        let colors = palette(cx);
        let mono = settings::mono(cx);
        if pr.commits.is_empty() {
            return empty(colors, Glyph::Commit, "No commits", "").into_any_element();
        }
        let mut list = card(colors);
        for (i, c) in pr.commits.iter().enumerate() {
            let author = c.authors.first().cloned().unwrap_or_default();
            let name = author.name.clone().filter(|n| !n.is_empty()).unwrap_or_else(|| author.login.clone());
            let sha = c.oid.clone();
            list = list.child(
                row()
                    .h(px(40.))
                    .px_3()
                    .gap_3()
                    .when(i > 0, |s| s.border_t_1().border_color(rgb(colors.row_border)))
                    .child(icon(Glyph::Commit).text_color(rgb(colors.muted)))
                    .child(self.avatar(Some(&author.login), &name, 18., cx))
                    .child(div().flex_1().min_w_0().truncate().child(c.message_headline.clone()))
                    .child(label(colors, name))
                    .child(label(colors, self.when(util::parse_time(&c.authored_date), cx)))
                    .child(
                        Button::new(SharedString::from(format!("pr-commit-{i}")))
                            .label(util::short_sha(&c.oid).to_string())
                            .ghost()
                            .xsmall()
                            .font_family(mono.clone())
                            .tooltip("Copy SHA")
                            .on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(sha.clone()))),
                    ),
            );
        }
        div()
            .id("pr-commits")
            .size_full()
            .overflow_y_scroll()
            .child(col().p_4().max_w(px(900.)).w_full().mx_auto().child(list))
            .into_any_element()
    }

    fn pr_checks(&self, pr: &gh::PullDetail, cx: &mut Context<Self>) -> AnyElement {
        let colors = palette(cx);
        let checks = &pr.summary.status_check_rollup;
        if checks.is_empty() {
            return empty(colors, Glyph::ListChecks, "No checks", "No status checks were reported for the head commit.").into_any_element();
        }
        let mut sorted = checks.iter().collect::<Vec<_>>();
        sorted.sort_by_key(|c| match c.outcome() {
            gh::CheckState::Fail => 0,
            gh::CheckState::Pending => 1,
            gh::CheckState::Pass => 2,
            gh::CheckState::Skipped => 3,
        });
        let mut list = card(colors);
        for (i, c) in sorted.into_iter().enumerate() {
            let url = c.url();
            list = list.child(
                row()
                    .id(("check", i))
                    .h(px(38.))
                    .px_3()
                    .gap_3()
                    .when(i > 0, |s| s.border_t_1().border_color(rgb(colors.row_border)))
                    .when(url.is_some(), |s| s.cursor_pointer().hover(|s| s.bg(rgb(colors.hover))))
                    .child(check_icon(colors, c.outcome()))
                    .child(div().flex_1().min_w_0().truncate().child(c.title()))
                    .when_some(c.workflow_name.clone().filter(|w| !w.is_empty()), |s, w| s.child(label(colors, w)))
                    .when(url.is_some(), |s| s.child(icon(IconName::ExternalLink).size(px(12.)).text_color(rgb(colors.quiet))))
                    .when_some(url, |s, url| s.on_click(move |_, _, cx| cx.open_url(&url))),
            );
        }
        div()
            .id("pr-checks")
            .size_full()
            .overflow_y_scroll()
            .child(col().p_4().max_w(px(900.)).w_full().mx_auto().child(list))
            .into_any_element()
    }
}

pub(super) fn label_pill(colors: Palette, l: &gh::Label) -> Div {
    let hex = u32::from_str_radix(l.color.trim_start_matches('#'), 16).unwrap_or(colors.quiet);
    row()
        .flex_shrink_0()
        .h(px(18.))
        .px(px(7.))
        .gap(px(5.))
        .rounded(px(9.))
        .border_1()
        .border_color(rgb(colors.border))
        .text_size(px(11.))
        .text_color(rgb(colors.muted))
        .child(div().size(px(7.)).rounded_full().bg(rgb(hex)))
        .child(l.name.clone())
}
fn review_pill(colors: Palette, decision: &str) -> Option<Div> {
    Some(match decision {
        "APPROVED" => pill("Approved", colors.green, colors.green_tint),
        "CHANGES_REQUESTED" => pill("Changes requested", colors.red, colors.red_tint),
        "REVIEW_REQUIRED" => pill("Review required", colors.amber, colors.amber_tint),
        _ => return None,
    })
}
