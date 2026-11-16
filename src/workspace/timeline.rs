//! The conversation rail shared by pull requests and issues: each item has an
//! avatar (or badge) in the left column and the line runs down to the next one.
use super::pulls::{AVATAR, RAIL_INSET, rail_color};
use super::ui::*;
use super::*;
use gpui::prelude::FluentBuilder;

impl Workspace {
    /// Lay out `items` (avatar or badge, content), already in order, on the rail
    /// inside a scrolling page. `id` names the scroll container.
    pub(super) fn timeline(&self, id: &'static str, items: Vec<(AnyElement, Div)>, cx: &mut Context<Self>) -> AnyElement {
        let colors = palette(cx);
        let Some(t) = self.current() else { return div().into_any_element() };
        let last = items.len().saturating_sub(1);
        let mut thread = col();
        for (i, (avatar, content)) in items.into_iter().enumerate() {
            thread = thread.child(
                div()
                    .relative()
                    .flex()
                    .w_full()
                    // Absolute so it spans the item's full height, through to the next avatar.
                    .when(i < last, |d| {
                        d.child(
                            div()
                                .absolute()
                                .left(px(AVATAR / 2. - 1.))
                                .top(px(AVATAR))
                                .bottom_0()
                                .w(px(2.))
                                .bg(rgb(rail_color(colors))),
                        )
                    })
                    .child(col().w(px(AVATAR)).flex_shrink_0().items_center().child(avatar))
                    .child(
                        content
                            .flex_1()
                            .min_w_0()
                            .pl(px(RAIL_INSET - AVATAR))
                            .pt(px(7.))
                            .pb(px(if i < last { 26. } else { 0. })),
                    ),
            );
        }
        div()
            .id((id, t.id))
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&t.page_scroll)
            .child(col().p_4().max_w(px(900.)).w_full().mx_auto().child(thread))
            .into_any_element()
    }
}
