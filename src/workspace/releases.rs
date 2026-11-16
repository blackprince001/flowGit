//! Releases: a timeline of release cards with notes and downloadable assets.
use super::ui::*;
use super::*;
use crate::{icons::Glyph, util};
use gpui::prelude::FluentBuilder;
use gpui_component::{
    Disableable, IconName, Sizable,
    button::{Button, ButtonVariants},
    input::Input,
};

impl Workspace {
    pub(super) fn releases_view(&self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let colors = palette(cx);
        let Some(t) = self.current() else { return div().into_any_element() };
        let Some(repo) = self.repo(&t.repo).cloned() else { return div().into_any_element() };
        let l = self.data(&t.repo).map(|d| &d.releases);
        let header = row()
            .h(px(BAR + 6.))
            .flex_shrink_0()
            .px_2()
            .gap_2()
            .border_b_1()
            .border_color(rgb(colors.row_border))
            .child(div().flex_1())
            .child(
                Button::new("open-release-draft")
                    .label("Draft release")
                    .icon(Glyph::Tag)
                    .outline()
                    .small()
                    .on_click(cx.listener(|s, _, window, cx| s.open_release_draft(window, cx))),
            );
        let panel = (t.release_draft == ReleaseDraft::Open).then(|| self.release_draft_panel(t, cx));
        let body = match state(colors, l, "Loading releases…") {
            Err(p) => p.into_any_element(),
            Ok(()) => {
                let releases = l.and_then(|l| l.data.as_ref()).unwrap();
                if releases.is_empty() {
                    let url = format!("https://github.com/{}/releases/new", repo.slug());
                    empty(colors, Glyph::Tag, "No releases yet", "Tag a version on GitHub to publish notes and binaries.")
                        .child(
                            Button::new("draft-release")
                                .label("Draft a release on GitHub")
                                .icon(IconName::ExternalLink)
                                .outline()
                                .small()
                                .on_click(move |_, _, cx| cx.open_url(&url)),
                        )
                        .into_any_element()
                } else {
                    // Only cards in view (plus overdraw) are laid out, Markdown included.
                    let list_state = t.release_list.clone();
                    if list_state.item_count() != releases.len() {
                        list_state.reset(releases.len());
                    }
                    list(
                        list_state,
                        cx.processor(|s, i: usize, window, cx| s.release_card(i, window, cx)),
                    )
                    .size_full()
                    .bg(rgb(colors.background))
                    .into_any_element()
                }
            }
        };
        col()
            .size_full()
            .bg(rgb(colors.background))
            .child(header)
            .children(panel)
            .child(div().flex_1().min_h_0().child(body))
            .into_any_element()
    }

    /// The draft form above the list. It sits inline, not in a modal.
    fn release_draft_panel(&self, t: &Tab, cx: &mut Context<Self>) -> AnyElement {
        let colors = palette(cx);
        let busy = self.busy.is_some();
        let generating = t.generated_notes.loading();
        let no_tag = t.draft_tag.read(cx).value().trim().is_empty();
        let fields = col()
            .gap_3()
            .child(
                row()
                    .items_end()
                    .gap_3()
                    .child(field(colors, "Tag", Input::new(&t.draft_tag).small()).w(px(200.)))
                    .child(field(colors, "Target branch", Input::new(&t.draft_branch).small()).flex_1()),
            )
            .child(field(colors, "Title", Input::new(&t.draft_title).small()))
            .child(field(colors, "Notes", Input::new(&t.draft_notes)));
        let footer = row()
            .gap_2()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(12.))
                    .text_color(rgb(colors.red))
                    .child(t.generated_notes.error().unwrap_or_default().to_string()),
            )
            .child(
                Button::new("generate-release-notes")
                    .label("Generate notes")
                    .outline()
                    .small()
                    .loading(generating)
                    .disabled(busy || generating || no_tag)
                    .on_click(cx.listener(|s, _, window, cx| s.generate_release_notes(window, cx))),
            )
            .child(button("cancel-release-draft", "Cancel").on_click(cx.listener(|s, _, _, cx| s.close_release_draft(cx))))
            .child(
                Button::new("save-release-draft")
                    .label("Save draft")
                    .primary()
                    .small()
                    .loading(self.busy_with("Creating draft"))
                    .disabled(busy || no_tag)
                    .on_click(cx.listener(|s, _, _, cx| s.save_release_draft(cx))),
            );
        col()
            .px_4()
            .pt_4()
            .child(
                card(colors)
                    .child(
                        row()
                            .h(px(BAR))
                            .px_4()
                            .border_b_1()
                            .border_color(rgb(colors.row_border))
                            .child(section(colors, "Draft release")),
                    )
                    .child(col().p_4().gap_3().child(fields).child(footer)),
            )
            .into_any_element()
    }

    /// Open the draft panel with empty fields, aimed at the default branch.
    pub(super) fn open_release_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let branch = self.current().and_then(|t| self.default_branch(&t.repo)).unwrap_or_default();
        let Some(t) = self.current_mut() else { return };
        if t.release_draft == ReleaseDraft::Open {
            // Already open: keep what was typed.
            t.draft_tag.update(cx, |s, cx| s.focus(window, cx));
            return;
        }
        t.release_draft = ReleaseDraft::Open;
        t.generated_notes = Load::default();
        t.draft_tag.update(cx, |s, cx| s.set_value("", window, cx));
        t.draft_branch.update(cx, |s, cx| s.set_value(branch, window, cx));
        t.draft_title.update(cx, |s, cx| s.set_value("", window, cx));
        t.draft_notes.update(cx, |s, cx| s.set_value("", window, cx));
        t.draft_tag.update(cx, |s, cx| s.focus(window, cx));
        cx.notify();
    }

    fn close_release_draft(&mut self, cx: &mut Context<Self>) {
        if let Some(t) = self.current_mut() {
            close_draft(t);
        }
        cx.notify();
    }

    /// Ask GitHub to write the title and notes for the draft's tag.
    fn generate_release_notes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(t) = self.current() else { return };
        let Some(repo) = self.repo(&t.repo).cloned() else { return };
        let tag = t.draft_tag.read(cx).value().trim().to_string();
        let branch = t.draft_branch.read(cx).value().trim().to_string();
        if tag.is_empty() || branch.is_empty() {
            self.notice = Notice::warn("Enter a tag and a target branch first.");
            cx.notify();
            return;
        }
        let (id, slug) = (t.id, repo.slug());
        if let Some(t) = self.tabs.iter_mut().find(|t| t.id == id) {
            t.generated_notes.start();
        }
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = smol::unblock(move || gh::generate_notes(&slug, &tag, &branch)).await;
            let _ = this.update_in(cx, |s, w, cx| {
                let Some(t) = s.tabs.iter_mut().find(|t| t.id == id) else { return };
                match result {
                    Ok(notes) => {
                        // A panel closed while the request ran keeps its fields as they were.
                        if t.release_draft == ReleaseDraft::Open {
                            t.draft_title.update(cx, |f, cx| f.set_value(notes.name.clone(), w, cx));
                            t.draft_notes.update(cx, |f, cx| f.set_value(notes.body.clone(), w, cx));
                        }
                        t.generated_notes.finish(Ok(notes));
                    }
                    Err(e) => t.generated_notes.finish(Err(e.to_string())),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Create the draft on GitHub, then refresh the list and close the panel.
    fn save_release_draft(&mut self, cx: &mut Context<Self>) {
        let Some(t) = self.current() else { return };
        let Some(repo) = self.repo(&t.repo).cloned() else { return };
        let tag = t.draft_tag.read(cx).value().trim().to_string();
        let branch = t.draft_branch.read(cx).value().trim().to_string();
        let title = t.draft_title.read(cx).value().trim().to_string();
        let notes = t.draft_notes.read(cx).value().to_string();
        if tag.is_empty() || branch.is_empty() {
            self.notice = Notice::warn("Enter a tag and a target branch first.");
            cx.notify();
            return;
        }
        let title = if title.is_empty() { tag.clone() } else { title };
        let (id, key, slug) = (t.id, repo.key(), repo.slug());
        let refresh = slug.clone();
        self.mutate(
            format!("Creating draft {tag}"),
            Report::Notice,
            move || gh::create_draft(&slug, &tag, &branch, &title, &notes),
            move |s, cx| {
                if let Some(t) = s.tabs.iter_mut().find(|t| t.id == id) {
                    close_draft(t);
                }
                s.fetch(&key, |d| &mut d.releases, true, move || gh::releases(&refresh), cx);
            },
            cx,
        );
    }

    fn release_card(&self, i: usize, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let colors = palette(cx);
        let mono = settings::mono(cx);
        let Some(t) = self.current() else { return div().into_any_element() };
        let Some(releases) = self.data(&t.repo).and_then(|d| d.releases.data.as_ref()) else {
            return div().into_any_element();
        };
        let Some(r) = releases.get(i) else { return div().into_any_element() };
        let latest = releases
            .iter()
            .find(|r| !r.draft && !r.prerelease)
            .map(|r| r.tag_name.clone());
        {
            let is_latest = latest.as_ref() == Some(&r.tag_name);
            let author = r.author.as_ref().map(|a| a.login.clone()).unwrap_or_default();
            let url = r.html_url.clone();
            let tag = r.tag_name.clone();
            let mut card = card(colors)
                .child(
                    row()
                        .px_4()
                        .py_3()
                        .gap_3()
                        .border_b_1()
                        .border_color(rgb(colors.row_border))
                        .child(
                            col()
                                .flex_1()
                                .min_w_0()
                                .gap(px(3.))
                                .child(
                                    row()
                                        .gap_2()
                                        .child(div().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).truncate().child(r.title()))
                                        .when(is_latest, |s| s.child(pill("Latest", colors.green, colors.green_tint)))
                                        .when(r.prerelease, |s| s.child(pill("Pre-release", colors.amber, colors.amber_tint)))
                                        .when(r.draft, |s| s.child(pill("Draft", colors.muted, colors.hover))),
                                )
                                .child(
                                    row()
                                        .gap_2()
                                        .text_size(px(12.))
                                        .text_color(rgb(colors.muted))
                                        .child(chip(colors, mono.clone(), r.tag_name.clone()))
                                        .when(!author.is_empty(), |s| {
                                            s.child(self.avatar(Some(&author), &author, 16., cx)).child(author.clone())
                                        })
                                        .child(format!("released {}", self.when(r.time(), cx))),
                                ),
                        )
                        .child(
                            Button::new(("copy-tag", i))
                                .icon(IconName::Copy)
                                .ghost()
                                .xsmall()
                                .tooltip("Copy tag")
                                .on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(tag.clone()))),
                        )
                        .child(
                            Button::new(("open-release", i))
                                .icon(IconName::ExternalLink)
                                .ghost()
                                .xsmall()
                                .tooltip("Open on GitHub")
                                .on_click(move |_, _, cx| cx.open_url(&url)),
                        ),
                );
            let body = r.body.clone().unwrap_or_default();
            card = card.child(div().px_5().py_4().child(if body.trim().is_empty() {
                label(colors, "No release notes").into_any_element()
            } else {
                markdown(format!("release-{}-{}", t.repo, r.tag_name), &body, window, cx)
            }));
            if !r.assets.is_empty() {
                let mut assets = col().border_t_1().border_color(rgb(colors.row_border)).child(
                    section(colors, format!("Assets · {}", r.assets.len())).px_4(),
                );
                for (k, a) in r.assets.iter().enumerate() {
                    let link = a.browser_download_url.clone();
                    assets = assets.child(
                        row()
                            .id(SharedString::from(format!("asset-{i}-{k}")))
                            .h(px(32.))
                            .px_4()
                            .gap_3()
                            .cursor_pointer()
                            .border_t_1()
                            .border_color(rgb(colors.row_border))
                            .hover(|s| s.bg(rgb(colors.hover)))
                            .child(icon(Glyph::Package).text_color(rgb(colors.muted)))
                            .child(div().flex_1().min_w_0().truncate().child(a.name.clone()))
                            .child(label(colors, util::bytes(a.size)))
                            .child(
                                row()
                                    .gap(px(4.))
                                    .w(px(80.))
                                    .justify_end()
                                    .text_size(px(12.))
                                    .text_color(rgb(colors.muted))
                                    .child(icon(Glyph::Download).size(px(12.)))
                                    .child(util::compact(a.download_count)),
                            )
                            .on_click(move |_, _, cx| cx.open_url(&link)),
                    );
                }
                card = card.child(assets);
            }
            // Timeline: date, a rail fading between neighbouring release colors, the card.
            let tone = |r: &gh::Release| {
                if latest.as_ref() == Some(&r.tag_name) {
                    colors.green
                } else if r.draft {
                    colors.quiet
                } else if r.prerelease {
                    colors.amber
                } else {
                    colors.blue
                }
            };
            let top = if i == 0 { 16. } else { 0. };
            let show_day = i == 0 || !same_day(releases[i - 1].time(), r.time());
            div().w_full().px_4().child(
                div()
                    .flex()
                    .max_w(px(1000.))
                    .w_full()
                    .mx_auto()
                    .gap_2()
                    .child(date_column(colors, r.time(), show_day, top + 16.))
                    .child(rail(
                        i.checked_sub(1).map(|p| tone(&releases[p])),
                        tone(r),
                        releases.get(i + 1).map(tone),
                        top + 20.,
                        None,
                        colors.background,
                    ))
                    .child(div().flex_1().min_w_0().pt(px(top)).pb_5().child(card)),
            )
            .into_any_element()
        }
    }
}

/// A form field: its label above the input.
fn field(colors: Palette, title: &'static str, input: impl IntoElement) -> Div {
    col().min_w_0().gap_1().child(label(colors, title)).child(input)
}

/// Close the draft panel and drop the notes it generated.
fn close_draft(t: &mut Tab) {
    t.release_draft = ReleaseDraft::Closed;
    t.generated_notes = Load::default();
}
