//! A unified diff as one virtualized list: a header per file, then numbered lines
//! tinted by change. Used by commit details and pull request "Files".
use super::ui::*;
use super::*;
use crate::{
    gh::{FileStatus, LineKind},
    icons::Glyph,
};
use gpui::prelude::FluentBuilder;
use gpui_component::{ActiveTheme, IconName};
use std::collections::HashMap;

const LINE: f32 = 22.;
const GUTTER: f32 = 22.;

// Keyboard stepping through a diff. Bound in main.rs under the `DiffNav`
// context, which the workspace sets while a diff is on screen and which never
// matches while a text input has focus.
actions!(gitflow, [NextFile, PreviousFile, NextHunk, PreviousHunk]);

/// Review comments on a pull request diff: threads per line, and the line
/// whose comment box is open.
pub struct Review {
    pub threads: HashMap<LineTarget, usize>,
    /// Where multi-line threads begin, by the line they end on.
    pub starts: HashMap<LineTarget, LineTarget>,
    /// Pending review comments per line, not yet sent.
    pub drafts: HashMap<LineTarget, usize>,
    /// The selected diff rows, first to last.
    pub rows: Option<(usize, usize)>,
}
impl Review {
    pub fn new(threads: &[gh::Thread], rows: Option<(usize, usize)>) -> Self {
        let mut map = HashMap::new();
        let mut starts: HashMap<LineTarget, LineTarget> = HashMap::new();
        for t in threads {
            if let Some(k) = thread_target(t) {
                *map.entry(k.clone()).or_insert(0) += 1;
                // Several threads on one line: show the widest range.
                if let Some(first) = thread_start(t)
                    && starts.get(&k).is_none_or(|s| first.line < s.line)
                {
                    starts.insert(k, first);
                }
            }
        }
        Self {
            threads: map,
            starts,
            drafts: HashMap::new(),
            rows,
        }
    }
    pub fn with_drafts(mut self, drafts: HashMap<LineTarget, usize>) -> Self {
        self.drafts = drafts;
        self
    }
}
/// The rows that start a file, in order.
pub fn file_rows(diff: &DiffView) -> Vec<usize> {
    diff.rows.iter().enumerate().filter(|(_, (_, line))| line.is_none()).map(|(i, _)| i).collect()
}
/// The rows that start a hunk, in order.
pub fn hunk_rows(diff: &DiffView) -> Vec<usize> {
    diff.rows
        .iter()
        .enumerate()
        .filter(|(_, (f, line))| line.is_some_and(|l| diff.files[*f].lines[l].kind == LineKind::Hunk))
        .map(|(i, _)| i)
        .collect()
}
/// The first of `rows` after `from`, or the last before it.
pub fn step(rows: &[usize], from: usize, forward: bool) -> Option<usize> {
    if forward {
        rows.iter().copied().find(|&r| r > from)
    } else {
        rows.iter().copied().rev().find(|&r| r < from)
    }
}

/// The row keyboard navigation last scrolled to, and the diff it was in.
/// Stepping from it, not from the first visible row, keeps j/k moving near the
/// end of a list, where a target can't reach the top of the view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NavPos {
    pub diff: usize,
    pub row: usize,
}
impl NavPos {
    pub fn new(diff: &Arc<DiffView>, row: usize) -> Self {
        Self { diff: Arc::as_ptr(diff) as usize, row }
    }
    /// The row to step from, if this position still belongs to `diff`.
    pub fn row_in(pos: Option<Self>, diff: &Arc<DiffView>) -> Option<usize> {
        pos.filter(|p| p.diff == Arc::as_ptr(diff) as usize).map(|p| p.row)
    }
}

/// The row to scroll to for a next or previous file (`rows` are file rows) or
/// hunk (`rows` are hunk rows). `top` is the first visible row and `visible` how
/// many fit. `last` is the row navigation last scrolled to; it is the place to
/// step from while it is still on screen, and `top` otherwise (the user has
/// scrolled away, or the diff changed).
pub fn nav_target(rows: &[usize], hunks: bool, top: usize, visible: usize, last: Option<usize>, forward: bool) -> Option<usize> {
    let from = match last {
        Some(l) if (top..=top + visible).contains(&l) => l,
        _ => top,
    };
    if hunks {
        // A hunk lands just under the pinned file header so its @@ line shows.
        step(rows, from + 1, forward).map(|r| r.saturating_sub(1))
    } else {
        step(rows, from, forward)
    }
}

/// Where a thread sits in the current diff; outdated threads have no line.
pub fn thread_target(t: &gh::Thread) -> Option<LineTarget> {
    Some(LineTarget {
        path: t.root.path.clone(),
        line: t.root.line? as u32,
        right: t.root.side.as_deref() != Some("LEFT"),
    })
}
/// The first line of a multi-line thread.
fn thread_start(t: &gh::Thread) -> Option<LineTarget> {
    let line = t.root.start_line? as u32;
    if Some(line as u64) == t.root.line {
        return None;
    }
    Some(LineTarget {
        path: t.root.path.clone(),
        line,
        right: t.root.start_side.as_deref().or(t.root.side.as_deref()) != Some("LEFT"),
    })
}
/// The new-side text of the diff rows `rows.0..=rows.1`: additions and context,
/// without the removed lines. This is what a suggestion replaces.
pub fn new_side_lines(diff: &DiffView, rows: (usize, usize)) -> Vec<String> {
    (rows.0..=rows.1)
        .filter_map(|r| {
            let (f, l) = *diff.rows.get(r)?;
            let line = diff.files[f].lines.get(l?)?;
            matches!(line.kind, LineKind::Add | LineKind::Context).then(|| line.text.clone())
        })
        .collect()
}
/// A fenced suggestion block holding `lines`, fenced wider than any backticks inside.
pub fn suggestion_block(lines: &[String]) -> String {
    let longest = lines
        .iter()
        .flat_map(|l| l.split(|c| c != '`').map(str::len))
        .max()
        .unwrap_or(0);
    let fence = "`".repeat(longest.max(2) + 1);
    format!("{fence}suggestion\n{}\n{fence}\n", lines.join("\n"))
}

/// The commentable line at a diff row, if it is one.
fn row_target(diff: &DiffView, row: usize) -> Option<LineTarget> {
    let (f, l) = *diff.rows.get(row)?;
    let file = &diff.files[f];
    line_target(file, &file.lines[l?])
}
fn line_target(file: &gh::FileDiff, line: &gh::DiffLine) -> Option<LineTarget> {
    let (line, right) = match line.kind {
        LineKind::Add | LineKind::Context => (line.new?, true),
        LineKind::Del => (line.old?, false),
        LineKind::Hunk => return None,
    };
    Some(LineTarget {
        path: file.path.clone(),
        line,
        right,
    })
}

/// A file's header row, in the list and pinned above it.
fn file_header(colors: Palette, file: &gh::FileDiff) -> Div {
    let (glyph, color) = status_glyph(colors, file.status);
    let more = file.lines.len().saturating_sub(DiffView::MAX_LINES);
    row()
        .h(px(LINE))
        .w_full()
        .px_2()
        .gap_2()
        .bg(rgb(colors.chrome))
        .border_y_1()
        .border_color(rgb(colors.border))
        .font_family(".SystemUIFont")
        .text_size(px(12.5))
        .child(glyph.size(px(13.)).text_color(rgb(color)))
        .child(
            div().font_weight(FontWeight::MEDIUM).whitespace_nowrap().child(match &file.old_path {
                Some(old) => format!("{old} → {}", file.path),
                None => file.path.clone(),
            }),
        )
        .child(counts(colors, file.additions, file.deletions))
        .when(file.binary, |s| s.child(label(colors, "binary")))
        .when(more > 0, |s| s.child(label(colors, format!("{more} more lines not shown"))))
}

pub fn status_glyph(colors: Palette, status: FileStatus) -> (gpui_component::Icon, u32) {
    match status {
        FileStatus::Added => (IconName::Plus.into(), colors.green),
        FileStatus::Deleted => (IconName::Minus.into(), colors.red),
        FileStatus::Renamed => (IconName::ArrowRight.into(), colors.blue),
        FileStatus::Modified => (Glyph::FileDiff.into(), colors.amber),
    }
}
pub fn counts(colors: Palette, additions: u32, deletions: u32) -> Div {
    row()
        .gap(px(6.))
        .flex_shrink_0()
        .text_size(px(12.))
        .child(div().text_color(rgb(colors.green)).child(format!("+{additions}")))
        .child(div().text_color(rgb(colors.red)).child(format!("−{deletions}")))
}

impl Workspace {
    /// The diff the active tab is showing, if it is showing one.
    pub(super) fn active_diff(&self) -> Option<Arc<DiffView>> {
        let t = self.current()?;
        let data = self.data(&t.repo)?;
        match t.view {
            View::Pulls => match &t.new_pr {
                compare::NewPr::Open { page: compare::ComparePage::Files, .. } => data.compare_diff.get(&t.new_pr.range().map(|(b, h)| (b.to_string(), h.to_string()))?)?.data.clone(),
                compare::NewPr::Open { .. } => None,
                compare::NewPr::Closed if t.pr_page == PrPage::Files => data.pull_diff.get(&t.pr?)?.data.clone(),
                compare::NewPr::Closed => None,
            },
            View::Commits => Some(data.commit.get(t.commit.as_ref()?)?.data.as_ref()?.diff.clone()),
            _ => None,
        }
    }
    /// Put a suggestion block with the selected lines into the line comment box.
    pub(super) fn insert_suggestion(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(diff), Some(t)) = (self.active_diff(), self.current()) else { return };
        let lines = new_side_lines(&diff, t.line_rows);
        if lines.is_empty() {
            return;
        }
        let input = t.line_composer.clone();
        let mut block = suggestion_block(&lines);
        // Start on a fresh line when there is already text before the cursor.
        let text = input.read(cx).value();
        if !text.is_empty() && !text.ends_with('\n') {
            block.insert(0, '\n');
        }
        input.update(cx, |i, cx| {
            i.insert(block, window, cx);
            i.focus(window, cx);
        });
    }
    /// Scroll the diff to the next or previous file (`files`) or hunk.
    fn step_diff(&mut self, files: bool, forward: bool, cx: &mut Context<Self>) {
        // Menus and dialogs sit over the diff; keys belong to them.
        if self.popup.is_some() || self.overlay != Overlay::None {
            return;
        }
        let (Some(diff), Some(t)) = (self.active_diff(), self.current()) else { return };
        let scroll = t.diff_scroll.clone();
        // The first row in view, which the pinned file header sits over.
        let (offset, height) = {
            let state = scroll.0.borrow();
            (-f32::from(state.base_handle.offset().y), f32::from(state.base_handle.bounds().size.height))
        };
        let top = (offset / LINE).floor().max(0.) as usize;
        let visible = (height / LINE).floor() as usize;
        let rows = if files { file_rows(&diff) } else { hunk_rows(&diff) };
        let last = NavPos::row_in(t.diff_nav, &diff);
        if let Some(row) = nav_target(&rows, !files, top, visible, last, forward) {
            scroll.scroll_to_item(row, ScrollStrategy::Top);
            if let Some(t) = self.current_mut() {
                t.diff_nav = Some(NavPos::new(&diff, row));
            }
            cx.notify();
        }
    }
    pub(super) fn next_file(&mut self, _: &NextFile, _: &mut Window, cx: &mut Context<Self>) {
        self.step_diff(true, true, cx);
    }
    pub(super) fn previous_file(&mut self, _: &PreviousFile, _: &mut Window, cx: &mut Context<Self>) {
        self.step_diff(true, false, cx);
    }
    pub(super) fn next_hunk(&mut self, _: &NextHunk, _: &mut Window, cx: &mut Context<Self>) {
        self.step_diff(false, true, cx);
    }
    pub(super) fn previous_hunk(&mut self, _: &PreviousHunk, _: &mut Window, cx: &mut Context<Self>) {
        self.step_diff(false, false, cx);
    }

    /// The changed-file list; clicking a file scrolls the diff to it.
    pub(super) fn diff_files(&self, colors: Palette, diff: &Arc<DiffView>, scroll: UniformListScrollHandle, cx: &mut Context<Self>) -> AnyElement {
        let mut list = col().py_1();
        for (i, f) in diff.files.iter().enumerate() {
            let (glyph, color) = status_glyph(colors, f.status);
            let (name, dir) = match f.path.rsplit_once('/') {
                Some((d, n)) => (n.to_string(), d.to_string()),
                None => (f.path.clone(), String::new()),
            };
            let target = diff.header_row(i);
            let scroll = scroll.clone();
            list = list.child(
                row()
                    .id(("diff-file", i))
                    .h(px(26.))
                    .px_2()
                    .gap_2()
                    .cursor_pointer()
                    .rounded(px(4.))
                    .hover(|s| s.bg(rgb(colors.hover)))
                    .child(glyph.size(px(12.)).text_color(rgb(color)))
                    .child(
                        row()
                            .flex_1()
                            .min_w_0()
                            .gap(px(5.))
                            .child(div().flex_shrink_0().max_w(relative(0.8)).truncate().child(name))
                            .child(div().min_w_0().truncate().text_size(px(11.)).text_color(rgb(colors.quiet)).child(dir)),
                    )
                    .child(counts(colors, f.additions, f.deletions))
                    .on_click(cx.listener(move |_, _, _, cx| {
                        if let Some(r) = target {
                            scroll.scroll_to_item(r, ScrollStrategy::Top);
                        }
                        cx.notify();
                    })),
            );
        }
        div()
            .id("diff-files")
            .size_full()
            .overflow_y_scroll()
            .px_1()
            .child(list)
            .into_any_element()
    }

    pub(super) fn diff_list(
        &self,
        id: (&'static str, u64),
        diff: Arc<DiffView>,
        scroll: UniformListScrollHandle,
        review: Option<Review>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = palette(cx);
        if diff.files.is_empty() {
            return empty(colors, Glyph::FileDiff, "No file changes", "This change has no textual diff.").into_any_element();
        }
        let mono = settings::mono(cx);
        let size = settings::get(cx).mono_size as f32;
        let longest = diff
            .files
            .iter()
            .flat_map(|f| f.lines.iter().take(DiffView::MAX_LINES))
            .map(|l| l.text.len())
            .max()
            .unwrap_or(0)
            .min(1200);
        let review = review.map(Arc::new);
        let gutter = if review.is_some() { GUTTER } else { 0. };
        let width = 120. + gutter + longest as f32 * size * 0.62;
        let rows = diff.rows.len();
        let d = diff.clone();
        let rv = review.clone();
        let list = uniform_list(
            id,
            rows,
            cx.processor(move |s, range: std::ops::Range<usize>, _, cx| {
                // The pinned header follows the first visible row; redraw when it moves.
                if s.diff_top != range.start {
                    s.diff_top = range.start;
                    cx.notify();
                }
                let colors = palette(cx);
                let dark = cx.theme().mode.is_dark();
                range
                    .map(|i| {
                        let (f, line) = d.rows[i];
                        let file = &d.files[f];
                        match line {
                            None => file_header(colors, file).into_any_element(),
                            Some(l) => {
                                let line = &file.lines[l];
                                let code_fg = if dark { super::syntax::ONE_DARK_FG } else { super::syntax::ONE_LIGHT_FG };
                                let (bg, fg, mark) = match line.kind {
                                    LineKind::Add => (Some(colors.green_tint), code_fg, "+"),
                                    LineKind::Del => (Some(colors.red_tint), code_fg, "−"),
                                    LineKind::Hunk => (Some(colors.blue_tint), colors.muted, ""),
                                    LineKind::Context => (None, code_fg, ""),
                                };
                                let num = |n: Option<u32>| {
                                    div()
                                        .w(px(44.))
                                        .flex_shrink_0()
                                        .pr_2()
                                        .text_right()
                                        .text_color(rgb(colors.quiet))
                                        .child(n.map(|n| n.to_string()).unwrap_or_default())
                                };
                                let text = d.text[f][l].clone();
                                let code = match d.highlights[f].as_ref().and_then(|h| h.line(dark, l)).filter(|st| !st.is_empty()) {
                                    Some(st) => StyledText::new(text).with_highlights(st.clone()).into_any_element(),
                                    None => text.into_any_element(),
                                };
                                let target = rv.as_ref().and_then(|_| line_target(file, line));
                                let picked = target.is_some() && rv.as_ref().and_then(|r| r.rows).is_some_and(|(a, b)| (a..=b).contains(&i));
                                let threads = target.as_ref().and_then(|t| rv.as_ref()?.threads.get(t).copied()).unwrap_or(0);
                                let pending = target.as_ref().and_then(|t| rv.as_ref()?.drafts.get(t).copied()).unwrap_or(0);
                                let comment = rv.as_ref().map(|_| {
                                    let cell = div().id(("comment-line", i)).w(px(GUTTER)).h_full().flex_shrink_0().flex().items_center().justify_center();
                                    let Some(target) = target.clone() else { return cell };
                                    // A selected range draws as one tall pill down the gutter, its
                                    // plus at the top: each row paints its slice.
                                    let span = rv.as_ref().and_then(|r| r.rows).filter(|&(a, b)| a != b && (a..=b).contains(&i));
                                    let mark = if let Some((a, b)) = span {
                                        let (first, last) = (i == a, i == b);
                                        div().w(px(16.)).h_full().flex().flex_col().child(
                                            div()
                                                .flex_1()
                                                .w_full()
                                                .when(first, |s| s.mt(px((LINE - 16.) / 2.)).rounded_t(px(8.)))
                                                .when(last, |s| s.mb(px((LINE - 16.) / 2.)).rounded_b(px(8.)))
                                                .bg(rgb(colors.blue))
                                                .when(first, |s| {
                                                    s.child(
                                                        div()
                                                            .h(px(16.))
                                                            .flex()
                                                            .items_center()
                                                            .justify_center()
                                                            .child(icon(IconName::Plus).size(px(11.)).text_color(rgb(colors.primary_text))),
                                                    )
                                                }),
                                        )
                                    } else if threads + pending > 0 {
                                        // Amber while any of it is still a draft.
                                        div()
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .size(px(16.))
                                            .rounded(px(8.))
                                            .bg(rgb(if pending > 0 { colors.amber } else { colors.blue }))
                                            .text_color(rgb(colors.primary_text))
                                            .font_family(".SystemUIFont")
                                            .text_size(px(10.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child((threads + pending).to_string())
                                    } else {
                                        div()
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .size(px(16.))
                                            .rounded(px(4.))
                                            .bg(rgb(colors.blue))
                                            .when(!picked, |s| s.opacity(0.).group_hover("diff-line", |s| s.opacity(1.)))
                                            .child(icon(IconName::Plus).size(px(11.)).text_color(rgb(colors.primary_text)))
                                    };
                                    let d = d.clone();
                                    // The row a thread ending here starts on, searching up this file.
                                    let thread_from = rv.as_ref().and_then(|r| r.starts.get(&target)).and_then(|start| {
                                        (0..i).rev().take_while(|&r| d.rows[r].0 == f).find(|&r| row_target(&d, r).as_ref() == Some(start))
                                    });
                                    cell.cursor_pointer().child(mark).on_click(cx.listener(move |s, e: &ClickEvent, window, cx| {
                                        if let Some(t) = s.current_mut() {
                                            // Shift-click extends from the first line clicked, as on GitHub,
                                            // within one file and one hunk.
                                            let anchor = t.line_target.as_ref().map(|_| t.line_anchor);
                                            let (a, b) = match anchor {
                                                Some(a) if e.modifiers().shift && a != i => (a.min(i), a.max(i)),
                                                // Opening a multi-line thread selects the block it covers.
                                                _ => match thread_from {
                                                    Some(from) => (from, i),
                                                    None => (i, i),
                                                },
                                            };
                                            let span_ok = (a..=b).all(|r| matches!(d.rows[r], (f2, Some(_)) if f2 == f && row_target(&d, r).is_some()));
                                            let (a, b) = if span_ok { (a, b) } else { (i, i) };
                                            if (a, b) == (i, i) {
                                                t.line_anchor = i;
                                            }
                                            t.line_rows = (a, b);
                                            t.line_start = (a != b).then(|| row_target(&d, a)).flatten();
                                            t.line_target = row_target(&d, b).or(Some(target.clone()));
                                            let input = t.line_composer.clone();
                                            input.update(cx, |i, cx| i.focus(window, cx));
                                        }
                                        cx.notify();
                                    }))
                                });
                                row()
                                    .group("diff-line")
                                    .h(px(LINE))
                                    .w_full()
                                    .when_some(bg, |s, bg| s.bg(rgb(bg)))
                                    .when(picked, |s| s.bg(rgb(colors.selected)))
                                    .children(comment)
                                    .child(num(line.old))
                                    .child(num(line.new))
                                    .child(
                                        div()
                                            .w(px(16.))
                                            .flex_shrink_0()
                                            .text_color(rgb(match line.kind {
                                                LineKind::Add => colors.green,
                                                LineKind::Del => colors.red,
                                                _ => colors.quiet,
                                            }))
                                            .child(mark),
                                    )
                                    .child(div().pr_4().whitespace_nowrap().text_color(rgb(fg)).child(code))
                                    .into_any_element()
                            }
                        }
                    })
                    .collect()
            }),
        )
        .track_scroll(scroll.clone())
        // Inside a horizontal scroller: keep sideways swipes from also scrolling rows.
        .map(|mut list| {
            list.style().restrict_scroll_to_axis = Some(true);
            list
        })
        .h_full()
        .min_w(px(width))
        .w_full();
        // Pin the header of the file at the top of the view; the next file's
        // header pushes it up and out as it arrives.
        let offset = -f32::from(scroll.0.borrow().base_handle.offset().y);
        let top = (offset / LINE).floor().max(0.) as usize;
        let sticky = (offset > 0.5).then(|| diff.rows.get(top)).flatten().map(|&(f, _)| {
            let next_is_header = matches!(diff.rows.get(top + 1), Some((_, None)));
            let shift = if next_is_header { offset - top as f32 * LINE } else { 0. };
            div()
                .absolute()
                .top(px(-shift))
                .left_0()
                .right_0()
                .occlude()
                .child(file_header(colors, &diff.files[f]))
        });
        div()
            .relative()
            .size_full()
            .overflow_hidden()
            .child(
                div()
                    .id(SharedString::from(format!("{}-x-{}", id.0, id.1)))
                    .size_full()
                    .overflow_x_scroll()
                    .map(|mut d| {
                        d.style().restrict_scroll_to_axis = Some(true);
                        d
                    })
                    .font_family(mono)
                    .text_size(px(size))
                    .child(list),
            )
            .children(sticky)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{file_rows, hunk_rows, nav_target, new_side_lines, step, suggestion_block};
    use crate::workspace::DiffView;

    fn sample() -> DiffView {
        // a.txt: two hunks; b.txt: one hunk.
        let text = "diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -1,2 +1,2 @@\n-x\n+y\n k\n@@ -10,2 +10,2 @@\n-p\n+q\n r\ndiff --git a/b.txt b/b.txt\n--- a/b.txt\n+++ b/b.txt\n@@ -1 +1 @@\n-m\n+n\n";
        DiffView::new(crate::gh::parse_diff(text))
    }

    #[test]
    fn finds_file_and_hunk_rows() {
        let d = sample();
        // Rows: 0 header a, 1 hunk, 2-4 lines, 5 hunk, 6-8 lines, 9 header b, 10 hunk, 11-12 lines.
        assert_eq!(file_rows(&d), vec![0, 9]);
        assert_eq!(hunk_rows(&d), vec![1, 5, 10]);
    }
    #[test]
    fn steps_between_rows_and_stops_at_the_ends() {
        let rows = [0, 9, 20];
        assert_eq!(step(&rows, 0, true), Some(9));
        assert_eq!(step(&rows, 5, true), Some(9));
        assert_eq!(step(&rows, 20, true), None);
        // From the middle of a file, previous goes to that file's own header.
        assert_eq!(step(&rows, 14, false), Some(9));
        assert_eq!(step(&rows, 9, false), Some(0));
        assert_eq!(step(&rows, 0, false), None);
        assert_eq!(step(&[], 3, true), None);
    }
    #[test]
    fn steps_from_the_top_row_when_nothing_was_navigated() {
        let files = [0, 9, 20];
        assert_eq!(nav_target(&files, false, 0, 10, None, true), Some(9));
        assert_eq!(nav_target(&files, false, 10, 10, None, false), Some(9));
        // Hunks land one row above the hunk.
        let hunks = [1, 5, 10];
        assert_eq!(nav_target(&hunks, true, 0, 10, None, true), Some(4));
    }
    #[test]
    fn keeps_moving_when_the_target_cannot_reach_the_top() {
        // 25 rows, 10 visible: the view stops at row 15, so file 20 sits mid-screen.
        let files = [0, 9, 20, 23];
        let first = nav_target(&files, false, 9, 10, Some(9), true);
        assert_eq!(first, Some(20));
        // The view is clamped at top 15. From the first visible row this would
        // pick 20 again; from the row navigated to it moves on.
        assert_eq!(nav_target(&files, false, 15, 10, first, true), Some(23));
        // Past the last file there is nowhere to go.
        assert_eq!(nav_target(&files, false, 15, 10, Some(23), true), None);
        // And back up.
        assert_eq!(nav_target(&files, false, 15, 10, Some(23), false), Some(20));
    }
    #[test]
    fn ignores_a_navigation_row_the_user_scrolled_away_from() {
        let files = [0, 9, 20, 40];
        // Last navigated to 9, but the view is now around row 30.
        assert_eq!(nav_target(&files, false, 30, 10, Some(9), true), Some(40));
        assert_eq!(nav_target(&files, false, 30, 10, Some(9), false), Some(20));
    }
    #[test]
    fn steps_hunks_from_the_last_hunk_reached() {
        let hunks = [1, 5, 10, 12];
        // Landed on the hunk at 10 (scrolled to row 9); the next is 12, not 10 again.
        assert_eq!(nav_target(&hunks, true, 6, 10, Some(9), true), Some(11));
        assert_eq!(nav_target(&hunks, true, 6, 10, Some(9), false), Some(4));
    }
    #[test]
    fn suggestion_takes_the_new_side_of_the_selection() {
        let d = sample();
        // Rows 2-4 are "-x", "+y", " k".
        assert_eq!(new_side_lines(&d, (2, 4)), vec!["y", "k"]);
        assert_eq!(new_side_lines(&d, (3, 3)), vec!["y"]);
        // A removed line alone has nothing to suggest.
        assert!(new_side_lines(&d, (2, 2)).is_empty());
    }
    #[test]
    fn suggestion_block_is_fenced() {
        assert_eq!(suggestion_block(&["a".into(), "b".into()]), "```suggestion\na\nb\n```\n");
        // Code that holds a fence gets a longer one.
        assert!(suggestion_block(&["```".into()]).starts_with("````suggestion"));
    }
}
