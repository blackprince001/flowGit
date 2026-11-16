//! Commits: the branch graph beside a virtualized history, with a commit's message,
//! changed files and diff in a lower pane.
use super::ui::*;
use super::*;
use crate::{
    gh::RefKind,
    graph::paint,
    icons::Glyph,
    menu::{Entry, Item},
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

const COMMIT_ROW: f32 = 30.;
const GRAPH_MAX: f32 = 300.;
const GRAPH_MIN: f32 = 58.;

fn ref_colors(colors: Palette, kind: RefKind) -> (u32, u32) {
    match kind {
        RefKind::Head => (colors.green, colors.green_tint),
        RefKind::Branch => (colors.blue, colors.blue_tint),
        RefKind::Remote => (colors.purple, colors.purple_tint),
        RefKind::Tag => (colors.amber, colors.amber_tint),
    }
}

impl Workspace {
    fn graph_ref_button(&self, local: bool, cx: &mut Context<Self>) -> Div {
        let colors = palette(cx);
        let Some(t) = self.current() else { return div() };
        if !local {
            let branch = self.branch_of(t).unwrap_or_else(|| "…".into());
            return self.branch_button("graph-branch", branch, cx);
        }
        let current = t.graph_ref.clone();
        self.popover_button(
            "graph-ref",
            button("graph-ref-trigger", current.clone().unwrap_or_else(|| "All branches".into()))
                .icon(icon(Glyph::Branch).text_color(rgb(colors.muted)))
                .dropdown_caret(true)
                .tooltip("Show every branch, or one"),
            Some("Find a branch…"),
            320.,
            |s, weak, _| {
                let Some(t) = s.current() else { return vec![] };
                let current = t.graph_ref.clone();
                let w = weak.clone();
                let mut out = vec![Entry::Item(
                    Item::new("All branches")
                        .icon(Glyph::Graph)
                        .checked(current.is_none())
                        .on_click(move |_, cx| {
                            let _ = w.update(cx, |s, cx| {
                                if let Some(t) = s.current_mut() {
                                    t.graph_ref = None;
                                }
                                cx.notify();
                            });
                        }),
                )];
                if let Some(branches) = s.data(&t.repo).and_then(|d| d.branches.data.as_ref()) {
                    out.push(Entry::Separator);
                    out.push(Entry::Label("Branches".into()));
                    for b in branches {
                        let name = b.name.clone();
                        let w = weak.clone();
                        out.push(Entry::Item(
                            Item::new(b.name.clone())
                                .icon(Glyph::Branch)
                                .checked(current.as_ref() == Some(&b.name))
                                .on_click(move |_, cx| {
                                    let name = name.clone();
                                    let _ = w.update(cx, |s, cx| {
                                        if let Some(t) = s.current_mut() {
                                            t.graph_ref = Some(name);
                                        }
                                        cx.notify();
                                    });
                                }),
                        ));
                    }
                }
                out
            },
            cx,
        )
    }

    pub(super) fn commits_view(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let colors = palette(cx);
        let Some(t) = self.current() else { return div().into_any_element() };
        let Some(repo) = self.repo(&t.repo).cloned() else { return div().into_any_element() };
        let local = repo.path.is_some();
        let graph_key = self.graph_key(t);
        let l = graph_key
            .as_ref()
            .and_then(|k| self.data(&t.repo).and_then(|d| d.commits.get(k)));
        let query = t.search.read(cx).value().trim().to_lowercase();
        let loaded = l.and_then(|l| l.data.clone());
        let matches = loaded.as_ref().map(|g| {
            if query.is_empty() {
                g.commits.len()
            } else {
                g.commits.iter().filter(|c| matches(c, &query)).count()
            }
        });
        let header = row()
            .h(px(BAR + 6.))
            .flex_shrink_0()
            .px_2()
            .gap_1()
            .border_b_1()
            .border_color(rgb(colors.row_border))
            .child(self.graph_ref_button(local, cx))
            .child(divider(colors))
            .child(
                div().w(px(280.)).child(
                    Input::new(&t.search)
                        .small()
                        .prefix(icon(IconName::Search).text_color(rgb(colors.muted)))
                        .cleanable(true),
                ),
            )
            .when_some(matches, |s, n| {
                s.child(label(
                    colors,
                    if query.is_empty() {
                        format!("{} commits", util::thousands(n as u64))
                    } else {
                        format!("{} matching", util::thousands(n as u64))
                    },
                ).px_2())
            })
            .child(div().flex_1())
            .when(!local, |s| {
                s.child(
                    row()
                        .gap(px(5.))
                        .mr_2()
                        .text_size(px(12.))
                        .text_color(rgb(colors.muted))
                        .child(icon(IconName::Info).size(px(12.)))
                        .child("One branch · link a local clone for every branch"),
                )
            })
            .when(local, |s| {
                s.child(
                    Button::new("git-fetch")
                        .label("Fetch")
                        .icon(Glyph::Fetch)
                        .outline()
                        .small()
                        .loading(self.busy_with("Fetching"))
                        .disabled(self.busy.is_some())
                        .tooltip("git fetch --all --prune")
                        .on_click(cx.listener(|s, _, _, cx| s.fetch_local(cx))),
                )
            });
        let list = match (state(colors, l, "Reading history…"), loaded) {
            (Ok(()), Some(g)) => self.history(t, g, query, cx),
            (Err(p), _) => p.into_any_element(),
            _ => loading(colors, "Reading history…").into_any_element(),
        };
        let body = match t.commit.clone() {
            Some(sha) => v_resizable(SharedString::from(format!("commit-split-{}", t.id)))
                .child(resizable_panel().size_range(px(120.)..Pixels::MAX).child(list))
                .child(
                    resizable_panel()
                        .size(px(380.))
                        .size_range(px(160.)..px(1600.))
                        .child(self.commit_detail(t, &sha, window, cx)),
                )
                .into_any_element(),
            None => list,
        };
        col()
            .size_full()
            .bg(rgb(colors.background))
            .child(header)
            .child(div().flex_1().min_h_0().child(body))
            .into_any_element()
    }

    fn history(&self, t: &Tab, g: Arc<CommitGraph>, query: String, cx: &mut Context<Self>) -> AnyElement {
        let colors = palette(cx);
        if g.commits.is_empty() {
            return empty(colors, Glyph::Commit, "No commits", "This branch has no history yet.").into_any_element();
        }
        // Wide enough for the "Graph" header even with a single lane.
        let graph_w = paint::width(g.graph.lanes).clamp(GRAPH_MIN, GRAPH_MAX);
        let tab = t.id;
        let selected = t.commit.clone();
        let mono = settings::mono(cx);
        let mono_size = px(settings::get(cx).mono_size as f32);
        let header = row()
            .h(px(26.))
            .flex_shrink_0()
            .border_b_1()
            .border_color(rgb(colors.row_border))
            .text_size(px(12.))
            .text_color(rgb(colors.muted))
            .child(div().w(px(graph_w)).flex_shrink_0().pl_3().child("Graph"))
            .child(div().flex_1().px_2().child("Description"))
            .child(div().w(px(170.)).flex_shrink_0().px_2().child("Author"))
            .child(div().w(px(84.)).flex_shrink_0().px_2().child("Commit"))
            .child(div().w(px(96.)).flex_shrink_0().px_2().child("Date"));
        let count = g.commits.len();
        let list = uniform_list(
            ("history", tab),
            count,
            cx.processor(move |s, range: std::ops::Range<usize>, _, cx| {
                let colors = palette(cx);
                range
                    .map(|i| {
                        let c = &g.commits[i];
                        let graph_row = g.graph.rows[i].clone();
                        let head = c.refs.iter().any(|r| r.kind == RefKind::Head);
                        let is_selected = selected.as_deref() == Some(c.sha.as_str());
                        let dim = !query.is_empty() && !matches(c, &query);
                        let sha = c.sha.clone();
                        let bg = if is_selected { colors.selected } else { colors.background };
                        let lanes = colors.lanes;
                        let graph = canvas(
                            |_, _, _| {},
                            move |bounds, _, window, _| {
                                paint::row(&graph_row, bounds, &|c| lanes[c % lanes.len()], bg, head, window)
                            },
                        )
                        .size_full();
                        let refs = c.refs.iter().take(4).map(|r| {
                            let (fg, bg) = ref_colors(colors, r.kind);
                            row()
                                .flex_shrink_0()
                                .max_w(px(200.))
                                .h(px(18.))
                                .px(px(5.))
                                .gap(px(3.))
                                .rounded(px(4.))
                                .bg(rgb(bg))
                                .text_color(rgb(fg))
                                .text_size(px(11.))
                                .font_weight(FontWeight::MEDIUM)
                                .when(r.kind == RefKind::Tag, |s| s.child(icon(Glyph::Tag).size(px(10.))))
                                .when(r.kind == RefKind::Head, |s| s.child(icon(IconName::Check).size(px(10.))))
                                .child(div().truncate().child(r.name.clone()))
                        });
                        row()
                            .id(("commit", i))
                            .h(px(COMMIT_ROW))
                            .w_full()
                            .cursor_pointer()
                            .when(is_selected, |s| s.bg(rgb(colors.selected)))
                            .when(!is_selected, |s| s.hover(|s| s.bg(rgb(colors.hover))))
                            .child(div().w(px(graph_w)).h_full().flex_shrink_0().overflow_hidden().child(graph))
                            .child(
                                row()
                                    .flex_1()
                                    .min_w_0()
                                    .px_2()
                                    .gap(px(6.))
                                    .when(dim, |s| s.opacity(0.3))
                                    .children(refs)
                                    .child(div().min_w_0().truncate().child(c.subject.clone())),
                            )
                            .child(
                                row()
                                    .w(px(170.))
                                    .flex_shrink_0()
                                    .px_2()
                                    .gap(px(6.))
                                    .when(dim, |s| s.opacity(0.3))
                                    .child(s.avatar(c.login.as_deref(), &c.author, 18., cx))
                                    .child(div().min_w_0().truncate().text_color(rgb(colors.muted)).child(c.author.clone())),
                            )
                            .child(
                                div()
                                    .w(px(84.))
                                    .flex_shrink_0()
                                    .px_2()
                                    .font_family(mono.clone())
                                    .text_size(mono_size)
                                    .text_color(rgb(colors.muted))
                                    .when(dim, |s| s.opacity(0.3))
                                    .child(util::short_sha(&c.sha).to_string()),
                            )
                            .child(
                                div()
                                    .w(px(96.))
                                    .flex_shrink_0()
                                    .px_2()
                                    .text_size(px(12.))
                                    .text_color(rgb(colors.muted))
                                    .when(dim, |s| s.opacity(0.3))
                                    .child(s.when(c.time, cx)),
                            )
                            .on_click(cx.listener(move |s, _, _, cx| {
                                if let Some(t) = s.tabs.iter_mut().find(|t| t.id == tab) {
                                    t.commit = if t.commit.as_ref() == Some(&sha) { None } else { Some(sha.clone()) };
                                }
                                cx.notify();
                            }))
                    })
                    .collect()
            }),
        )
        .track_scroll(t.list_scroll.clone())
        .flex_1()
        .min_h_0();
        col().size_full().child(header).child(list).into_any_element()
    }

    fn commit_detail(&self, t: &Tab, sha: &str, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let colors = palette(cx);
        let mono = settings::mono(cx);
        let commit = self
            .graph_key(t)
            .and_then(|k| self.data(&t.repo)?.commits.get(&k)?.data.clone())
            .and_then(|g| g.commits.iter().find(|c| c.sha == sha).cloned());
        let l = self.data(&t.repo).and_then(|d| d.commit.get(sha));
        let info = l.and_then(|l| l.data.clone());
        let message = info.as_ref().map(|i| i.message.clone());
        let title = message
            .as_deref()
            .and_then(|m| m.lines().next())
            .map(String::from)
            .or_else(|| commit.as_ref().map(|c| c.subject.clone()))
            .unwrap_or_default();
        let rest = message
            .as_deref()
            .map(|m| m.lines().skip(1).collect::<Vec<_>>().join("\n").trim().to_string())
            .unwrap_or_default();
        let full = sha.to_string();
        let tab = t.id;
        let parents = commit.as_ref().map(|c| c.parents.clone()).unwrap_or_default();
        let head = col()
            .flex_shrink_0()
            .px_3()
            .pt_2()
            .pb_2()
            .gap(px(6.))
            .border_b_1()
            .border_color(rgb(colors.row_border))
            .child(
                row()
                    .gap_2()
                    .child(icon(Glyph::Commit).text_color(rgb(colors.muted)))
                    .child(div().flex_1().min_w_0().truncate().font_weight(FontWeight::SEMIBOLD).child(title))
                    .child(
                        Button::new("copy-sha")
                            .label(util::short_sha(sha).to_string())
                            .icon(IconName::Copy)
                            .ghost()
                            .xsmall()
                            .tooltip("Copy full SHA")
                            .on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(full.clone()))),
                    )
                    .child(icon_button("close-commit", IconName::Close, "Close · esc").xsmall().on_click(cx.listener(move |s, _, _, cx| {
                        if let Some(t) = s.tabs.iter_mut().find(|t| t.id == tab) {
                            t.commit = None;
                        }
                        cx.notify();
                    }))),
            )
            .when_some(commit.as_ref(), |s, c| {
                s.child(
                    row()
                        .gap_2()
                        .text_size(px(12.))
                        .text_color(rgb(colors.muted))
                        .child(self.avatar(c.login.as_deref(), &c.author, 16., cx))
                        .child(div().text_color(rgb(colors.text)).child(c.author.clone()))
                        .child(format!("committed {}", self.when(c.time, cx)))
                        .when(!parents.is_empty(), |s| {
                            s.child(div().text_color(rgb(colors.quiet)).child("·")).child(if parents.len() > 1 { "parents" } else { "parent" })
                        })
                        .children(parents.iter().map(|p| {
                            let p2 = p.clone();
                            div()
                                .id(SharedString::from(format!("parent-{p}")))
                                .font_family(mono.clone())
                                .text_color(rgb(colors.blue))
                                .cursor_pointer()
                                .child(util::short_sha(p).to_string())
                                .tooltip(|w, cx| Tooltip::new("Select parent").build(w, cx))
                                .on_click(cx.listener(move |s, _, _, cx| {
                                    if let Some(t) = s.tabs.iter_mut().find(|t| t.id == tab) {
                                        t.commit = Some(p2.clone());
                                    }
                                    cx.notify();
                                }))
                        })),
                )
            })
            .when(!rest.is_empty(), |s| {
                s.child(
                    div()
                        .max_h(px(120.))
                        .id("commit-message")
                        .overflow_y_scroll()
                        .text_size(px(12.5))
                        .line_height(relative(1.5))
                        .text_color(rgb(colors.muted))
                        .whitespace_normal()
                        .child(rest),
                )
            });
        let body = match (l, info) {
            (_, Some(info)) => {
                let diff = info.diff.clone();
                h_resizable(SharedString::from(format!("commit-files-{}", t.id)))
                    .child(
                        resizable_panel()
                            .size(px(260.))
                            .size_range(px(180.)..px(500.))
                            .child(
                                col()
                                    .size_full()
                                    .border_r_1()
                                    .border_color(rgb(colors.row_border))
                                    .child(
                                        row()
                                            .h(px(28.))
                                            .px_3()
                                            .gap_2()
                                            .text_size(px(12.))
                                            .text_color(rgb(colors.muted))
                                            .child(format!("{} files", diff.files.len()))
                                            .child(div().flex_1())
                                            .child(super::diff::counts(colors, diff.additions, diff.deletions)),
                                    )
                                    .child(div().flex_1().min_h_0().child(self.diff_files(colors, &diff, t.diff_scroll.clone(), cx))),
                            ),
                    )
                    .child(resizable_panel().child(self.diff_list(("commit-diff", t.id), diff, t.diff_scroll.clone(), None, cx)))
                    .into_any_element()
            }
            (Some(l), _) if l.error().is_some() => failed(colors, l.error().unwrap_or_default()).into_any_element(),
            _ => loading(colors, "Loading changes…").into_any_element(),
        };
        col()
            .size_full()
            .bg(rgb(colors.background))
            .border_t_1()
            .border_color(rgb(colors.border))
            .child(head)
            .child(div().flex_1().min_h_0().child(body))
            .into_any_element()
    }
}

fn matches(c: &gh::Commit, q: &str) -> bool {
    c.subject.to_lowercase().contains(q)
        || c.author.to_lowercase().contains(q)
        || c.sha.starts_with(q)
        || c.refs.iter().any(|r| r.name.to_lowercase().contains(q))
}

