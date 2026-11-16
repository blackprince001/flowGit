//! Code search inside the Code view: a field in its header, and results that
//! replace the file tree until you leave. Results are grouped by file, one row per
//! matching line; clicking one opens the file on that line.
use super::ui::*;
use super::*;
use crate::util;
use gpui::prelude::FluentBuilder;
use gpui_component::{
    IconName, Sizable,
    input::Input,
};

const LINE: f32 = 22.;

actions!(gitflow, [SearchCode]);

/// The Code view's search, as one flow.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum CodeSearch {
    /// The file tree or file viewer shows.
    #[default]
    Off,
    /// The field is open and nothing has been searched yet.
    Ready,
    /// `query` was submitted. Its results live in `RepoData::search`.
    Results { query: String },
}
pub enum SearchEvent {
    Open,
    Submit(String),
    Leave,
}
impl CodeSearch {
    pub fn next(self, event: SearchEvent) -> Self {
        match (self, event) {
            (_, SearchEvent::Leave) => Self::Off,
            (Self::Off, SearchEvent::Open) => Self::Ready,
            (state, SearchEvent::Open) => state,
            // Only an open field can be submitted.
            (Self::Off, SearchEvent::Submit(_)) => Self::Off,
            (_, SearchEvent::Submit(q)) => match q.trim() {
                "" => Self::Ready,
                q => Self::Results { query: q.to_string() },
            },
        }
    }
    pub fn is_on(&self) -> bool {
        !matches!(self, Self::Off)
    }
}

/// The line a search result opened a file on, kept with its file so it can't
/// mark a line of some other file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileLine {
    pub path: String,
    pub line: u32,
}

/// Results flattened for a virtualized list: a header per file, then its lines.
pub struct SearchView {
    pub files: Vec<gh::FileMatches>,
    pub rows: Vec<(usize, Option<usize>)>,
    pub matches: usize,
    pub truncated: bool,
}
impl SearchView {
    pub fn new(found: gh::CodeMatches) -> Self {
        let mut rows = vec![];
        for (f, file) in found.files.iter().enumerate() {
            rows.push((f, None));
            rows.extend((0..file.lines.len()).map(|l| (f, Some(l))));
        }
        Self {
            matches: found.files.iter().map(|f| f.lines.len()).sum(),
            truncated: found.truncated,
            files: found.files,
            rows,
        }
    }
}

fn plural(n: usize, one: &str) -> String {
    format!("{} {one}{}", util::thousands(n as u64), if n == 1 { "" } else { "s" })
}

impl Workspace {
    /// Open the search field on the current tab's Code view.
    pub(super) fn search_code(&mut self, _: &SearchCode, window: &mut Window, cx: &mut Context<Self>) {
        let Some(t) = self.current_mut() else { return };
        t.view = View::Code;
        t.code_search = std::mem::take(&mut t.code_search).next(SearchEvent::Open);
        t.code_query.update(cx, |i, cx| i.focus(window, cx));
        cx.notify();
    }
    /// Close the search if the Code view is showing one. True when it did.
    pub(super) fn leave_search(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(t) = self.current_mut() else { return false };
        if t.view != View::Code || !t.code_search.is_on() {
            return false;
        }
        t.code_search = std::mem::take(&mut t.code_search).next(SearchEvent::Leave);
        self.focus.focus(window);
        cx.notify();
        true
    }
    /// Enter in a tab's search field.
    pub(super) fn submit_search(&mut self, tab: u64, query: String, cx: &mut Context<Self>) {
        let Some(i) = self.tabs.iter().position(|t| t.id == tab) else { return };
        let branch = self.branch_of(&self.tabs[i]);
        let t = &mut self.tabs[i];
        t.code_search = std::mem::take(&mut t.code_search).next(SearchEvent::Submit(query));
        // Searching again after a failure tries again.
        if let (CodeSearch::Results { query }, Some(branch)) = (&t.code_search, branch)
            && let Some(d) = self.data.get_mut(&t.repo)
            && d.search.get(&(branch.clone(), query.clone())).is_some_and(|l| l.error().is_some())
        {
            d.search.remove(&(branch, query.clone()));
        }
        cx.notify();
    }
    /// Start the search the tab asks for, unless it already has an answer.
    pub(super) fn ensure_search(&mut self, key: &str, slug: &str, path: Option<std::path::PathBuf>, branch: &str, force: bool, cx: &mut Context<Self>) {
        let Some(t) = self.current() else { return };
        let CodeSearch::Results { query } = &t.code_search else { return };
        let query = query.clone();
        // A local clone is grepped, unless the tab browses another branch.
        let local = path.filter(|_| t.branch.is_none());
        let k = (branch.to_string(), query.clone());
        let slug = slug.to_string();
        self.fetch(
            key,
            move |d| d.search.entry(k.clone()).or_default(),
            force,
            move || Ok(Arc::new(SearchView::new(gh::search_code(&slug, local.as_deref(), &query)?))),
            cx,
        );
    }
    /// Open a result's file, on its line when the search knows it.
    fn open_hit(&mut self, path: &str, line: Option<u32>, cx: &mut Context<Self>) {
        if let Some(t) = self.current_mut() {
            t.code_search = std::mem::take(&mut t.code_search).next(SearchEvent::Leave);
            t.target_line = line.map(|line| FileLine { path: path.to_string(), line });
            if let Some(line) = line {
                // The viewer consumes this when its list first draws, once the file has loaded.
                t.list_scroll
                    .scroll_to_item_strict(line.saturating_sub(1) as usize, ScrollStrategy::Center);
            }
        }
        self.show_file(path, cx);
    }

    /// The header's field in place of the latest commit.
    pub(super) fn search_field(&self, colors: Palette, t: &Tab, cx: &mut Context<Self>) -> Div {
        row()
            .flex_1()
            .min_w_0()
            .gap_1()
            .child(
                div()
                    .flex_1()
                    .max_w(px(560.))
                    // The field swallows Escape, so it has to hear it itself.
                    .on_key_down(cx.listener(|s, e: &KeyDownEvent, window, cx| {
                        if e.keystroke.key == "escape" && s.leave_search(window, cx) {
                            cx.stop_propagation();
                        }
                    }))
                    .child(
                        Input::new(&t.code_query)
                            .small()
                            .prefix(icon(IconName::Search).text_color(rgb(colors.muted)))
                            .cleanable(true),
                    ),
            )
            .child(icon_button("search-close", IconName::Close, "Close search · esc").on_click(cx.listener(|s, _, window, cx| {
                s.leave_search(window, cx);
            })))
    }

    /// The bar under the header: what the search found.
    pub(super) fn search_summary(&self, colors: Palette, t: &Tab, branch: &str) -> Div {
        let bar = row().h(px(32.)).flex_shrink_0().px_3().gap_2().border_b_1().border_color(rgb(colors.row_border)).text_size(px(13.));
        let CodeSearch::Results { query } = &t.code_search else {
            return bar.text_color(rgb(colors.muted)).child("Search code");
        };
        let l = self.data(&t.repo).and_then(|d| d.search.get(&(branch.to_string(), query.clone())));
        match l.and_then(|l| l.data.as_ref()) {
            Some(v) => bar
                .child(div().font_weight(FontWeight::MEDIUM).child(format!("{} in {}", plural(v.matches, "match"), plural(v.files.len(), "file"))))
                .child(div().min_w_0().truncate().text_color(rgb(colors.muted)).child(format!("for “{query}”")))
                .when(v.truncated, |s| s.child(label(colors, "· more than this, narrow the search"))),
            None if l.is_some_and(|l| l.error().is_some()) => bar.text_color(rgb(colors.muted)).child("Search failed"),
            None => bar.text_color(rgb(colors.muted)).child(spinner(colors)).child("Searching…"),
        }
    }

    /// The results, in place of the file tree or file viewer.
    pub(super) fn search_results(&self, colors: Palette, t: &Tab, branch: &str, cx: &mut Context<Self>) -> AnyElement {
        let CodeSearch::Results { query } = &t.code_search else {
            let (title, detail) = if self.is_local(&t.repo) && t.branch.is_none() {
                ("Search this repository", "Searches the files in your working tree. Type a word or phrase and press Enter.")
            } else {
                ("Search this repository", "Searches the default branch on GitHub. Type a word or phrase and press Enter.")
            };
            return empty(colors, IconName::Search, title, detail).into_any_element();
        };
        let l = self.data(&t.repo).and_then(|d| d.search.get(&(branch.to_string(), query.clone())));
        let body = match state(colors, l, "Searching…") {
            Err(placeholder) => placeholder.into_any_element(),
            Ok(()) => {
                let v = l.and_then(|l| l.data.clone()).unwrap();
                if v.rows.is_empty() {
                    empty(colors, IconName::Search, "No matches", format!("Nothing in this repository matches “{query}”")).into_any_element()
                } else {
                    self.hit_list(colors, t.id, query, v, cx)
                }
            }
        };
        col()
            .size_full()
            .child(div().flex_1().min_h_0().child(body))
            .when(t.branch.is_some(), |s| {
                s.child(
                    row()
                        .h(px(28.))
                        .flex_shrink_0()
                        .px_3()
                        .gap_2()
                        .border_t_1()
                        .border_color(rgb(colors.row_border))
                        .text_color(rgb(colors.muted))
                        .text_size(px(12.))
                        .child(icon(IconName::Info).size(px(12.)))
                        .child("GitHub code search only covers the default branch, so these results may not match this branch."),
                )
            })
            .into_any_element()
    }

    fn hit_list(&self, colors: Palette, tab: u64, query: &str, view: Arc<SearchView>, cx: &mut Context<Self>) -> AnyElement {
        let mono = settings::mono(cx);
        let size = px(settings::get(cx).mono_size as f32);
        let rows = view.rows.len();
        // A new query is a new list, so it starts at the top.
        let id = ("search-hits", tab ^ util::hash(query) as u64);
        let mark = HighlightStyle {
            background_color: Some(Hsla::from(rgb(colors.amber)).opacity(0.35)),
            ..Default::default()
        };
        uniform_list(
            id,
            rows,
            cx.processor(move |_, range: std::ops::Range<usize>, _, cx| {
                let colors = palette(cx);
                range
                    .map(|i| {
                        let (f, line) = view.rows[i];
                        let file = &view.files[f];
                        let path = file.path.clone();
                        match line {
                            None => row()
                                .id(("hit-file", i))
                                .h(px(LINE))
                                .w_full()
                                .px_2()
                                .gap_2()
                                .bg(rgb(colors.chrome))
                                .border_y_1()
                                .border_color(rgb(colors.border))
                                .font_family(".SystemUIFont")
                                .text_size(px(12.5))
                                .cursor_pointer()
                                .hover(|s| s.bg(rgb(colors.hover)))
                                .child(icon(IconName::File).size(px(13.)).text_color(rgb(colors.muted)))
                                .child(div().flex_1().min_w_0().truncate().font_weight(FontWeight::MEDIUM).child(file.path.clone()))
                                .when(!file.lines.is_empty(), |s| s.child(count_badge(colors, file.lines.len())))
                                .on_click(cx.listener(move |s, _, _, cx| s.open_hit(&path, None, cx)))
                                .into_any_element(),
                            Some(l) => {
                                let hit = &file.lines[l];
                                let n = hit.line;
                                let text = SharedString::from(hit.text.clone());
                                let code = if hit.ranges.is_empty() {
                                    text.into_any_element()
                                } else {
                                    StyledText::new(text)
                                        .with_highlights(hit.ranges.iter().map(|r| (r.clone(), mark)).collect::<Vec<_>>())
                                        .into_any_element()
                                };
                                row()
                                    .id(("hit", i))
                                    .h(px(LINE))
                                    .w_full()
                                    .font_family(mono.clone())
                                    .text_size(size)
                                    .cursor_pointer()
                                    .hover(|s| s.bg(rgb(colors.hover)))
                                    .child(
                                        div()
                                            .w(px(64.))
                                            .flex_shrink_0()
                                            .pr_3()
                                            .text_right()
                                            .text_color(rgb(colors.quiet))
                                            .child(n.map(|n| n.to_string()).unwrap_or_default()),
                                    )
                                    .child(div().flex_1().min_w_0().truncate().text_color(rgb(colors.text)).child(code))
                                    .on_click(cx.listener(move |s, _, _, cx| s.open_hit(&path, n, cx)))
                                    .into_any_element()
                            }
                        }
                    })
                    .collect()
            }),
        )
        .h_full()
        .w_full()
        .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: it would bring gpui's own `test` attribute in over the standard one.
    use super::{CodeSearch, SearchEvent, SearchView};
    use crate::gh;

    #[test]
    fn opening_keeps_a_search_that_is_showing() {
        assert_eq!(CodeSearch::Off.next(SearchEvent::Open), CodeSearch::Ready);
        let showing = CodeSearch::Results { query: "x".into() };
        assert_eq!(showing.clone().next(SearchEvent::Open), showing);
    }
    #[test]
    fn submitting_runs_the_trimmed_query() {
        assert_eq!(
            CodeSearch::Ready.next(SearchEvent::Submit("  fn main ".into())),
            CodeSearch::Results { query: "fn main".into() }
        );
        // An empty field goes back to waiting; a closed search ignores Enter.
        assert_eq!(CodeSearch::Results { query: "x".into() }.next(SearchEvent::Submit("  ".into())), CodeSearch::Ready);
        assert_eq!(CodeSearch::Off.next(SearchEvent::Submit("x".into())), CodeSearch::Off);
    }
    #[test]
    fn leaving_always_closes() {
        for s in [CodeSearch::Off, CodeSearch::Ready, CodeSearch::Results { query: "x".into() }] {
            assert_eq!(s.next(SearchEvent::Leave), CodeSearch::Off);
        }
    }
    #[test]
    fn flattens_results_into_rows() {
        let line = |n| gh::MatchLine { line: Some(n), text: "x".into(), ranges: vec![] };
        let v = SearchView::new(gh::CodeMatches {
            files: vec![
                gh::FileMatches { path: "a".into(), lines: vec![line(1), line(2)] },
                gh::FileMatches { path: "b".into(), lines: vec![line(9)] },
            ],
            truncated: false,
        });
        assert_eq!(v.rows, [(0, None), (0, Some(0)), (0, Some(1)), (1, None), (1, Some(0))]);
        assert_eq!(v.matches, 3);
    }
}
