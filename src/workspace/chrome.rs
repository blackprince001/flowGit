//! The frame around repository views: title bar, repository sidebar, tab strip,
//! the repo's view switcher, its brief panel, status bar and overlays.
use super::ui::*;
use super::*;
use crate::{
    icons::Glyph,
    menu::{Entry, Item},
    util,
};
use gpui::prelude::FluentBuilder;
use gpui_component::{
    Disableable,
    Icon, IconName, Sizable, TitleBar,
    button::{Button, ButtonVariants},
    input::Input,
    resizable::*,
    tooltip::Tooltip,
};

pub fn view_glyph(view: View) -> Icon {
    match view {
        View::Code => Glyph::Code.into(),
        View::Commits => Glyph::Graph.into(),
        View::Issues => Glyph::CircleDot.into(),
        View::Pulls => Glyph::PullRequest.into(),
        View::Actions => Glyph::CirclePlay.into(),
        View::Releases => Glyph::Tag.into(),
        View::Changes => Glyph::FileDiff.into(),
    }
}

impl Workspace {
    fn title(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = palette(cx);
        let tab = self.current();
        let repo = tab.and_then(|t| self.repo(&t.repo));
        let private = tab
            .and_then(|t| self.data(&t.repo))
            .and_then(|d| d.info.data.as_ref())
            .is_some_and(|i| i.is_private);
        TitleBar::new()
            .bg(rgb(colors.chrome))
            .border_b_1()
            .border_color(rgb(colors.border))
            .child(
                row()
                    .w_full()
                    .min_w_0()
                    .pr_2()
                    .gap(px(6.))
                    .text_size(px(13.))
                    .child(icon(Glyph::Graph).text_color(rgb(colors.blue)))
                    .child(div().font_weight(FontWeight::MEDIUM).child("Gitflow"))
                    .when_some(repo, |s, r| {
                        s.child(div().text_color(rgb(colors.quiet)).child("/"))
                            .child(div().text_color(rgb(colors.muted)).child(r.owner.clone()))
                            .child(div().text_color(rgb(colors.quiet)).child("/"))
                            .child(div().child(r.name.clone()))
                            .when(private, |s| {
                                s.child(icon(Glyph::Lock).size(px(11.)).text_color(rgb(colors.muted)))
                            })
                            .when_some(tab, |s, t| {
                                s.child(div().text_color(rgb(colors.quiet)).child("·"))
                                    .child(div().text_color(rgb(colors.muted)).child(t.view.title()))
                            })
                    })
                    .child(div().flex_1())
                    .when_some(self.busy.clone(), |s, busy| {
                        s.child(
                            row()
                                .gap(px(6.))
                                .mr_2()
                                .text_color(rgb(colors.muted))
                                .child(spinner(colors))
                                .child(format!("{busy}…")),
                        )
                    })
                    // Cached data is on screen while the live copy loads.
                    .when(self.busy.is_none() && self.refreshing > 0, |s| {
                        s.child(
                            row()
                                .gap(px(6.))
                                .mr_2()
                                .text_size(px(12.))
                                .text_color(rgb(colors.muted))
                                .child(spinner(colors))
                                .child("Refreshing…"),
                        )
                    })
                    .when(tab.is_some(), |s| {
                        s.child(
                            icon_button("refresh", Glyph::Refresh, "Refresh · ⌘ R")
                                .on_click(cx.listener(|s, _, w, cx| s.refresh(&Refresh, w, cx))),
                        )
                        .child(
                            toggle(
                                colors,
                                "toggle-brief",
                                IconName::PanelRight,
                                "Toggle repository brief · ⌘ ⌥ B",
                                self.brief,
                            )
                            .on_click(cx.listener(|s, _, w, cx| s.toggle_brief(&ToggleBrief, w, cx))),
                        )
                    }),
            )
            .into_any_element()
    }

    fn add_menu(&self, id: &'static str, trigger: Button, cx: &mut Context<Self>) -> Div {
        self.popover_button(id, trigger, None, 250., |_, weak, _| {
            let (a, b) = (weak.clone(), weak);
            vec![
                Entry::Item(
                    Item::new("Add local repository…")
                        .icon(Glyph::FolderGit)
                        .detail("⌘O")
                        .on_click(move |w, cx| {
                            let _ = a.update(cx, |s, cx| s.add_local(&AddLocal, w, cx));
                        }),
                ),
                Entry::Item(
                    Item::new("Add from GitHub URL…")
                        .icon(Glyph::Link)
                        .detail("⇧⌘O")
                        .on_click(move |w, cx| {
                            let _ = b.update(cx, |s, cx| s.open_add_url(&AddUrl, w, cx));
                        }),
                ),
            ]
        }, cx)
    }

    fn repo_menu(&mut self, key: String, position: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let weak = cx.weak_entity();
        self.open_popover(
            "repo-menu",
            position,
            None,
            230.,
            Rc::new(move |cx| {
                let Some(w) = weak.upgrade() else { return vec![] };
                let s = w.read(cx);
                let Some(repo) = s.repo(&key).cloned() else { return vec![] };
                let url = format!("https://github.com/{}", repo.slug());
                let (k1, k2) = (key.clone(), key.clone());
                let (w1, w2, w3) = (weak.clone(), weak.clone(), weak.clone());
                let mut out = vec![
                    Entry::Item(Item::new("Open in new tab").icon(IconName::Plus).on_click(move |win, cx| {
                        let _ = w1.update(cx, |s, cx| s.open_repo_new_tab(&k1, win, cx));
                    })),
                    Entry::Item(Item::new("Open on GitHub").icon(IconName::ExternalLink).on_click({
                        let url = url.clone();
                        move |_, cx| cx.open_url(&url)
                    })),
                    Entry::Item(Item::new("Copy URL").icon(IconName::Copy).on_click(move |_, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(url.clone()))
                    })),
                ];
                if let Some(path) = repo.path.clone() {
                    out.push(Entry::Item(Item::new("Reveal in Finder").icon(IconName::FolderOpen).on_click(
                        move |_, cx| cx.reveal_path(&path),
                    )));
                } else {
                    let (w4, owner, name) = (weak.clone(), repo.owner.clone(), repo.name.clone());
                    out.push(Entry::Item(Item::new("Clone…").icon(Glyph::Fetch).detail("gh repo clone").on_click(
                        move |w, cx| {
                            let (owner, name) = (owner.clone(), name.clone());
                            let _ = w4.update(cx, |s, cx| s.clone_repo(owner, name, w, cx));
                        },
                    )));
                    out.push(Entry::Item(
                        Item::new("Link a local clone…").icon(Glyph::FolderGit).on_click(move |w, cx| {
                            let _ = w3.update(cx, |s, cx| s.add_local(&AddLocal, w, cx));
                        }),
                    ));
                }
                out.push(Entry::Separator);
                out.push(Entry::Item(Item::new("Remove from sidebar").icon(IconName::Delete).on_click(
                    move |_, cx| {
                        let _ = w2.update(cx, |s, cx| s.remove_repo(&k2, cx));
                    },
                )));
                out
            }),
            window,
            cx,
        );
    }

    fn sidebar_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = palette(cx);
        let q = self.repo_search.read(cx).value().trim().to_lowercase();
        let active = self.current().map(|t| t.repo.clone());
        let mut list = col().py_1();
        let shown = self
            .repos
            .iter()
            .filter(|r| q.is_empty() || r.slug().to_lowercase().contains(&q))
            .collect::<Vec<_>>();
        for r in &shown {
            let key = r.key();
            let is_active = active.as_ref() == Some(&key);
            let open = self.tabs.iter().any(|t| t.repo == key);
            let loading = self.data(&key).is_some_and(|d| d.info.loading && d.info.data.is_none());
            let running = self.running_runs(&key);
            let mut tip = match &r.path {
                Some(p) => format!("{} · {}", r.slug(), p.display()),
                None => r.slug(),
            };
            if running > 0 {
                tip.push_str(&format!("\n{running} workflow run{} in progress", if running == 1 { "" } else { "s" }));
            }
            let (k1, k2) = (key.clone(), key.clone());
            list = list.child(
                row()
                    .id(SharedString::from(format!("repo-{key}")))
                    .h(px(30.))
                    .mx_1()
                    .px_2()
                    .gap_2()
                    .rounded(px(5.))
                    .cursor_pointer()
                    .when(is_active, |s| s.bg(rgb(colors.selected)))
                    .when(!is_active, |s| s.hover(|s| s.bg(rgb(colors.hover))))
                    .child(
                        icon(if r.path.is_some() {
                            Icon::from(Glyph::FolderGit)
                        } else {
                            Icon::from(IconName::GitHub)
                        })
                        .size(px(14.))
                        .text_color(rgb(if is_active { colors.blue } else { colors.muted })),
                    )
                    .child(
                        row()
                            .flex_1()
                            .min_w_0()
                            .gap(px(6.))
                            .child(div().flex_shrink_0().max_w(relative(0.7)).truncate().child(r.name.clone()))
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(12.))
                                    .text_color(rgb(colors.quiet))
                                    .child(r.owner.clone()),
                            ),
                    )
                    .when(loading, |s| s.child(spinner(colors)))
                    .when(running > 0 && !loading, |s| {
                        s.child(
                            row()
                                .gap(px(4.))
                                .child(spinner(colors))
                                .child(div().text_size(px(11.)).text_color(rgb(colors.amber)).child(running.to_string())),
                        )
                    })
                    .when(open && !is_active && !loading && running == 0, |s| {
                        s.child(div().size(px(5.)).rounded_full().bg(rgb(colors.quiet)))
                    })
                    .tooltip(move |w, cx| Tooltip::new(tip.clone()).build(w, cx))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |s, e: &MouseDownEvent, w, cx| {
                            cx.stop_propagation();
                            s.repo_menu(k1.clone(), e.position, w, cx);
                        }),
                    )
                    .on_click(cx.listener(move |s, e: &ClickEvent, w, cx| {
                        if e.modifiers().platform {
                            s.open_repo_new_tab(&k2, w, cx)
                        } else {
                            s.open_repo(&k2, w, cx)
                        }
                    })),
            );
        }
        if self.repos.is_empty() {
            list = list.child(
                col()
                    .p_3()
                    .gap(px(6.))
                    .child(div().text_color(rgb(colors.muted)).child("No repositories yet"))
                    .child(label(colors, "Add a local clone or a GitHub URL to get started.").line_height(relative(1.45))),
            );
        } else if shown.is_empty() {
            list = list.child(div().p_3().text_color(rgb(colors.muted)).child("No matches"));
        }
        let user = self.user.data.as_ref();
        let account = match (&self.user, user) {
            (_, Some(u)) => u.login.clone(),
            (l, None) if l.loading => "Checking gh…".into(),
            _ => "Not signed in".into(),
        };
        col()
            .size_full()
            .bg(rgb(colors.chrome))
            .border_r_1()
            .border_color(rgb(colors.border))
            .child(
                row()
                    .h(px(BAR))
                    .flex_shrink_0()
                    .pl_3()
                    .pr_1()
                    .gap_2()
                    .border_b_1()
                    .border_color(rgb(colors.border))
                    .child(div().font_weight(FontWeight::MEDIUM).child("Repositories"))
                    .child(count_badge(colors, self.repos.len()))
                    .child(div().flex_1())
                    .child(self.add_menu(
                        "add-repo",
                        icon_button("add-repo-button", IconName::Plus, "Add repository"),
                        cx,
                    )),
            )
            .child(
                row().px_2().pt_2().pb(px(6.)).child(
                    Input::new(&self.repo_search)
                        .w_full()
                        .small()
                        .prefix(icon(IconName::Search).text_color(rgb(colors.muted)))
                        .cleanable(true),
                ),
            )
            .child(div().id("repo-list").flex_1().min_h_0().overflow_y_scroll().child(list))
            .child(
                row()
                    .h(px(BAR + 4.))
                    .flex_shrink_0()
                    .px_2()
                    .gap_2()
                    .border_t_1()
                    .border_color(rgb(colors.border))
                    .text_size(px(13.))
                    .child(self.avatar(user.map(|u| u.login.as_str()), &account, 20., cx))
                    .child(
                        col()
                            .flex_1()
                            .min_w_0()
                            .child(div().truncate().child(account))
                            .child(
                                label(
                                    colors,
                                    match &self.gh_version.data {
                                        Some(v) => format!("gh cli {v}"),
                                        None => "gh cli".into(),
                                    },
                                )
                                .font_family(settings::mono(cx))
                                .text_size(px(11.)),
                            ),
                    )
                    .when(self.user.error.is_some(), |s| {
                        s.child(
                            icon(IconName::TriangleAlert)
                                .text_color(rgb(colors.amber))
                                .size(px(13.)),
                        )
                    })
                    .child(
                        icon_button("open-prefs", IconName::Settings, "Preferences · ⌘ ,")
                            .on_click(cx.listener(|s, _, w, cx| s.open_prefs(&OpenPreferences, w, cx))),
                    ),
            )
            .into_any_element()
    }

    fn tab_menu(&mut self, index: usize, position: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        self.active = Some(index);
        let weak = cx.weak_entity();
        self.open_popover(
            "tab-menu",
            position,
            None,
            220.,
            Rc::new(move |cx| {
                let Some(w) = weak.upgrade() else { return vec![] };
                let many = w.read(cx).tabs.len() > 1;
                let (a, b, c) = (weak.clone(), weak.clone(), weak.clone());
                vec![
                    Entry::Item(Item::new("Duplicate tab").detail("⇧⌘D").on_click(move |win, cx| {
                        let _ = a.update(cx, |s, cx| s.duplicate(index, win, cx));
                    })),
                    Entry::Separator,
                    Entry::Item(Item::new("Close tab").detail("⌘W").on_click(move |_, cx| {
                        let _ = b.update(cx, |s, cx| s.close(index, cx));
                    })),
                    Entry::Item(Item::new("Close other tabs").disabled(!many).on_click(move |_, cx| {
                        let _ = c.update(cx, |s, cx| s.close_others(index, cx));
                    })),
                ]
            }),
            window,
            cx,
        );
    }

    fn tabs_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = palette(cx);
        let mut tabs = row().id("tabs-scroll").h_full().overflow_x_scroll();
        // Pinned Home tabs: always there, compact, never closed.
        for (k, page) in home::HomePage::ALL.into_iter().enumerate() {
            let active = self.active.is_none() && self.home_page == page;
            let count = self.home_count(page);
            let tip = match k {
                0 => format!("{} · ⌘ 0", page.title()),
                _ => page.title().to_string(),
            };
            tabs = tabs.child(
                row()
                    .id(("home-tab", k))
                    .h_full()
                    .px_3()
                    .gap(px(6.))
                    .flex_shrink_0()
                    .border_r_1()
                    .border_color(rgb(colors.border))
                    .cursor_pointer()
                    .map(|s| {
                        if active {
                            s.bg(rgb(colors.background)).text_color(rgb(colors.text))
                        } else {
                            s.border_b_1()
                                .text_color(rgb(colors.muted))
                                .hover(|s| s.text_color(rgb(colors.text)))
                        }
                    })
                    .child(icon(page.icon()).size(px(13.)).text_color(rgb(if active { colors.blue } else { colors.muted })))
                    .child(page.title())
                    .when(count > 0, |s| s.child(count_badge(colors, count)))
                    .when(self.popup.is_none(), |tab| tab.tooltip(move |w, cx| Tooltip::new(tip.clone()).build(w, cx)))
                    .on_click(cx.listener(move |s, _, w, cx| s.show_home_page(page, w, cx))),
            );
        }
        for (i, t) in self.tabs.iter().enumerate() {
            let active = self.active == Some(i);
            let group = SharedString::from(format!("tab-{}", t.id));
            let repo = self.repo(&t.repo);
            let name = repo.map(|r| r.name.clone()).unwrap_or_else(|| t.repo.clone());
            let tip = format!("{} · {}", repo.map(Repo::slug).unwrap_or_default(), t.view.title());
            let busy = self.data(&t.repo).is_some_and(|d| d.info.loading && d.info.data.is_none());
            tabs = tabs.child(
                row()
                    .id(("tab", t.id))
                    .group(group.clone())
                    .h_full()
                    .min_w(px(140.))
                    .max_w(px(250.))
                    .pl_3()
                    .pr_1()
                    .gap(px(6.))
                    .flex_shrink_0()
                    .border_r_1()
                    .border_color(rgb(colors.border))
                    .cursor_pointer()
                    .map(|s| {
                        if active {
                            s.bg(rgb(colors.background)).text_color(rgb(colors.text))
                        } else {
                            s.border_b_1()
                                .text_color(rgb(colors.muted))
                                .hover(|s| s.text_color(rgb(colors.text)))
                        }
                    })
                    .child(if busy {
                        spinner(colors).into_any_element()
                    } else {
                        icon(view_glyph(t.view))
                            .size(px(13.))
                            .text_color(rgb(if active { colors.blue } else { colors.muted }))
                            .into_any_element()
                    })
                    .child(
                        row()
                            .flex_1()
                            .min_w_0()
                            .gap(px(5.))
                            .child(div().flex_shrink_0().max_w(relative(0.75)).truncate().child(name))
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(12.))
                                    .text_color(rgb(colors.quiet))
                                    .child(t.view.title()),
                            ),
                    )
                    .child(
                        div()
                            .when(!active, |s| s.opacity(0.).group_hover(group, |s| s.opacity(1.)))
                            .child(
                                Button::new(("close", t.id))
                                    .icon(icon(IconName::Close))
                                    .ghost()
                                    .xsmall()
                                    .tooltip("Close tab · ⌘ W")
                                    .on_click(cx.listener(move |s, _, _, cx| {
                                        cx.stop_propagation();
                                        s.close(i, cx);
                                    })),
                            ),
                    )
                    .when(self.popup.is_none(), |tab| {
                        tab.tooltip(move |w, cx| Tooltip::new(tip.clone()).build(w, cx))
                    })
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |s, e: &MouseDownEvent, w, cx| {
                            cx.stop_propagation();
                            s.tab_menu(i, e.position, w, cx);
                        }),
                    )
                    .on_mouse_down(
                        MouseButton::Middle,
                        cx.listener(move |s, _, _, cx| s.close(i, cx)),
                    )
                    .on_click(cx.listener(move |s, _, w, cx| {
                        s.active = Some(i);
                        s.focus.focus(w);
                        cx.notify();
                    })),
            );
        }
        let new_tab = self.popover_button(
            "new-tab",
            icon_button("new-tab-button", IconName::Plus, "Open repository in a new tab"),
            Some("Open repository…"),
            320.,
            |s, weak, _| {
                s.repos
                    .iter()
                    .map(|r| {
                        let weak = weak.clone();
                        let key = r.key();
                        Entry::Item(
                            Item::new(r.slug())
                                .icon(if r.path.is_some() {
                                    Icon::from(Glyph::FolderGit)
                                } else {
                                    Icon::from(IconName::GitHub)
                                })
                                .on_click(move |w, cx| {
                                    let _ = weak.update(cx, |s, cx| s.open_repo_new_tab(&key, w, cx));
                                }),
                        )
                    })
                    .collect()
            },
            cx,
        );
        let edge = || {
            row()
                .h_full()
                .flex_shrink_0()
                .px_1()
                .gap(px(2.))
                .border_b_1()
                .border_color(rgb(colors.border))
        };
        let many = self.tabs.len() > 1;
        row()
            .h(px(BAR))
            .flex_shrink_0()
            .bg(rgb(colors.chrome))
            .child(
                edge()
                    .border_r_1()
                    .child(
                        icon_button("prev-tab", IconName::ArrowLeft, "Previous tab · ⌘ ⇧ [")
                            .disabled(!many)
                            .on_click(cx.listener(|s, _, _, cx| s.cycle(-1, cx))),
                    )
                    .child(
                        icon_button("next-tab", IconName::ArrowRight, "Next tab · ⌘ ⇧ ]")
                            .disabled(!many)
                            .on_click(cx.listener(|s, _, _, cx| s.cycle(1, cx))),
                    ),
            )
            .child(div().min_w_0().h_full().child(tabs))
            .child(div().flex_1().h_full().border_b_1().border_color(rgb(colors.border)))
            .child(
                edge()
                    .child(
                        icon_button("duplicate-tab", IconName::GalleryVerticalEnd, "Duplicate tab · ⇧ ⌘ D")
                            .disabled(self.active.is_none())
                            .on_click(cx.listener(|s, _, w, cx| s.duplicate_tab(&DuplicateTab, w, cx))),
                    )
                    .child(new_tab),
            )
            .into_any_element()
    }

    /// The repository's view switcher: Code, Commits, Pull requests, Actions, Releases.
    fn nav(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = palette(cx);
        let Some(t) = self.current() else {
            return div().into_any_element();
        };
        let data = self.data(&t.repo);
        let info = data.and_then(|d| d.info.data.as_ref());
        let running = data
            .and_then(|d| d.runs.get(&None))
            .and_then(|l| l.data.as_ref())
            .map(|r| r.iter().filter(|r| r.active()).count())
            .unwrap_or(0);
        let local = self.is_local(&t.repo);
        let mut nav = row().h_full().gap(px(2.));
        for (i, view) in View::ALL.into_iter().filter(|v| v.available(local)).enumerate() {
            let active = t.view == view;
            let badge: Option<AnyElement> = match view {
                View::Pulls => info
                    .map(|i| i.pull_requests.total_count)
                    .filter(|n| *n > 0)
                    .map(|n| count_badge(colors, n as usize).into_any_element()),
                View::Actions if running > 0 => Some(
                    row()
                        .gap(px(4.))
                        .child(spinner(colors))
                        .child(div().text_size(px(11.)).text_color(rgb(colors.amber)).child(running.to_string()))
                        .into_any_element(),
                ),
                View::Releases => data
                    .and_then(|d| d.releases.data.as_ref())
                    .filter(|r| !r.is_empty())
                    .map(|r| count_badge(colors, r.len()).into_any_element()),
                View::Changes => data
                    .and_then(|d| d.status.data.as_ref())
                    .map(|st| st.count())
                    .filter(|n| *n > 0)
                    .map(|n| count_badge(colors, n).into_any_element()),
                _ => None,
            };
            nav = nav.child(
                row()
                    .id(("nav", i))
                    .h_full()
                    .px(px(10.))
                    .gap(px(6.))
                    .cursor_pointer()
                    .border_b_2()
                    .border_color(if active { rgb(colors.blue).into() } else { transparent_black() })
                    .text_color(rgb(if active { colors.text } else { colors.muted }))
                    .when(active, |s| s.font_weight(FontWeight::MEDIUM))
                    .when(!active, |s| s.hover(|s| s.text_color(rgb(colors.text))))
                    .child(
                        icon(view_glyph(view))
                            .size(px(14.))
                            .text_color(rgb(if active { colors.blue } else { colors.muted })),
                    )
                    .child(view.title())
                    .children(badge)
                    .tooltip(move |w, cx| Tooltip::new(format!("{} · ⌘ {}", view.title(), i + 1)).build(w, cx))
                    .on_click(cx.listener(move |s, _, _, cx| s.set_view(view, cx))),
            );
        }
        let url = info.map(|i| i.url.clone());
        row()
            .h(px(BAR + 4.))
            .flex_shrink_0()
            .px_2()
            .gap_1()
            .bg(rgb(colors.background))
            .border_b_1()
            .border_color(rgb(colors.border))
            .child(nav)
            .child(div().flex_1())
            .when_some(url, |s, url| {
                let page = match t.view {
                    View::Code => String::new(),
                    View::Commits => "/commits".into(),
                    View::Issues => match t.issue {
                        Some(n) => format!("/issues/{n}"),
                        None => "/issues".into(),
                    },
                    View::Pulls => match t.pr {
                        Some(n) => format!("/pull/{n}"),
                        None => "/pulls".into(),
                    },
                    View::Actions => match t.run {
                        Some(r) => format!("/actions/runs/{r}"),
                        None => "/actions".into(),
                    },
                    View::Releases => "/releases".into(),
                    View::Changes => String::new(),
                };
                let full = format!("{url}{page}");
                s.child(
                    icon_button("open-github", IconName::ExternalLink, "Open this page on GitHub")
                        .on_click(move |_, _, cx| cx.open_url(&full)),
                )
            })
            .into_any_element()
    }

    fn brief_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = palette(cx);
        let Some(t) = self.current() else {
            return div().into_any_element();
        };
        let repo = self.repo(&t.repo).cloned();
        let data = self.data(&t.repo);
        let mut body = col().px_3().pb_4().gap_1();
        match data.map(|d| &d.info) {
            Some(Load { data: Some(info), .. }) => {
                body = body.child(
                    div()
                        .pt_3()
                        .line_height(relative(1.5))
                        .text_color(rgb(if info.description.is_some() { colors.text } else { colors.muted }))
                        .child(info.description.clone().filter(|d| !d.is_empty()).unwrap_or_else(|| "No description".into())),
                );
                if let Some(topics) = info.repository_topics.as_ref().filter(|t| !t.is_empty()) {
                    body = body.child(
                        div().flex().flex_wrap().gap_1().pt_2().children(
                            topics.iter().take(10).map(|t| pill(t.name.clone(), colors.blue, colors.blue_tint)),
                        ),
                    );
                }
                if let Some(home) = info.homepage_url.clone().filter(|h| !h.is_empty()) {
                    let label_text = home.trim_start_matches("https://").trim_start_matches("http://").to_string();
                    body = body.child(
                        row()
                            .id("homepage")
                            .pt_2()
                            .gap(px(6.))
                            .cursor_pointer()
                            .text_color(rgb(colors.blue))
                            .child(icon(Glyph::Link).size(px(13.)))
                            .child(div().truncate().child(label_text))
                            .on_click(move |_, _, cx| cx.open_url(&home)),
                    );
                }
                let stat = |glyph: Icon, n: u64, what: &'static str| {
                    row()
                        .gap(px(5.))
                        .text_size(px(12.))
                        .text_color(rgb(colors.muted))
                        .child(glyph.size(px(13.)))
                        .child(div().text_color(rgb(colors.text)).child(util::compact(n)))
                        .child(what)
                };
                let open_issues = Some(info.issues.total_count).map(|n| {
                    let show_open = cx.listener(|s, _, _, cx| {
                        if let Some(t) = s.current_mut() {
                            t.issue_filter = IssueFilter::Open;
                        }
                        s.set_view(View::Issues, cx);
                    });
                    stat(Glyph::CircleDot.into(), n, "open issues")
                        .id("brief-open-issues")
                        .cursor_pointer()
                        .hover(|s| s.text_color(rgb(colors.blue)))
                        .on_click(show_open)
                });
                body = body.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_x_3()
                        .gap_y_1()
                        .pt_3()
                        .child(stat(IconName::Star.into(), info.stargazer_count, "stars"))
                        .child(stat(Glyph::Fork.into(), info.fork_count, "forks"))
                        .child(stat(IconName::Eye.into(), info.watchers.total_count, "watching"))
                        .children(open_issues),
                );
                let fact = |glyph: Icon, text: String| {
                    row()
                        .h(px(24.))
                        .gap_2()
                        .text_size(px(12.5))
                        .child(glyph.size(px(13.)).text_color(rgb(colors.muted)))
                        .child(div().min_w_0().truncate().child(text))
                };
                let mut facts = col().pt_3();
                facts = facts.child(fact(Glyph::Branch.into(), info.default_branch()));
                if let Some(l) = &info.primary_language {
                    facts = facts.child(fact(Glyph::Code.into(), l.name.clone()));
                }
                if let Some(l) = info.license_info.as_ref().filter(|l| !l.name.is_empty()) {
                    facts = facts.child(fact(Glyph::Scale.into(), l.name.clone()));
                }
                let flags = [info.is_fork.then_some("Fork"), info.is_archived.then_some("Archived")]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>();
                if !flags.is_empty() {
                    facts = facts.child(fact(Glyph::Fork.into(), flags.join(" · ")));
                }
                if let Some(pushed) = info.pushed_at.as_deref() {
                    facts = facts.child(fact(Glyph::History.into(), format!("Pushed {}", self.when(util::parse_time(pushed), cx))));
                }
                if let Some(origin) = data.and_then(|d| d.origin.data.clone()) {
                    let shown = origin
                        .trim_start_matches("git@github.com:")
                        .trim_start_matches("https://github.com/")
                        .trim_end_matches(".git")
                        .to_string();
                    facts = facts.child(
                        fact(Glyph::Link.into(), format!("origin  {shown}"))
                            .id("local-origin")
                            .cursor_pointer()
                            .hover(|s| s.text_color(rgb(colors.blue)))
                            .tooltip(move |w, cx| Tooltip::new(format!("{origin} · click to copy")).build(w, cx))
                            .on_click({
                                let url = data.and_then(|d| d.origin.data.clone()).unwrap_or_default();
                                move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(url.clone()))
                            }),
                    );
                }
                if let Some(path) = repo.as_ref().and_then(|r| r.path.clone()) {
                    let shown = path.display().to_string().replace(&std::env::var("HOME").unwrap_or_default(), "~");
                    facts = facts.child(
                        fact(Glyph::FolderGit.into(), shown)
                            .id("local-path")
                            .cursor_pointer()
                            .hover(|s| s.text_color(rgb(colors.blue)))
                            .on_click(move |_, _, cx| cx.reveal_path(&path)),
                    );
                }
                body = body.child(facts);
                body = body.child(self.activity_strip(colors, data, cx));
            }
            Some(Load { error: Some(e), .. }) => {
                body = body.child(div().pt_3().text_color(rgb(colors.red)).text_size(px(12.)).child(e.clone()));
            }
            _ => body = body.child(row().pt_3().gap_2().text_color(rgb(colors.muted)).child(spinner(colors)).child("Loading…")),
        }

        // Releases
        let releases = data.and_then(|d| d.releases.data.as_ref());
        body = body.child(self.brief_header(colors, "Releases", releases.map(|r| r.len()), View::Releases, cx));
        match releases {
            Some(r) if r.is_empty() => body = body.child(label(colors, "No releases published")),
            Some(r) => {
                for (i, rel) in r.iter().filter(|r| !r.draft).take(3).enumerate() {
                    let latest = i == 0 && !rel.prerelease;
                    body = body.child(
                        row()
                            .id(("brief-release", i))
                            .py(px(5.))
                            .gap_2()
                            .items_start()
                            .cursor_pointer()
                            .rounded(px(5.))
                            .child(div().pt(px(2.)).child(icon(Glyph::Tag).size(px(13.)).text_color(rgb(if latest { colors.green } else { colors.muted }))))
                            .child(
                                col()
                                    .flex_1()
                                    .min_w_0()
                                    .child(
                                        row()
                                            .gap(px(6.))
                                            .child(div().min_w_0().truncate().child(rel.title()))
                                            .when(latest, |s| s.child(pill("Latest", colors.green, colors.green_tint)))
                                            .when(rel.prerelease, |s| s.child(pill("Pre-release", colors.amber, colors.amber_tint))),
                                    )
                                    .child(label(colors, format!("{} · {}", rel.tag_name, self.when(rel.time(), cx)))),
                            )
                            .on_click(cx.listener(|s, _, _, cx| s.set_view(View::Releases, cx))),
                    );
                }
            }
            None => {}
        }

        // Recent runs
        let runs = data.and_then(|d| d.runs.get(&None)).and_then(|l| l.data.as_ref());
        body = body.child(self.brief_header(colors, "Recent runs", None, View::Actions, cx));
        match runs {
            Some(r) if r.is_empty() => body = body.child(label(colors, "No workflow runs")),
            Some(r) => {
                for run in r.iter().take(5) {
                    let id = run.database_id;
                    body = body.child(
                        row()
                            .id(("brief-run", id))
                            .py(px(4.))
                            .gap_2()
                            .items_start()
                            .cursor_pointer()
                            .child(div().pt(px(2.)).child(run_icon(colors, &run.status, &run.conclusion)))
                            .child(
                                col()
                                    .flex_1()
                                    .min_w_0()
                                    .child(div().truncate().text_size(px(12.5)).child(run.display_title.clone()))
                                    .child(label(colors, format!("{} · {}", run.workflow_name, self.when(util::parse_time(&run.created_at), cx))).truncate()),
                            )
                            .on_click(cx.listener(move |s, _, _, cx| {
                                if let Some(t) = s.current_mut() {
                                    t.view = View::Actions;
                                    t.run = Some(id);
                                    t.job = None;
                                    t.log_open = false;
                                }
                                cx.notify();
                            })),
                    );
                }
            }
            None => {}
        }

        // Contributors
        let people = data.and_then(|d| d.contributors.data.as_ref());
        body = body.child(self.brief_header(colors, "Contributors", people.map(|p| p.len()), View::Commits, cx));
        if let Some(people) = people {
            let mut grid = div().flex().flex_wrap().gap(px(6.)).pt_1();
            for p in people.iter().take(24) {
                let tip = format!("{} · {} commits", p.login, util::thousands(p.contributions));
                let url = format!("https://github.com/{}", p.login);
                grid = grid.child(
                    div()
                        .id(SharedString::from(format!("contrib-{}", p.login)))
                        .cursor_pointer()
                        .child(self.avatar(Some(&p.login), &p.login, 26., cx))
                        .tooltip(move |w, cx| Tooltip::new(tip.clone()).build(w, cx))
                        .on_click(move |_, _, cx| cx.open_url(&url)),
                );
            }
            body = body.child(grid);
            if people.is_empty() {
                body = body.child(label(colors, "GitHub hasn’t computed contributors yet"));
            }
        }
        col()
            .size_full()
            .bg(rgb(colors.chrome))
            .border_l_1()
            .border_color(rgb(colors.border))
            .child(div().id("brief-scroll").flex_1().min_h_0().overflow_y_scroll().child(body))
            .into_any_element()
    }
    /// Commits per day for the last 30 days as a heat strip: grey for days without
    /// commits, deepening shades of green for busier ones. Hovering opens a popover
    /// docked under the strip; its arrow tracks the hovered day.
    fn activity_strip(&self, colors: Palette, data: Option<&RepoData>, cx: &mut Context<Self>) -> Div {
        let mono = settings::mono(cx);
        let l = data.map(|d| &d.activity);
        let Some(act) = l.and_then(|l| l.data.clone()) else {
            return row()
                .pt_4()
                .gap_2()
                .text_size(px(12.))
                .text_color(rgb(colors.muted))
                .when(l.and_then(|l| l.error.clone()).is_none(), |s| s.child(spinner(colors)).child("Reading activity…"));
        };
        let max = act.days.iter().map(|(_, c)| c.len()).max().unwrap_or(0).max(1) as f32;
        let total: usize = act.days.iter().map(|(_, c)| c.len()).sum();
        // Shades of green: faint for a quiet day, full green for the busiest.
        let low = mix(colors.green, colors.background, 0.75);
        let heat = move |n: usize| -> u32 {
            if n == 0 {
                return colors.hover;
            }
            mix(low, colors.green, (n as f32 / max).sqrt())
        };
        let days = act.days.len();
        let hovered = self.activity_hover.filter(|i| *i < days);
        let mut bars = row()
            .id("activity-bars")
            .h(px(26.))
            .gap(px(2.))
            // Gaps between bars keep the last day, so the popover doesn't flicker;
            // leaving the strip closes it.
            .on_hover(cx.listener(|s, over: &bool, _, cx| {
                if !*over && s.activity_hover.take().is_some() {
                    cx.notify();
                }
            }));
        for (i, (_, commits)) in act.days.iter().enumerate() {
            let lit = hovered == Some(i);
            bars = bars.child(
                div()
                    .id(("activity", i))
                    .flex_1()
                    .h_full()
                    .rounded(px(2.))
                    .bg(rgb(heat(commits.len())))
                    .when(lit, |s| s.border_1().border_color(rgb(colors.text)))
                    .on_hover(cx.listener(move |s, over: &bool, _, cx| {
                        if *over && s.activity_hover != Some(i) {
                            s.activity_hover = Some(i);
                            cx.notify();
                        }
                    })),
            );
        }
        // Docked under the strip at its full width; only the arrow follows the
        // hovered bar. Deferred so it paints over the sections below.
        let popover = hovered.map(|i| {
            let (ts, commits) = &act.days[i];
            let n = commits.len();
            let centre = (i as f32 + 0.5) / days as f32;
            let (fill, edge) = (colors.popover, colors.popover_border);
            let arrow = canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    let (o, sz) = (bounds.origin, bounds.size);
                    let apex = point(o.x + sz.width / 2., o.y);
                    let left = point(o.x, o.y + sz.height);
                    let right = point(o.x + sz.width, o.y + sz.height);
                    let mut f = PathBuilder::fill();
                    f.move_to(left);
                    f.line_to(apex);
                    f.line_to(right);
                    f.close();
                    if let Ok(path) = f.build() {
                        window.paint_path(path, rgb(fill));
                    }
                    let mut st = PathBuilder::stroke(px(1.));
                    st.move_to(left);
                    st.line_to(apex);
                    st.line_to(right);
                    if let Ok(path) = st.build() {
                        window.paint_path(path, rgb(edge));
                    }
                },
            )
            .absolute()
            .top(px(-7.))
            .left(relative(centre))
            .ml(px(-7.))
            .w(px(14.))
            .h(px(8.));
            deferred(
                col()
                    .absolute()
                    .top(px(34.))
                    .left_0()
                    .right_0()
                    .p(px(10.))
                    .gap(px(5.))
                    .rounded(px(8.))
                    .bg(rgb(colors.popover))
                    .border_1()
                    .border_color(rgb(colors.popover_border))
                    .shadow_lg()
                    .occlude()
                    .child(arrow)
                    .child(
                        row()
                            .gap(px(6.))
                            .child(div().font_weight(FontWeight::SEMIBOLD).child(day(*ts)))
                            .child(div().text_color(rgb(colors.quiet)).child("•"))
                            .child(div().text_color(rgb(colors.muted)).child(format!("{n} commit{}", if n == 1 { "" } else { "s" }))),
                    )
                    .when(n == 0, |s| s.child(div().text_size(px(12.)).text_color(rgb(colors.muted)).child("No commits this day")))
                    .children(commits.iter().take(3).map(|c| {
                        row()
                            .w_full()
                            .gap_2()
                            .text_size(px(12.))
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .font_family(mono.clone())
                                    .text_size(px(11.5))
                                    .text_color(rgb(colors.muted))
                                    .child(util::short_sha(&c.sha).to_string()),
                            )
                            .child(div().flex_1().min_w_0().truncate().child(c.subject.clone()))
                    }))
                    .when(n > 3, |s| s.child(div().text_size(px(11.5)).text_color(rgb(colors.quiet)).child(format!("and {} more", n - 3)))),
            )
            .with_priority(1)
        });
        col()
            .pt_4()
            .gap(px(6.))
            .child(
                row()
                    .text_size(px(12.))
                    .text_color(rgb(colors.muted))
                    .font_weight(FontWeight::MEDIUM)
                    .child("Last 30 days")
                    .child(div().flex_1())
                    .child(format!("{} commit{}", util::thousands(total as u64), if total == 1 { "" } else { "s" })),
            )
            .child(div().relative().child(bars).children(popover))
            .child(
                row()
                    .justify_between()
                    .text_size(px(11.))
                    .text_color(rgb(colors.quiet))
                    .child(act.days.first().map(|(t, _)| day(*t)).unwrap_or_default())
                    .child("Today"),
            )
    }
    fn brief_header(&self, colors: Palette, title: &'static str, count: Option<usize>, view: View, cx: &mut Context<Self>) -> Stateful<Div> {
        row()
            .id(SharedString::from(format!("brief-{title}")))
            .pt_4()
            .pb_1()
            .gap_2()
            .text_size(px(12.))
            .text_color(rgb(colors.muted))
            .font_weight(FontWeight::MEDIUM)
            .cursor_pointer()
            .hover(|s| s.text_color(rgb(colors.text)))
            .child(title)
            .when_some(count.filter(|c| *c > 0), |s, c| s.child(count_badge(colors, c)))
            .child(div().flex_1().h(px(1.)).bg(rgb(colors.border)))
            .child(icon(IconName::ChevronRight).size(px(12.)))
            .on_click(cx.listener(move |s, _, _, cx| s.set_view(view, cx)))
    }

    fn status(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = palette(cx);
        let tab = self.current();
        let repo = tab.and_then(|t| self.repo(&t.repo));
        let branch = tab.and_then(|t| match t.view {
            View::Commits if repo.is_some_and(|r| r.path.is_some()) => {
                Some(t.graph_ref.clone().unwrap_or_else(|| "all branches".into()))
            }
            _ => self.branch_of(t),
        });
        let signed_in = self.user.data.is_some();
        row()
            .h(px(28.))
            .flex_shrink_0()
            .px_1()
            .gap(px(2.))
            .bg(rgb(colors.chrome))
            .border_t_1()
            .border_color(rgb(colors.border))
            .text_size(px(12.))
            .text_color(rgb(colors.muted))
            .child(
                toggle(colors, "sidebar-toggle", IconName::PanelLeft, "Toggle sidebar · ⌘ B", self.sidebar)
                    .on_click(cx.listener(|s, _, w, cx| s.toggle_sidebar(&ToggleSidebar, w, cx))),
            )
            .child(divider(colors))
            .child(
                row()
                    .gap(px(6.))
                    .px_1()
                    .child(div().size(px(6.)).rounded_full().bg(rgb(if signed_in {
                        colors.green
                    } else if self.user.loading {
                        colors.amber
                    } else {
                        colors.red
                    })))
                    .child(match &self.user.data {
                        Some(u) => u.login.clone(),
                        None if self.user.loading => "Connecting to gh…".into(),
                        None => "gh not signed in".into(),
                    }),
            )
            .when_some(repo, |s, r| {
                s.child(div().text_color(rgb(colors.quiet)).child("·"))
                    .child(div().px_1().child(r.slug()))
            })
            .when_some(branch, |s, b| {
                s.child(
                    row()
                        .gap(px(4.))
                        .px_1()
                        .child(icon(Glyph::Branch).size(px(12.)))
                        .child(b),
                )
            })
            .child(div().flex_1())
            .when_some(self.busy.clone(), |s, b| {
                s.child(row().gap(px(6.)).px_2().child(spinner(colors)).child(b))
            })
            .into_any_element()
    }

    fn add_url_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = palette(cx);
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
                    .w(px(520.))
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
                            // GPUI does not clip children to a parent's radius; match it inside the 1px border.
                            .rounded_t(px(9.))
                            .border_b_1()
                            .border_color(rgb(colors.border))
                            .child(icon(Glyph::Link).text_color(rgb(colors.muted)))
                            .child(div().flex_1().font_weight(FontWeight::MEDIUM).child("Add repository from GitHub"))
                            .child(label(colors, "esc")),
                    )
                    .child(
                        col()
                            .p_4()
                            .gap_3()
                            .child(Input::new(&self.url_input).prefix(icon(IconName::GitHub).text_color(rgb(colors.muted))))
                            .child(
                                row()
                                    .gap_2()
                                    .child(
                                        label(colors, "Clone downloads a local copy for the full branch graph.")
                                            .flex_1()
                                            .min_w_0()
                                            .line_height(relative(1.4)),
                                    )
                                    .child(
                                        Button::new("add-url-clone")
                                            .label("Clone…")
                                            .icon(Glyph::Fetch)
                                            .outline()
                                            .small()
                                            .disabled(self.overlay == Overlay::AddUrl { resolving: true } || self.busy.is_some())
                                            .tooltip("gh repo clone into a folder you choose")
                                            .on_click(cx.listener(|s, _, w, cx| s.submit_url(true, w, cx))),
                                    )
                                    .child(
                                        Button::new("add-url-submit")
                                            .label("Add")
                                            .primary()
                                            .small()
                                            .loading(self.overlay == Overlay::AddUrl { resolving: true })
                                            .disabled(self.overlay == Overlay::AddUrl { resolving: true })
                                            .on_click(cx.listener(|s, _, w, cx| s.submit_url(false, w, cx))),
                                    ),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn notice_view(&self, notice: &Notice, cx: &mut Context<Self>) -> AnyElement {
        let colors = palette(cx);
        let (fg, bg) = if notice.ok {
            (colors.green, colors.green_tint)
        } else {
            (colors.amber, colors.amber_tint)
        };
        row()
            .id("notice")
            .occlude()
            .absolute()
            .bottom(px(40.))
            .right(px(16.))
            .w(px(440.))
            .pl_3()
            .pr_1()
            .py_2()
            .gap_2()
            .items_start()
            .bg(rgb(bg))
            .border_1()
            .border_color(rgb(colors.border))
            .rounded(px(8.))
            .shadow_md()
            .child(
                div().pt(px(2.)).child(
                    icon(if notice.ok { IconName::CircleCheck } else { IconName::TriangleAlert }).text_color(rgb(fg)),
                ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .pt(px(1.))
                    .text_size(px(13.))
                    .line_height(relative(1.45))
                    .child(notice.text.clone()),
            )
            .child(
                icon_button("dismiss-notice", IconName::Close, "Dismiss")
                    .xsmall()
                    .on_click(cx.listener(|s, _, _, cx| {
                        s.notice = None;
                        cx.notify();
                    })),
            )
            .into_any_element()
    }

    fn content(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(t) = self.current() else {
            return div().into_any_element();
        };
        match t.view {
            View::Code => self.code_view(window, cx),
            View::Commits => self.commits_view(window, cx),
            View::Issues => self.issues_view(window, cx),
            View::Pulls => self.pulls_view(window, cx),
            View::Actions => self.actions_view(window, cx),
            View::Releases => self.releases_view(window, cx),
            View::Changes => self.changes_view(window, cx),
        }
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure(false, cx);
        if let Some(id) = self.pending_clear.take()
            && let Some(t) = self.tabs.iter().find(|t| t.id == id)
        {
            t.composer.update(cx, |s, cx| s.set_value("", window, cx));
        }
        if let Some(id) = self.pending_line_clear.take()
            && let Some(t) = self.tabs.iter().find(|t| t.id == id)
        {
            t.line_composer.update(cx, |s, cx| s.set_value("", window, cx));
        }
        if std::mem::take(&mut self.pending_review_clear) {
            self.review_body.update(cx, |s, cx| s.set_value("", window, cx));
        }
        self.clear_committed_message(window, cx);
        let colors = palette(cx);
        let center = if self.current().is_some() {
            let view = self.content(window, cx);
            let body = if self.brief {
                h_resizable("brief-panels")
                    .with_state(&self.brief_panels)
                    .child(resizable_panel().size_range(px(420.)..Pixels::MAX).child(view))
                    .child(
                        resizable_panel()
                            .size(px(300.))
                            .size_range(px(240.)..px(480.))
                            .child(self.brief_view(cx)),
                    )
                    .into_any_element()
            } else {
                view
            };
            col()
                .size_full()
                .min_w_0()
                .child(self.tabs_view(cx))
                .child(self.nav(cx))
                .child(div().flex_1().min_h_0().child(body))
                .into_any_element()
        } else {
            col()
                .size_full()
                .min_w_0()
                .child(self.tabs_view(cx))
                .child(div().flex_1().min_h_0().child(self.home_view(cx)))
                .into_any_element()
        };
        let main = if self.sidebar {
            h_resizable("sidebar-panels")
                .with_state(&self.sidebar_panels)
                .child(
                    resizable_panel()
                        .size(px(250.))
                        .size_range(px(190.)..px(400.))
                        .child(self.sidebar_view(cx)),
                )
                .child(resizable_panel().size_range(px(480.)..Pixels::MAX).child(center))
                .into_any_element()
        } else {
            center
        };
        let mut root = col()
            .relative()
            .size_full()
            .bg(rgb(colors.background))
            .text_color(rgb(colors.text))
            .font_family(".SystemUIFont")
            .text_size(px(13.))
            .track_focus(&self.focus)
            // Diff stepping keys (j k n p) apply while a diff is on screen.
            .when(self.overlay == Overlay::None && self.active_diff().is_some(), |d| d.key_context("DiffNav"))
            .on_action(cx.listener(Self::refresh))
            .on_action(cx.listener(Self::open_palette))
            .on_action(cx.listener(Self::find_file))
            .on_action(cx.listener(Self::toggle_sidebar))
            .on_action(cx.listener(Self::toggle_brief))
            .on_action(cx.listener(Self::close_tab))
            .on_action(cx.listener(Self::duplicate_tab))
            .on_action(cx.listener(|s, _: &NextTab, _, cx| s.cycle(1, cx)))
            .on_action(cx.listener(|s, _: &PreviousTab, _, cx| s.cycle(-1, cx)))
            .on_action(cx.listener(Self::open_prefs))
            .on_action(cx.listener(Self::show_home))
            .on_action(cx.listener(Self::add_local))
            .on_action(cx.listener(Self::open_add_url))
            .on_action(cx.listener(Self::submit))
            .on_action(cx.listener(Self::back))
            .on_action(cx.listener(Self::new_pull_request))
            .on_action(cx.listener(Self::open_finish_review))
            .on_action(cx.listener(Self::next_file))
            .on_action(cx.listener(Self::previous_file))
            .on_action(cx.listener(Self::next_hunk))
            .on_action(cx.listener(Self::previous_hunk))
            .on_action(cx.listener(|s, _: &ShowCode, _, cx| s.set_view(View::Code, cx)))
            .on_action(cx.listener(|s, _: &ShowCommits, _, cx| s.set_view(View::Commits, cx)))
            .on_action(cx.listener(|s, _: &ShowIssues, _, cx| s.set_view(View::Issues, cx)))
            .on_action(cx.listener(|s, _: &ShowPulls, _, cx| s.set_view(View::Pulls, cx)))
            .on_action(cx.listener(|s, _: &ShowActions, _, cx| s.set_view(View::Actions, cx)))
            .on_action(cx.listener(|s, _: &ShowReleases, _, cx| s.set_view(View::Releases, cx)))
            .on_action(cx.listener(|s, _: &DraftRelease, window, cx| {
                s.set_view(View::Releases, cx);
                s.open_release_draft(window, cx);
            }))
            .on_action(cx.listener(|s, _: &ShowChanges, _, cx| s.set_view(View::Changes, cx)))
            .on_key_down(cx.listener(|s, e: &KeyDownEvent, w, cx| {
                if e.keystroke.key != "escape" {
                    return;
                }
                if s.popup.is_some() || s.overlay != Overlay::None {
                    s.popup = None;
                    s.overlay = Overlay::None;
                    cx.notify();
                } else if s.focus.is_focused(w) {
                    s.back(&Back, w, cx);
                }
            }))
            .child(self.title(cx))
            .child(div().flex_1().min_h_0().child(main))
            .child(self.status(cx));
        match self.overlay {
            Overlay::None => {}
            Overlay::Prefs => root = root.child(self.preferences_view(cx)),
            Overlay::AddUrl { .. } => root = root.child(self.add_url_view(cx)),
            Overlay::FinishReview { .. } => root = root.child(self.finish_review_view(cx)),
        }
        if let Some(notice) = &self.notice {
            root = root.child(self.notice_view(notice, cx));
        }
        if let Some(menu) = &self.popup {
            root = root.child(
                deferred(
                    anchored()
                        .position(self.popup_position)
                        .snap_to_window_with_margin(px(8.))
                        .child(menu.clone()),
                )
                .with_priority(2),
            );
        }
        root
    }
}
