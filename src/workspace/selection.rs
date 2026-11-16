use super::LineTarget;

/// The diff lines picked for a comment: one line, or a range inside one file.
/// The four parts always describe the same pick, so they live together.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineSelection {
    /// The first line of a range. `None` for a single line.
    pub start: Option<LineTarget>,
    /// The line the comment attaches to: the only line, or the last of a range.
    pub end: LineTarget,
    /// The selected diff rows, first to last.
    pub rows: (usize, usize),
    /// The row a shift-click extends from.
    pub anchor: usize,
}

impl LineSelection {
    /// Pick the line on `row`, as a click on its comment mark does.
    ///
    /// `prev` is what was selected before. With shift held, a click extends
    /// from the previous anchor to `row`, in either direction. A click on a
    /// multi-line thread's end (`thread_from` is the row it starts on)
    /// selects the block it covers. `line_at(r)` gives the commentable line on
    /// row `r` in the clicked file, or `None` for a hunk header, another file
    /// or a row with no line. A range with any such row is refused and the
    /// click picks `row` alone.
    pub fn click(
        prev: Option<&Self>,
        row: usize,
        clicked: LineTarget,
        shift: bool,
        thread_from: Option<usize>,
        line_at: impl Fn(usize) -> Option<LineTarget>,
    ) -> Self {
        let (a, b) = match prev.map(|p| p.anchor) {
            Some(anchor) if shift && anchor != row => (anchor.min(row), anchor.max(row)),
            _ => match thread_from {
                Some(from) => (from, row),
                None => (row, row),
            },
        };
        let (a, b) = if (a..=b).all(|r| line_at(r).is_some()) { (a, b) } else { (row, row) };
        // A fresh pick, or none before it, anchors here. A range keeps its anchor.
        let anchor = match prev {
            Some(p) if (a, b) != (row, row) => p.anchor,
            _ => row,
        };
        Self {
            start: (a != b).then(|| line_at(a)).flatten(),
            end: line_at(b).unwrap_or(clicked),
            rows: (a, b),
            anchor,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::LineSelection;
    use crate::workspace::LineTarget;

    fn at(path: &str, line: u32) -> LineTarget {
        LineTarget {
            path: path.into(),
            line,
            right: true,
        }
    }
    /// Rows 0..10 are lines 1..=10 of a.rs, except row 5, a hunk header.
    /// Rows 10..13 are lines 1..=3 of b.rs.
    fn line_in(path: &'static str) -> impl Fn(usize) -> Option<LineTarget> {
        // What the diff view hands in: only rows of the clicked file count.
        move |row| {
            let t = match row {
                0..10 => at("a.rs", row as u32 + 1),
                10..13 => at("b.rs", row as u32 - 9),
                _ => return None,
            };
            (row != 5 && t.path == path).then_some(t)
        }
    }
    fn click(prev: Option<&LineSelection>, row: usize, shift: bool, thread_from: Option<usize>) -> LineSelection {
        let line_at = line_in("a.rs");
        LineSelection::click(prev, row, line_at(row).unwrap(), shift, thread_from, line_at)
    }

    #[test]
    fn a_click_picks_one_line() {
        let s = click(None, 2, false, None);
        assert_eq!(s.rows, (2, 2));
        assert_eq!(s.start, None);
        assert_eq!(s.end, at("a.rs", 3));
        assert_eq!(s.anchor, 2);
    }

    #[test]
    fn a_plain_click_replaces_the_selection() {
        let first = click(None, 2, false, None);
        let s = click(Some(&first), 7, false, None);
        assert_eq!((s.rows, s.anchor, s.start), ((7, 7), 7, None));
    }

    #[test]
    fn shift_click_extends_down() {
        let first = click(None, 1, false, None);
        let s = click(Some(&first), 4, true, None);
        assert_eq!(s.rows, (1, 4));
        assert_eq!(s.start, Some(at("a.rs", 2)));
        assert_eq!(s.end, at("a.rs", 5));
        assert_eq!(s.anchor, 1);
    }

    #[test]
    fn shift_click_extends_up() {
        let first = click(None, 8, false, None);
        let s = click(Some(&first), 6, true, None);
        assert_eq!(s.rows, (6, 8));
        assert_eq!(s.start, Some(at("a.rs", 7)));
        assert_eq!(s.end, at("a.rs", 9));
        assert_eq!(s.anchor, 8);
    }

    #[test]
    fn the_anchor_holds_across_several_shift_clicks() {
        let first = click(None, 2, false, None);
        let down = click(Some(&first), 4, true, None);
        let up = click(Some(&down), 0, true, None);
        assert_eq!(up.rows, (0, 2));
        assert_eq!(up.anchor, 2);
    }

    #[test]
    fn shift_click_on_the_anchor_picks_it_alone() {
        let first = click(None, 3, false, None);
        let s = click(Some(&first), 3, true, None);
        assert_eq!((s.rows, s.start), ((3, 3), None));
    }

    #[test]
    fn shift_click_without_a_selection_is_a_plain_click() {
        let s = click(None, 4, true, None);
        assert_eq!((s.rows, s.anchor), ((4, 4), 4));
    }

    #[test]
    fn a_range_over_a_hunk_header_is_refused() {
        let first = click(None, 3, false, None);
        let s = click(Some(&first), 7, true, None);
        assert_eq!((s.rows, s.start, s.anchor), ((7, 7), None, 7));
    }

    #[test]
    fn a_range_into_another_file_is_refused() {
        let first = click(None, 9, false, None);
        let in_b = line_in("b.rs");
        let s = LineSelection::click(Some(&first), 11, in_b(11).unwrap(), true, None, in_b);
        assert_eq!((s.rows, s.start, s.anchor), ((11, 11), None, 11));
        assert_eq!(s.end, at("b.rs", 2));
    }

    #[test]
    fn clicking_a_thread_selects_its_block() {
        let s = click(None, 3, false, Some(1));
        assert_eq!(s.rows, (1, 3));
        assert_eq!(s.start, Some(at("a.rs", 2)));
        assert_eq!(s.end, at("a.rs", 4));
        // Nothing was selected before, so the clicked row is the anchor.
        assert_eq!(s.anchor, 3);
    }

    #[test]
    fn a_thread_click_keeps_the_previous_anchor() {
        let first = click(None, 0, false, None);
        let s = click(Some(&first), 3, false, Some(1));
        assert_eq!((s.rows, s.anchor), ((1, 3), 0));
    }

    #[test]
    fn a_thread_block_across_a_hunk_header_picks_the_row_alone() {
        let s = click(None, 7, false, Some(4));
        assert_eq!((s.rows, s.start), ((7, 7), None));
    }
}
