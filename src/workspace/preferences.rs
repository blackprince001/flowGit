//! Preferences, laid out like Zed's settings (and CloudWatch v2's): a searchable
//! section list on the left, titled rows with descriptions and controls on the right.
use super::ui::*;
use super::*;
use crate::{
    icons::Glyph,
    menu::{Entry, Item},
    settings::{COMMIT_LIMITS, MergeMethod, REFRESH_CHOICES, ThemePref},
};
use gpui::prelude::FluentBuilder;
use gpui_component::{
    Disableable,
    Icon, IconName, Sizable,
    button::{Button, ButtonVariants},
    input::Input,
    switch::Switch,
};

const GENERAL_WORDS: &str = "general default view tab open commits graph limit history refresh auto poll interval dates relative notifications alerts desktop review ci";
const GITHUB_WORDS: &str = "github gh cli account login auth path executable merge squash rebase delete branch avatars";
const APPEARANCE_WORDS: &str = "appearance theme light dark system font mono monospace size interface";

fn setting(colors: Palette, title: &str, description: &str, control: impl IntoElement) -> Div {
    row()
        .gap_6()
        .py(px(14.))
        .border_b_1()
        .border_color(rgb(colors.row_border))
        .child(
            col()
                .flex_1()
                .min_w_0()
                .gap(px(3.))
                .child(div().text_size(px(14.)).child(title.to_string()))
                .when(!description.is_empty(), |s| {
                    s.child(
                        div()
                            .text_size(px(13.))
                            .line_height(relative(1.45))
                            .text_color(rgb(colors.muted))
                            .child(description.to_string()),
                    )
                }),
        )
        .child(div().flex_shrink_0().child(control))
}
fn page_header(colors: Palette, mono: SharedString, title: &str, subtitle: &str) -> Div {
    col()
        .gap(px(6.))
        .pb_3()
        .border_b_1()
        .border_color(rgb(colors.row_border))
        .child(div().text_size(px(20.)).font_weight(FontWeight::SEMIBOLD).child(title.to_string()))
        .child(div().font_family(mono).text_size(px(12.)).text_color(rgb(colors.muted)).child(subtitle.to_string()))
}
fn group(colors: Palette, mono: SharedString, title: &str) -> Div {
    div()
        .pt_6()
        .pb_1()
        .font_family(mono)
        .text_size(px(12.))
        .text_color(rgb(colors.muted))
        .border_b_1()
        .border_color(rgb(colors.row_border))
        .child(title.to_string())
}
fn picker(colors: Palette, id: &'static str, text: String) -> Button {
    Button::new(id).label(text).outline().small().child(
        Icon::new(IconName::ChevronsUpDown)
            .size(px(13.))
            .text_color(rgb(colors.muted)),
    )
}

impl Workspace {
    fn close_prefs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.overlay = Overlay::None;
        self.popup = None;
        self.focus.focus(window);
        cx.notify();
    }
    fn prefs_matches(&self, cx: &App, words: &str) -> bool {
        let q = self.prefs_search.read(cx).value().trim().to_lowercase();
        q.is_empty() || q.split_whitespace().all(|w| words.contains(w))
    }
    fn prefs_nav(&self, colors: Palette, cx: &mut Context<Self>) -> Div {
        let mut nav = col().gap(px(2.));
        let pages = [
            (PrefsPage::General, "General", Icon::from(IconName::Settings2), GENERAL_WORDS),
            (PrefsPage::GitHub, "GitHub", Icon::from(IconName::GitHub), GITHUB_WORDS),
            (PrefsPage::Appearance, "Appearance", Icon::from(IconName::Palette), APPEARANCE_WORDS),
        ];
        let mut any = false;
        for (page, title, glyph, words) in pages {
            if !self.prefs_matches(cx, words) {
                continue;
            }
            any = true;
            let active = self.prefs_page == page;
            nav = nav.child(
                row()
                    .id(title)
                    .h(px(30.))
                    .px_2()
                    .gap_2()
                    .rounded(px(6.))
                    .cursor_pointer()
                    .text_size(px(14.))
                    .map(|s| if active { s.bg(rgb(colors.hover)) } else { s.hover(|s| s.bg(rgb(colors.hover))) })
                    .child(glyph.size(px(15.)).text_color(rgb(if active { colors.blue } else { colors.muted })))
                    .child(title)
                    .on_click(cx.listener(move |s, _, _, cx| {
                        s.prefs_page = page;
                        cx.notify();
                    })),
            );
        }
        if !any {
            nav = nav.child(div().px_2().py_1().text_color(rgb(colors.muted)).child("No matching settings"));
        }
        col()
            .w(px(240.))
            .h_full()
            .flex_shrink_0()
            .rounded_l(px(12.))
            .p_3()
            .gap_3()
            .bg(rgb(colors.chrome))
            .border_r_1()
            .border_color(rgb(colors.border))
            .child(
                Input::new(&self.prefs_search)
                    .small()
                    .prefix(Icon::new(IconName::Search).size(px(14.)).text_color(rgb(colors.muted)))
                    .cleanable(true),
            )
            .child(div().id("prefs-nav").flex_1().min_h_0().overflow_y_scroll().child(nav))
            .child(
                row()
                    .pt_2()
                    .gap_2()
                    .border_t_1()
                    .border_color(rgb(colors.border))
                    .text_size(px(12.))
                    .text_color(rgb(colors.muted))
                    .child(kbd("escape"))
                    .child("Close settings"),
            )
    }

    fn general_page(&self, colors: Palette, mono: SharedString, cx: &mut Context<Self>) -> Div {
        let prefs = settings::get(cx).clone();
        let view = self.popover_button(
            "pref-view",
            picker(colors, "pref-view-trigger", prefs.default_view.title().into()),
            None,
            220.,
            |_, weak, cx| {
                let current = settings::get(cx).default_view;
                // The default must exist for every repository, remote ones included.
                View::ALL
                    .into_iter()
                    .filter(|v| v.available(false))
                    .map(|v| {
                        let weak = weak.clone();
                        Entry::Item(Item::new(v.title()).icon(super::chrome::view_glyph(v)).checked(v == current).on_click(move |_, cx| {
                            let _ = weak.update(cx, |s, cx| s.update_settings(cx, |p| p.default_view = v));
                        }))
                    })
                    .collect()
            },
            cx,
        );
        let limit = self.popover_button(
            "pref-limit",
            picker(colors, "pref-limit-trigger", format!("{} commits", prefs.commit_limit)),
            None,
            200.,
            |_, weak, cx| {
                let current = settings::get(cx).commit_limit;
                COMMIT_LIMITS
                    .iter()
                    .map(|&n| {
                        let weak = weak.clone();
                        Entry::Item(Item::new(format!("{n} commits")).checked(n == current).on_click(move |_, cx| {
                            let _ = weak.update(cx, |s, cx| s.update_settings(cx, |p| p.commit_limit = n));
                        }))
                    })
                    .collect()
            },
            cx,
        );
        let refresh = self.popover_button(
            "pref-refresh",
            picker(
                colors,
                "pref-refresh-trigger",
                REFRESH_CHOICES.iter().find(|(s, _)| *s == prefs.refresh_secs).map(|(_, l)| format!("Every {}", l)).unwrap_or_else(|| format!("{}s", prefs.refresh_secs)),
            )
            .disabled(!prefs.auto_refresh),
            None,
            200.,
            |_, weak, cx| {
                let current = settings::get(cx).refresh_secs;
                REFRESH_CHOICES
                    .iter()
                    .map(|&(n, title)| {
                        let weak = weak.clone();
                        Entry::Item(Item::new(title).checked(n == current).on_click(move |_, cx| {
                            let _ = weak.update(cx, |s, cx| s.update_settings(cx, |p| p.refresh_secs = n));
                        }))
                    })
                    .collect()
            },
            cx,
        );
        let weak = cx.weak_entity();
        let alerts_switch = {
            let weak = cx.weak_entity();
            Switch::new("pref-alerts").checked(prefs.desktop_alerts).on_click(move |on: &bool, _, cx| {
                let on = *on;
                let _ = weak.update(cx, |s, cx| s.update_settings(cx, |p| p.desktop_alerts = on));
            })
        };
        let refresh_switch = {
            let weak = cx.weak_entity();
            Switch::new("pref-auto-refresh").checked(prefs.auto_refresh).on_click(move |on: &bool, _, cx| {
                let on = *on;
                let _ = weak.update(cx, |s, cx| s.update_settings(cx, |p| p.auto_refresh = on));
            })
        };
        col()
            .child(page_header(colors, mono.clone(), "General", "Tabs and data"))
            .child(group(colors, mono.clone(), "Tabs"))
            .child(setting(colors, "Open repositories on", "The view a new repository tab starts with. Duplicated tabs keep their own.", view))
            .child(group(colors, mono.clone(), "Data"))
            .child(setting(colors, "Commit graph depth", "How much history the Commits view reads. Larger graphs take longer on remote-only repositories.", limit))
            .child(setting(
                colors,
                "Auto-refresh",
                "Re-read the visible view in the background, and check every repository for running Actions. Refreshing never shows a loading indicator.",
                refresh_switch,
            ))
            .child(setting(
                colors,
                "Refresh interval",
                "How often auto-refresh runs. Workflow runs you are watching are polled every few seconds either way.",
                refresh,
            ))
            .child(setting(
                colors,
                "Relative dates",
                "Show “3h ago” instead of calendar dates.",
                Switch::new("pref-relative").checked(prefs.relative_dates).on_click(move |on: &bool, _, cx| {
                    let on = *on;
                    let _ = weak.update(cx, |s, cx| s.update_settings(cx, |p| p.relative_dates = on));
                }),
            ))
            .child(group(colors, mono.clone(), "Notifications"))
            .child(setting(
                colors,
                "Desktop alerts",
                "Show a macOS notification for new review requests and CI activity. The Home screen checks GitHub every minute either way.",
                alerts_switch,
            ))
    }

    fn github_page(&self, colors: Palette, mono: SharedString, cx: &mut Context<Self>) -> Div {
        let prefs = settings::get(cx).clone();
        let account: AnyElement = match (&self.user.data, self.user.error()) {
            (Some(u), _) => row()
                .gap_2()
                .child(self.avatar(Some(&u.login), &u.login, 22., cx))
                .child(col().child(u.login.clone()).when_some(u.name.clone(), |s, n| s.child(label(colors, n))))
                .into_any_element(),
            (None, Some(_)) => pill("Not signed in", colors.red, colors.red_tint).into_any_element(),
            _ => row().gap_2().child(spinner(colors)).child("Checking…").into_any_element(),
        };
        let merge = self.popover_button(
            "pref-merge",
            picker(colors, "pref-merge-trigger", prefs.merge_method.title().into()),
            None,
            260.,
            |_, weak, cx| {
                let current = settings::get(cx).merge_method;
                MergeMethod::ALL
                    .into_iter()
                    .map(|m| {
                        let weak = weak.clone();
                        Entry::Item(Item::new(m.title()).icon(Glyph::Merge).checked(m == current).on_click(move |_, cx| {
                            let _ = weak.update(cx, |s, cx| s.update_settings(cx, |p| p.merge_method = m));
                        }))
                    })
                    .collect()
            },
            cx,
        );
        let (w1, w2) = (cx.weak_entity(), cx.weak_entity());
        col()
            .child(page_header(colors, mono.clone(), "GitHub", "gh CLI"))
            .child(group(colors, mono.clone(), "Account"))
            .child(setting(
                colors,
                "Signed in as",
                &match self.user.error() {
                    Some(e) => e.to_string(),
                    None => format!(
                        "flowGit uses your gh login and never stores a token. gh {}.",
                        self.gh_version.data.clone().unwrap_or_default()
                    ),
                },
                row()
                    .gap_2()
                    .child(account)
                    .child(
                        icon_button("pref-recheck", Glyph::Refresh, "Check again")
                            .loading(self.user.loading())
                            .on_click(cx.listener(|s, _, _, cx| s.load_account(cx))),
                    ),
            ))
            .child(setting(
                colors,
                "gh executable",
                "Leave empty to search /opt/homebrew/bin, /usr/local/bin and /usr/bin.",
                row()
                    .gap_1()
                    .child(Input::new(&self.gh_path_input).small().w(px(240.)))
                    .child(
                        Button::new("pref-gh-apply")
                            .label("Apply")
                            .outline()
                            .small()
                            .on_click(cx.listener(|s, _, _, cx| {
                                let path = s.gh_path_input.read(cx).value().to_string();
                                s.update_settings(cx, |p| p.gh_path = path);
                                s.load_account(cx);
                            })),
                    ),
            ))
            .child(group(colors, mono.clone(), "Pull requests"))
            .child(setting(colors, "Default merge method", "The main Merge button uses this; the others stay in its menu.", merge))
            .child(setting(
                colors,
                "Delete branch after merge",
                "Passes --delete-branch to gh pr merge.",
                Switch::new("pref-delete").checked(prefs.delete_branch).on_click(move |on: &bool, _, cx| {
                    let on = *on;
                    let _ = w1.update(cx, |s, cx| s.update_settings(cx, |p| p.delete_branch = on));
                }),
            ))
            .child(setting(
                colors,
                "Load avatars",
                "Fetch profile pictures from avatars.githubusercontent.com. Off shows initials.",
                Switch::new("pref-avatars").checked(prefs.avatars).on_click(move |on: &bool, _, cx| {
                    let on = *on;
                    let _ = w2.update(cx, |s, cx| s.update_settings(cx, |p| p.avatars = on));
                }),
            ))
    }

    fn appearance_page(&self, colors: Palette, mono: SharedString, cx: &mut Context<Self>) -> Div {
        let prefs = settings::get(cx).clone();
        let themes = [(ThemePref::System, "System".to_string()), (ThemePref::Light, "Light".into()), (ThemePref::Dark, "Dark".into())];
        let theme = super::pulls::segmented(colors, "pref-theme", &themes, prefs.theme, |s, t, cx| s.update_settings(cx, |p| p.theme = t), cx);
        let font = self.popover_button(
            "pref-font",
            picker(colors, "pref-font-trigger", prefs.mono_font.clone()),
            Some("Search fonts…"),
            300.,
            |s, weak, cx| {
                let current = settings::get(cx).mono_font.clone();
                let names = s.font_names.get_or_init(|| {
                    let mut names = cx
                        .text_system()
                        .all_font_names()
                        .into_iter()
                        .filter(|n| !n.starts_with('.'))
                        .collect::<Vec<_>>();
                    names.sort_by_key(|n| n.to_lowercase());
                    names.dedup();
                    names
                });
                let item = |name: &str, preview: bool| {
                    let weak = weak.clone();
                    let font = name.to_string();
                    let item = Item::new(name.to_string()).checked(current == name).on_click(move |_, cx| {
                        let font = font.clone();
                        let _ = weak.update(cx, |s, cx| s.update_settings(cx, |p| p.mono_font = font));
                    });
                    Entry::Item(if preview { item.font(name.to_string()) } else { item })
                };
                let mut out = vec![Entry::Label("Suggested monospace".into())];
                out.extend(settings::SUGGESTED_MONO.iter().filter(|f| names.iter().any(|n| n == *f)).map(|f| item(f, true)));
                out.push(Entry::Separator);
                out.push(Entry::Label("All installed fonts".into()));
                out.extend(names.iter().map(|n| item(n, false)));
                out
            },
            cx,
        );
        let size = prefs.mono_size;
        let stepper = row()
            .h(px(26.))
            .rounded(px(6.))
            .border_1()
            .border_color(rgb(colors.border))
            .child(
                Button::new("mono-smaller")
                    .icon(IconName::Minus)
                    .ghost()
                    .xsmall()
                    .disabled(size <= *settings::MONO_SIZES.start())
                    .on_click(cx.listener(|s, _, _, cx| s.update_settings(cx, |p| p.mono_size = p.mono_size.saturating_sub(1)))),
            )
            .child(div().w(px(44.)).text_center().text_size(px(13.)).child(format!("{size} px")))
            .child(
                Button::new("mono-larger")
                    .icon(IconName::Plus)
                    .ghost()
                    .xsmall()
                    .disabled(size >= *settings::MONO_SIZES.end())
                    .on_click(cx.listener(|s, _, _, cx| s.update_settings(cx, |p| p.mono_size += 1))),
            );
        let lanes = colors.lanes;
        col()
            .child(page_header(colors, mono.clone(), "Appearance", "Interface"))
            .child(group(colors, mono.clone(), "Theme"))
            .child(setting(colors, "Color theme", "Follow macOS, or keep one appearance.", theme))
            .child(group(colors, mono.clone(), "Monospace"))
            .child(setting(colors, "Font family", "Used for code, diffs, logs, SHAs and branch names.", font))
            .child(setting(colors, "Font size", "Code, diff and log text size.", stepper))
            .child(
                col()
                    .mt_4()
                    .p_3()
                    .gap_2()
                    .rounded(px(8.))
                    .bg(rgb(colors.panel))
                    .border_1()
                    .border_color(rgb(colors.row_border))
                    .child(label(colors, "Preview"))
                    .child(
                        row()
                            .gap(px(6.))
                            .children(lanes.iter().map(|c| div().size(px(10.)).rounded_full().bg(rgb(*c)))),
                    )
                    .child(
                        div()
                            .font_family(mono.clone())
                            .text_size(px(size as f32))
                            .child(row().bg(rgb(colors.red_tint)).child("− let graph = layout(commits);"))
                            .child(row().bg(rgb(colors.green_tint)).child("+ let graph = graph::layout(history.iter());")),
                    ),
            )
            .child(
                row().pt_4().child(div().flex_1()).child(
                    Button::new("reset-appearance")
                        .label("Restore defaults")
                        .ghost()
                        .small()
                        .on_click(cx.listener(|s, _, _, cx| {
                            s.update_settings(cx, |p| {
                                let d = Settings::default();
                                p.theme = d.theme;
                                p.mono_font = d.mono_font;
                                p.mono_size = d.mono_size;
                            })
                        })),
                ),
            )
    }

    pub(super) fn preferences_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = palette(cx);
        let mono = settings::mono(cx);
        let content = match self.prefs_page {
            PrefsPage::General => self.general_page(colors, mono, cx),
            PrefsPage::GitHub => self.github_page(colors, mono, cx),
            PrefsPage::Appearance => self.appearance_page(colors, mono, cx),
        };
        div()
            .absolute()
            .inset_0()
            .occlude()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgba(colors.scrim))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .capture_key_down(cx.listener(|s, e: &KeyDownEvent, w, cx| {
                if e.keystroke.key == "escape" && s.popup.is_none() {
                    cx.stop_propagation();
                    s.close_prefs(w, cx);
                }
            }))
            .child(
                row()
                    .items_start()
                    .w(px(940.))
                    .max_w(relative(0.94))
                    .h(px(640.))
                    .max_h(relative(0.92))
                    .bg(rgb(colors.background))
                    .border_1()
                    .border_color(rgb(colors.border))
                    .rounded(px(12.))
                    .shadow_xl()
                    .overflow_hidden()
                    .child(self.prefs_nav(colors, cx))
                    .child(
                        col()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .child(
                                row().h(px(44.)).flex_shrink_0().px_3().child(div().flex_1()).child(
                                    Button::new("close-prefs")
                                        .icon(IconName::Close)
                                        .ghost()
                                        .small()
                                        .tooltip("Close · esc")
                                        .on_click(cx.listener(|s, _, w, cx| s.close_prefs(w, cx))),
                                ),
                            )
                            .child(
                                div()
                                    .id("prefs-content")
                                    .flex_1()
                                    .min_h_0()
                                    .overflow_y_scroll()
                                    .px(px(40.))
                                    .pb(px(40.))
                                    .child(content.max_w(px(700.))),
                            ),
                    ),
            )
            .into_any_element()
    }
}
