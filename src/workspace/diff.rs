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
