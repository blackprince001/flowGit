//! Small building blocks shared by every view, in CloudWatch v2's visual roles.
use super::*;
use gpui::prelude::FluentBuilder;
use crate::{
    gh::CheckState,
    icons::Glyph,
    menu::Entry,
    util,
};
use gpui_component::{
    Icon, IconName, Sizable,
    button::{Button, ButtonVariants},
    kbd::Kbd,
    spinner::Spinner,
};

/// Shared layout rhythm: tab strip, nav bar and panel headers line up.
pub const BAR: f32 = 34.;
pub const ROW: f32 = 28.;

pub fn row() -> Div {
    div().flex().items_center()
}
pub fn col() -> Div {
    div().flex().flex_col()
}
pub fn label(colors: Palette, text: impl Into<SharedString>) -> Div {
    div()
        .text_color(rgb(colors.muted))
        .text_size(px(12.))
        .child(text.into())
}
pub fn icon(name: impl Into<Icon>) -> Icon {
    name.into().size(px(14.))
}
pub fn button(id: impl Into<ElementId>, text: impl Into<SharedString>) -> Button {
    Button::new(id).label(text).ghost().small()
}
pub fn icon_button(id: impl Into<ElementId>, name: impl Into<Icon>, tip: &'static str) -> Button {
    Button::new(id).icon(name).ghost().small().tooltip(tip)
}
/// An icon toggle that reads as "on" through color.
pub fn toggle(colors: Palette, id: &'static str, name: impl Into<Icon>, tip: &'static str, on: bool) -> Button {
    Button::new(id)
        .icon(icon(name).text_color(rgb(if on { colors.blue } else { colors.muted })))
        .ghost()
        .xsmall()
        .tooltip(tip)
}
pub fn divider(colors: Palette) -> Div {
    div()
        .w(px(1.))
        .h(px(16.))
        .mx_1()
        .flex_shrink_0()
        .bg(rgb(colors.border))
}
pub fn kbd(keys: &str) -> Kbd {
    Kbd::new(Keystroke::parse(keys).expect("valid keystroke"))
}
pub fn pill(text: impl Into<SharedString>, fg: u32, bg: u32) -> Div {
    div()
        .flex_shrink_0()
        .px(px(6.))
        .h(px(18.))
        .flex()
        .items_center()
        .gap(px(4.))
        .rounded(px(4.))
        .bg(rgb(bg))
        .text_color(rgb(fg))
        .text_size(px(11.))
        .font_weight(FontWeight::MEDIUM)
        .child(text.into())
}
pub fn count_badge(colors: Palette, n: usize) -> Div {
    div()
        .flex_shrink_0()
        .min_w(px(18.))
        .h(px(16.))
        .px(px(4.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .bg(rgb(colors.hover))
        .text_size(px(11.))
        .text_color(rgb(colors.muted))
        .child(n.to_string())
}
/// A branch or ref name, in mono on a quiet chip.
pub fn chip(colors: Palette, mono: SharedString, text: impl Into<SharedString>) -> Div {
    tinted_chip(colors.blue, colors.blue_tint, mono, text)
}
pub fn tinted_chip(fg: u32, bg: u32, mono: SharedString, text: impl Into<SharedString>) -> Div {
    div()
        .flex_shrink_0()
        .max_w(px(260.))
        .px(px(6.))
        .h(px(18.))
        .flex()
        .items_center()
        .rounded(px(4.))
        .bg(rgb(bg))
        .text_color(rgb(fg))
        .font_family(mono)
        .text_size(px(11.5))
        .truncate()
        .child(text.into())
}
pub fn spinner(colors: Palette) -> Spinner {
    Spinner::new().xsmall().color(rgb(colors.amber).into())
}

/// A centered placeholder for empty, loading and failed states.
pub fn empty(colors: Palette, glyph: impl Into<Icon>, title: impl Into<SharedString>, detail: impl Into<SharedString>) -> Div {
    col()
        .flex_1()
        .size_full()
        .items_center()
        .justify_center()
        .gap_2()
        .p_6()
        .child(icon(glyph).size(px(22.)).text_color(rgb(colors.quiet)))
        .child(div().font_weight(FontWeight::MEDIUM).child(title.into()))
        .child(
            label(colors, detail)
                .max_w(px(420.))
                .text_center()
                .line_height(relative(1.45)),
        )
}
pub fn loading(colors: Palette, text: &str) -> Div {
    row()
        .flex_1()
        .size_full()
        .justify_center()
        .gap_2()
        .text_color(rgb(colors.muted))
        .child(Spinner::new().small())
        .child(text.to_string())
}
pub fn failed(colors: Palette, error: &str) -> Div {
    empty(colors, IconName::TriangleAlert, "Couldn’t load this", error.to_string())
}

/// Ready content, or a placeholder for loading / failure.
#[allow(clippy::result_large_err)]
pub fn state<T>(colors: Palette, l: Option<&Load<T>>, what: &str) -> Result<(), Div> {
    match l {
        Some(Load { data: Some(_), .. }) => Ok(()),
        Some(Load {
            error: Some(e), ..
        }) => Err(failed(colors, e)),
        _ => Err(loading(colors, what)),
    }
}

/// Run and check outcomes as a glyph and a color.
pub fn run_glyph(colors: Palette, status: &str, conclusion: &str) -> (Icon, u32) {
    let status = status.to_ascii_lowercase();
    if status != "completed" {
        return if status == "in_progress" {
            (Glyph::Activity.into(), colors.amber)
        } else {
            (Glyph::CircleDashed.into(), colors.amber)
        };
    }
    match conclusion.to_ascii_lowercase().as_str() {
        "success" => (IconName::CircleCheck.into(), colors.green),
        "failure" | "timed_out" | "startup_failure" => (IconName::CircleX.into(), colors.red),
        "cancelled" => (Glyph::CircleSlash.into(), colors.quiet),
        "action_required" => (IconName::TriangleAlert.into(), colors.amber),
        _ => (Glyph::CircleSlash.into(), colors.quiet),
    }
}
pub fn run_icon(colors: Palette, status: &str, conclusion: &str) -> AnyElement {
    let (glyph, color) = run_glyph(colors, status, conclusion);
    if status.eq_ignore_ascii_case("in_progress") {
        return spinner(colors).into_any_element();
    }
    icon(glyph).text_color(rgb(color)).into_any_element()
}
pub fn check_icon(colors: Palette, state: CheckState) -> Icon {
    match state {
        CheckState::Pass => icon(IconName::CircleCheck).text_color(rgb(colors.green)),
        CheckState::Fail => icon(IconName::CircleX).text_color(rgb(colors.red)),
        CheckState::Pending => icon(Glyph::CircleDashed).text_color(rgb(colors.amber)),
        CheckState::Skipped => icon(Glyph::CircleSlash).text_color(rgb(colors.quiet)),
    }
}
/// Pull request state as glyph, color and tint: open, draft, merged, closed.
pub fn pr_glyph(colors: Palette, state: &str, draft: bool) -> (Glyph, u32, u32, &'static str) {
    match state {
        "MERGED" => (Glyph::Merge, colors.purple, colors.purple_tint, "Merged"),
        "CLOSED" => (Glyph::PullRequestClosed, colors.red, colors.red_tint, "Closed"),
        _ if draft => (Glyph::PullRequestDraft, colors.muted, colors.hover, "Draft"),
        _ => (Glyph::PullRequest, colors.green, colors.green_tint, "Open"),
    }
}

/// Drop HTML comments that PR templates leave behind before rendering Markdown.
pub fn clean_markdown(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("<!--") {
        out.push_str(&rest[..start]);
        match rest[start..].find("-->") {
            Some(end) => rest = &rest[start + end + 3..],
            None => {
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    out.trim().to_string()
}
/// Markdown as GitHub shows comments and release notes: newlines are line breaks.
pub fn markdown(id: impl Into<SharedString>, text: &str, _window: &mut Window, cx: &mut App) -> AnyElement {
    render_md(id.into(), text, super::md::Breaks::Hard, None, cx)
}
/// Markdown as GitHub shows `.md` files: wrapped lines join into paragraphs.
/// `base` places the file in its repository, for relative image paths.
pub fn markdown_doc(id: impl Into<SharedString>, text: &str, base: Option<super::md::Base>, cx: &mut App) -> AnyElement {
    render_md(id.into(), text, super::md::Breaks::Soft, base, cx)
}
fn render_md(id: SharedString, text: &str, breaks: super::md::Breaks, base: Option<super::md::Base>, cx: &App) -> AnyElement {
    super::md::render(
        id,
        &clean_markdown(text),
        breaks,
        palette(cx),
        settings::mono(cx),
        px(settings::get(cx).mono_size as f32),
        cx.try_global::<crate::avatars::Avatars>().cloned(),
        base,
    )
}

impl Workspace {
    /// A round avatar, or initials on a lane color until the image arrives.
    pub(super) fn avatar(&self, login: Option<&str>, name: &str, size: f32, cx: &App) -> AnyElement {
        let colors = palette(cx);
        let login = login.filter(|l| !l.is_empty());
        if settings::get(cx).avatars
            && let Some(image) = login.and_then(|l| self.avatars.get(l))
        {
            return img(ImageSource::Image(image))
                .size(px(size))
                .flex_shrink_0()
                .rounded_full()
                .into_any_element();
        }
        let seed = login.unwrap_or(name);
        div()
            .size(px(size))
            .flex_shrink_0()
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(Hsla::from(rgb(colors.lane(util::hash(seed)))).opacity(0.18))
            .text_color(rgb(colors.lane(util::hash(seed))))
            .text_size(px((size * 0.42).max(8.)))
            .font_weight(FontWeight::SEMIBOLD)
            .child(util::initials(if name.is_empty() { seed } else { name }))
            .into_any_element()
    }
    /// Wrap a popover trigger so its painted bounds are known when it is clicked.
    pub(super) fn anchor(&self, id: &'static str, trigger: impl IntoElement) -> Div {
        let anchors = self.anchors.clone();
        div().relative().flex_shrink_0().child(trigger).child(
            canvas(
                move |bounds, _, _| {
                    anchors.borrow_mut().insert(id, bounds);
                },
                |_, _, _, _| {},
            )
            .absolute()
            .inset_0(),
        )
    }
    /// A button that opens a popover built from live workspace state.
    pub(super) fn popover_button(
        &self,
        id: &'static str,
        button: Button,
        search: Option<&'static str>,
        width: f32,
        build: impl Fn(&Workspace, WeakEntity<Workspace>, &App) -> Vec<Entry> + 'static,
        cx: &mut Context<Self>,
    ) -> Div {
        let weak = cx.weak_entity();
        let build: popover::Builder = Rc::new(move |cx: &App| {
            weak.upgrade()
                .map(|w| build(w.read(cx), weak.clone(), cx))
                .unwrap_or_default()
        });
        self.anchor(
            id,
            button.on_click(cx.listener(move |s, e: &ClickEvent, w, cx| {
                // Mouse-down outside already closed this trigger's menu: treat the click as a toggle.
                if s.dismissed
                    .take()
                    .is_some_and(|(a, at)| a == id && at.elapsed() < Duration::from_millis(400))
                {
                    return;
                }
                s.open_popover(id, e.position(), search, width, build.clone(), w, cx)
            })),
        )
    }
    pub(super) fn when(&self, ts: i64, cx: &App) -> String {
        util::when(ts, settings::get(cx).relative_dates)
    }
    pub(super) fn busy_with(&self, prefix: &str) -> bool {
        self.busy.as_deref().is_some_and(|b| b.starts_with(prefix))
    }
}

/// A form field: its title above the control.
pub fn field(colors: Palette, title: impl Into<SharedString>, control: impl IntoElement) -> Div {
    col().min_w_0().gap_1().child(label(colors, title)).child(control)
}

/// A titled panel with an optional trailing control, used by detail pages.
pub fn section(colors: Palette, title: impl Into<SharedString>) -> Div {
    row()
        .h(px(30.))
        .gap_2()
        .text_size(px(12.))
        .text_color(rgb(colors.muted))
        .font_weight(FontWeight::MEDIUM)
        .child(title.into())
}
pub fn card(colors: Palette) -> Div {
    col()
        .rounded(px(8.))
        .border_1()
        .border_color(rgb(colors.border))
        .bg(rgb(colors.background))
        .overflow_hidden()
}
// ── Timeline ─────────────────────────────────────────────────────────────────

pub const RAIL: f32 = 28.;
pub const DATE_COL: f32 = 92.;
const DOT: f32 = 10.;

/// Blend two RGB colors; `t` = 0 gives `a`, 1 gives `b`.
pub fn mix(a: u32, b: u32, t: f32) -> u32 {
    let ch = |c: u32, s: u32| ((c >> s) & 0xff) as f32;
    let blend = |s: u32| ((ch(a, s) + (ch(b, s) - ch(a, s)) * t).round().clamp(0., 255.) as u32) << s;
    blend(16) | blend(8) | blend(0)
}

/// A vertical stretch of rail fading from one color into another.
fn rail_piece(from: u32, to: u32) -> Div {
    div()
        .w(px(2.))
        .bg(linear_gradient(180., linear_color_stop(rgb(from), 0.), linear_color_stop(rgb(to), 1.)))
}

/// The middle column of a timeline row. The rail runs unbroken through the dot's
/// centre and the dot is drawn over it, so line and dot always join. Rails meet
/// neighbouring rows halfway between the two status colors, so the line reads as
/// one gradient from each item's color to the next. `dot` replaces the plain dot
/// (e.g. a spinner); it sits on a disc of `surface` so the rail doesn't show through.
pub fn rail(
    prev: Option<u32>,
    color: u32,
    next: Option<u32>,
    dot_top: f32,
    dot: Option<AnyElement>,
    surface: u32,
) -> Div {
    const SLOT: f32 = 14.;
    let centre = dot_top + SLOT / 2.;
    let marker = match dot {
        Some(custom) => div()
            .absolute()
            .top(px(dot_top))
            .left(px((RAIL - SLOT) / 2.))
            .size(px(SLOT))
            .rounded_full()
            .bg(rgb(surface))
            .flex()
            .items_center()
            .justify_center()
            .child(custom),
        None => div()
            .absolute()
            .top(px(centre - DOT / 2.))
            .left(px((RAIL - DOT) / 2.))
            .size(px(DOT))
            .rounded_full()
            .bg(rgb(color)),
    };
    col()
        .relative()
        .w(px(RAIL))
        .flex_shrink_0()
        .items_center()
        .child(match prev {
            Some(p) => rail_piece(mix(p, color, 0.5), color).h(px(centre)),
            None => div().h(px(centre)),
        })
        .child(match next {
            Some(n) => rail_piece(color, mix(color, n, 0.5)).flex_1(),
            None => div().flex_1(),
        })
        .child(marker)
}

/// "Today", "Yesterday", "Nov 14" or "Nov 14, 2024" in local time.
pub fn day(ts: i64) -> String {
    use chrono::{Datelike, Local, TimeZone};
    let (Some(t), Some(n)) = (Local.timestamp_opt(ts, 0).single(), Local.timestamp_opt(util::now(), 0).single()) else {
        return String::new();
    };
    let days = (n.date_naive() - t.date_naive()).num_days();
    match days {
        0 => "Today".into(),
        1 => "Yesterday".into(),
        _ if t.year() == n.year() => t.format("%b %-d").to_string(),
        _ => t.format("%b %-d, %Y").to_string(),
    }
}
pub fn clock(ts: i64) -> String {
    use chrono::{Local, TimeZone};
    Local
        .timestamp_opt(ts, 0)
        .single()
        .map(|t| t.format("%-I:%M %p").to_string())
        .unwrap_or_default()
}
/// The left column: the day (only where it changes) above the time.
pub fn date_column(colors: Palette, ts: i64, show_day: bool, top: f32) -> Div {
    col()
        .w(px(DATE_COL))
        .flex_shrink_0()
        .items_end()
        .pt(px(top))
        .pr_1()
        .gap(px(1.))
        .when(show_day, |s| s.child(div().text_size(px(12.5)).font_weight(FontWeight::MEDIUM).child(day(ts))))
        .child(div().text_size(px(11.5)).text_color(rgb(colors.muted)).child(clock(ts)))
}
pub fn same_day(a: i64, b: i64) -> bool {
    day(a) == day(b)
}

#[cfg(test)]
mod tests {
    #[test]
    fn strips_template_comments() {
        assert_eq!(super::clean_markdown("a<!-- hidden -->b\n<!-- x"), "ab");
        assert_eq!(super::clean_markdown("plain"), "plain");
    }
    #[test]
    fn mixes_colors() {
        assert_eq!(super::mix(0x000000, 0xffffff, 0.5), 0x808080);
        assert_eq!(super::mix(0xff0000, 0x0000ff, 0.), 0xff0000);
        assert_eq!(super::mix(0xff0000, 0x0000ff, 1.), 0x0000ff);
    }
}
