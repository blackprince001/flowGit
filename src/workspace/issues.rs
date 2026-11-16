//! Issues: a filterable list, and a detail page with the conversation, a comment
//! box, and close and reopen buttons. Laid out like the pull request view.
use super::pulls::{AVATAR, label_pill, segmented, stamp};
use super::ui::*;
use super::*;
use crate::{
    gh::{IssueAction, IssueFilter},
    icons::Glyph,
    menu::{Entry, Item},
    util,
};
use gpui::prelude::FluentBuilder;
use gpui_component::{
    Disableable, Icon, IconName, Sizable,
    button::{Button, ButtonVariants},
    input::Input,
    tooltip::Tooltip,
};

const ISSUE_ROW: f32 = 56.;

/// Issue state as glyph, color, tint and title: open, closed, closed as not planned.
fn issue_glyph(
    colors: Palette,
    state: &str,
    reason: Option<&str>,
) -> (Icon, u32, u32, &'static str) {
    match (state, reason) {
        ("CLOSED", Some("NOT_PLANNED")) => (
            Glyph::CircleSlash.into(),
            colors.muted,
            colors.hover,
            "Closed as not planned",
        ),
        ("CLOSED", _) => (
            IconName::CircleCheck.into(),
            colors.purple,
            colors.purple_tint,
            "Closed",
        ),
        _ => (
            Glyph::CircleDot.into(),
            colors.green,
            colors.green_tint,
            "Open",
        ),
    }
}

impl Workspace {
    pub(super) fn issues_view(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        match self.current().and_then(|t| t.issue) {
            Some(n) => self.issue_page(n, window, cx),
            None => self.issue_list(cx),
        }
    }

    fn issue_list(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = palette(cx);
        let Some(t) = self.current() else {
            return div().into_any_element();
        };
        let Some(repo) = self.repo(&t.repo).cloned() else {
            return div().into_any_element();
        };
        let l = self
            .data(&t.repo)
            .and_then(|d| d.issues.get(&t.issue_filter));
        let q = t.search.read(cx).value().trim().to_lowercase();
        let new_issue = format!("https://github.com/{}/issues/new", repo.slug());
        let options = IssueFilter::ALL.map(|f| (f, f.title().to_string()));
        let header = row()
            .h(px(BAR + 6.))
            .flex_shrink_0()
            .px_2()
            .gap_2()
            .border_b_1()
            .border_color(rgb(colors.row_border))
            .child(segmented(
                colors,
                "issue-filter",
                &options,
                t.issue_filter,
                |s, f, _| {
                    if let Some(t) = s.current_mut() {
                        t.issue_filter = f;
                    }
                },
                cx,
            ))
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
                Button::new("new-issue")
                    .label("New issue")
                    .icon(Glyph::CircleDot)
                    .outline()
                    .small()
                    .tooltip("Open a new issue on GitHub")
                    .on_click(move |_, _, cx| cx.open_url(&new_issue)),
            );
        let body = match state(colors, l, "Loading issues…") {
            Err(p) => p.into_any_element(),
            Ok(()) => {
                let issues = l.and_then(|l| l.data.as_ref()).unwrap();
                let shown = issues
                    .iter()
                    .filter(|i| {
                        q.is_empty()
                            || i.title.to_lowercase().contains(&q)
                            || i.author.login.to_lowercase().contains(&q)
                            || i.number.to_string() == q.trim_start_matches('#')
                    })
                    .collect::<Vec<_>>();
                if shown.is_empty() {
                    let title = match t.issue_filter {
                        IssueFilter::All => "No issues".to_string(),
                        f => format!("No {} issues", f.title().to_lowercase()),
                    };
                    empty(
                        colors,
                        Glyph::CircleDot,
                        if q.is_empty() {
                            title
                        } else {
                            "No matches".to_string()
                        },
                        if q.is_empty() {
                            "Nothing to show here."
                        } else {
                            "Try another search."
                        },
                    )
                    .into_any_element()
                } else {
                    let shown = Arc::new(shown.into_iter().cloned().collect::<Vec<_>>());
                    uniform_list(
                        ("issue-list", t.id),
                        shown.len(),
                        cx.processor(move |s, range: std::ops::Range<usize>, _, cx| {
                            range.map(|i| s.issue_row(i, &shown[i], cx)).collect()
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

    /// One issue row; fixed height so the list can virtualize.
    fn issue_row(&self, i: usize, is: &gh::Issue, cx: &mut Context<Self>) -> Stateful<Div> {
        let colors = palette(cx);
        let (glyph, color, _, _) = issue_glyph(colors, &is.state, is.state_reason.as_deref());
        let n = is.number;
        let comments = is.comments.len();
        let created = util::parse_time(&is.created_at);
        let tip = format!("Updated {}", stamp(util::parse_time(&is.updated_at)));
        row()
            .id(("issue", n))
            .w_full()
            .h(px(ISSUE_ROW))
            .px_3()
            .py_2()
            .gap_3()
            .items_start()
            .cursor_pointer()
            .when(i > 0, |s| {
                s.border_t_1().border_color(rgb(colors.row_border))
            })
            .hover(|s| s.bg(rgb(colors.hover)))
            .child(
                div()
                    .pt(px(2.))
                    .child(icon(glyph).size(px(16.)).text_color(rgb(color))),
            )
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
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(is.title.clone()),
                            )
                            .children(is.labels.iter().take(3).map(|l| label_pill(colors, l))),
                    )
                    .child(
                        row()
                            .w_full()
                            .gap(px(6.))
                            .overflow_hidden()
                            .text_size(px(12.))
                            .text_color(rgb(colors.muted))
                            .child(format!("#{n}"))
                            .child(
                                div()
                                    .id(("issue-when", n))
                                    .child(format!(
                                        "opened {} by {}",
                                        self.when(created, cx),
                                        is.author.login
                                    ))
                                    .tooltip(move |w, cx| Tooltip::new(tip.clone()).build(w, cx)),
                            ),
                    ),
            )
            .child(
                row()
                    .flex_shrink_0()
                    .gap_3()
                    .pt(px(2.))
                    .when(comments > 0, |s| {
                        s.child(
                            row()
                                .gap(px(4.))
                                .text_size(px(12.))
                                .text_color(rgb(colors.muted))
                                .child(
                                    icon(Glyph::Message)
                                        .size(px(13.))
                                        .text_color(rgb(colors.muted)),
                                )
                                .child(comments.to_string()),
                        )
                    })
                    .child(self.avatar(Some(&is.author.login), &is.author.login, 20., cx)),
            )
            .on_click(cx.listener(move |s, _, _, cx| {
                if let Some(t) = s.current_mut() {
                    t.issue = Some(n);
                }
                cx.notify();
            }))
    }

    fn issue_page(&self, n: u64, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let colors = palette(cx);
        let Some(t) = self.current() else {
            return div().into_any_element();
        };
        let l = self.data(&t.repo).and_then(|d| d.issue.get(&n));
        let crumb = row()
            .h(px(BAR))
            .flex_shrink_0()
            .px_3()
            .gap_1()
            .text_size(px(13.))
            .child(
                row()
                    .id("crumb-issues")
                    .gap(px(4.))
                    .cursor_pointer()
                    .text_color(rgb(colors.blue))
                    .child(icon(IconName::ChevronLeft).size(px(13.)))
                    .child("Issues")
                    .on_click(cx.listener(|s, _, _, cx| {
                        if let Some(t) = s.current_mut() {
                            t.issue = None;
                        }
                        cx.notify();
                    })),
            )
            .child(div().text_color(rgb(colors.quiet)).child("/"))
            .child(div().text_color(rgb(colors.muted)).child(format!("#{n}")));
        let Some(issue) = l.and_then(|l| l.data.as_ref()) else {
            let body = match l.and_then(|l| l.error.clone()) {
                Some(e) => failed(colors, &e),
                None => loading(colors, "Loading issue…"),
            };
            return col()
                .size_full()
                .child(crumb)
                .child(body)
                .into_any_element();
        };
        let p = &issue.summary;
        let (glyph, fg, bg, status) = issue_glyph(colors, &p.state, p.state_reason.as_deref());
        let busy = self.busy.is_some();
        let url = issue.url.clone();
        let more = self.popover_button(
            "issue-more",
            icon_button("issue-more-trigger", IconName::Ellipsis, "More actions").disabled(busy),
            None,
            260.,
            move |_, _, _| {
                let (u1, u2) = (url.clone(), url.clone());
                vec![
                    Entry::Item(Item::new("Copy link").icon(IconName::Copy).on_click(
                        move |_, cx| cx.write_to_clipboard(ClipboardItem::new_string(u1.clone())),
                    )),
                    Entry::Item(
                        Item::new("Open on GitHub")
                            .icon(IconName::ExternalLink)
                            .on_click(move |_, cx| cx.open_url(&u2)),
                    ),
                ]
            },
            cx,
        );
        let assignees = issue
            .assignees
            .iter()
            .map(|a| a.login.clone())
            .collect::<Vec<_>>()
            .join(", ");
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
                            .child(
                                div()
                                    .text_color(rgb(colors.quiet))
                                    .font_weight(FontWeight::NORMAL)
                                    .child(format!("#{n}")),
                            ),
                    )
                    .child(row().gap_1().flex_shrink_0().child(more)),
            )
            .child(
                row()
                    .gap_2()
                    .flex_wrap()
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
                            .child(status),
                    )
                    .child(self.avatar(Some(&p.author.login), &p.author.login, 18., cx))
                    .child(
                        div()
                            .text_color(rgb(colors.text))
                            .child(p.author.login.clone()),
                    )
                    .child(div().text_color(rgb(colors.quiet)).child("•"))
                    .child(format!("opened {}", stamp(util::parse_time(&p.created_at))))
                    .when(!assignees.is_empty(), |s| {
                        s.child(div().text_color(rgb(colors.quiet)).child("•"))
                            .child(format!("assigned to {assignees}"))
                    })
                    .children(p.labels.iter().map(|l| label_pill(colors, l))),
            );
        let sub = row()
            .h(px(BAR))
            .flex_shrink_0()
            .px_3()
            .gap_2()
            .border_b_1()
            .border_color(rgb(colors.border))
            .child(segmented(
                colors,
                "issue-page",
                &[((), format!("Conversation  {}", p.comments.len()))],
                (),
                |_, _, _| {},
                cx,
            ));
        let body = self.issue_conversation(issue, window, cx);
        col()
            .size_full()
            .bg(rgb(colors.background))
            .child(crumb)
            .child(head)
            .child(sub)
            .child(div().flex_1().min_h_0().child(body))
            .into_any_element()
    }

    /// The body and comments on the rail, the issue's state, then the comment box.
    fn issue_conversation(
        &self,
        is: &gh::IssueDetail,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = palette(cx);
        let Some(t) = self.current() else {
            return div().into_any_element();
        };
        let p = &is.summary;
        let n = p.number;
        let open = p.state == "OPEN";
        let busy = self.busy.is_some();
        // Everything on the rail, oldest first: (time, avatar or badge, content).
        let mut items: Vec<(i64, AnyElement, Div)> = vec![];
        let opened = util::parse_time(&p.created_at);
        items.push((
            opened,
            self.avatar(Some(&p.author.login), &p.author.login, AVATAR, cx),
            self.message(
                format!("issue-{n}-body"),
                &p.author.login,
                Some(div().text_color(rgb(colors.muted)).child("opened this")),
                opened,
                &is.body,
                None,
                window,
                cx,
            ),
        ));
        for (i, c) in p.comments.iter().enumerate() {
            let at = util::parse_time(&c.created_at);
            items.push((
                at,
                self.avatar(Some(&c.author.login), &c.author.login, AVATAR, cx),
                self.message(
                    format!("issue-{n}-c{i}"),
                    &c.author.login,
                    None,
                    at,
                    &c.body,
                    None,
                    window,
                    cx,
                ),
            ));
        }
        items.sort_by_key(|(at, _, _)| *at);

        let (glyph, color, _, title) = issue_glyph(colors, &p.state, p.state_reason.as_deref());
        let status = row().min_h(px(AVATAR)).text_size(px(13.)).child(
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(color))
                .child(title),
        );
        items.push((i64::MAX, self.badge(glyph, color, cx), status));

        let composer = col().gap_2().child(Input::new(&t.issue_composer)).child(
            row()
                .gap_2()
                .flex_wrap()
                .child(label(colors, "Markdown · ⌘ Return to send"))
                .child(div().flex_1())
                .when(open, |r| {
                    r.child(
                        Button::new("close-completed")
                            .label("Close as completed")
                            .icon(IconName::CircleCheck)
                            .outline()
                            .small()
                            .disabled(busy)
                            .on_click(
                                cx.listener(move |s, _, w, cx| s.confirm_close(n, false, w, cx)),
                            ),
                    )
                    .child(
                        Button::new("close-not-planned")
                            .label("Close as not planned")
                            .icon(Glyph::CircleSlash)
                            .outline()
                            .small()
                            .disabled(busy)
                            .on_click(
                                cx.listener(move |s, _, w, cx| s.confirm_close(n, true, w, cx)),
                            ),
                    )
                })
                .when(!open, |r| {
                    r.child(
                        Button::new("reopen-issue")
                            .label("Reopen")
                            .icon(Glyph::RotateCcw)
                            .outline()
                            .small()
                            .disabled(busy)
                            .on_click(cx.listener(move |s, _, _, cx| {
                                s.issue_action(n, IssueAction::Reopen, cx)
                            })),
                    )
                })
                .child(
                    Button::new("send-comment")
                        .label("Comment")
                        .icon(Glyph::Message)
                        .primary()
                        .small()
                        .loading(self.busy_with("Commenting"))
                        .disabled(busy)
                        .on_click(cx.listener(|s, _, w, cx| s.submit(&Submit, w, cx))),
                ),
        );
        let me = self.user.data.as_ref().map(|u| u.login.clone());
        items.push((
            i64::MAX,
            self.avatar(me.as_deref(), "you", AVATAR, cx),
            composer,
        ));

        self.timeline("issue-conversation", items.into_iter().map(|(_, avatar, content)| (avatar, content)).collect(), cx)
    }

    /// Closing asks first, as closing a pull request does.
    fn confirm_close(
        &mut self,
        number: u64,
        not_planned: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let answer = if not_planned {
            "Close as not planned"
        } else {
            "Close as completed"
        };
        let prompt = window.prompt(
            PromptLevel::Warning,
            &format!("Close #{number}?"),
            Some("You can reopen it later."),
            &[answer, "Cancel"],
            cx,
        );
        cx.spawn(async move |this, cx| {
            if prompt.await == Ok(0) {
                let _ = this.update(cx, |s, cx| {
                    s.issue_action(number, IssueAction::Close { not_planned }, cx)
                });
            }
        })
        .detach();
    }

    /// Run an issue change through `mutate`, then refresh the list and the issue.
    pub(super) fn issue_action(
        &mut self,
        number: u64,
        action: IssueAction,
        cx: &mut Context<Self>,
    ) {
        let Some(t) = self.current() else { return };
        let Some(repo) = self.repo(&t.repo).cloned() else {
            return;
        };
        let (key, slug, tab) = (repo.key(), repo.slug(), t.id);
        let label = match &action {
            IssueAction::Comment(_) => format!("Commenting on #{number}"),
            IssueAction::Close { .. } => format!("Closing #{number}"),
            IssueAction::Reopen => format!("Reopening #{number}"),
        };
        let clears_composer = matches!(action, IssueAction::Comment(_));
        self.mutate(
            label,
            Report::Notice,
            move || gh::issue_action(&slug, number, &action),
            move |s, cx| {
                if let Some(d) = s.data.get_mut(&key) {
                    d.issue.remove(&number);
                    d.issues.clear();
                }
                if clears_composer {
                    s.pending_issue_clear = Some(tab);
                }
                s.ensure(false, cx);
            },
            cx,
        );
    }
}
