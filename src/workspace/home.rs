//! The home screen, shown when no repository tab is active: what is waiting on
//! you across GitHub (review requests, your pull requests, assigned issues) and
//! your unread notifications.
use super::ui::*;
use super::*;
use crate::{
    gh::{InboxItem, Notification, Target},
    icons::Glyph,
    util,
};
use gpui::prelude::FluentBuilder;
use gpui_component::{Disableable, Icon, IconName};
use std::collections::HashSet;

/// How often the unread notifications are checked, with Home showing or not.
pub const NOTIFICATION_POLL: Duration = Duration::from_secs(60);
/// A burst of new notifications raises at most this many alerts.
const MAX_ALERTS: usize = 5;
const PLACEHOLDER: f32 = 150.;
const ROW_HEIGHT: f32 = 56.;

/// Everything on Home. It belongs to the account, not to a repository.
#[derive(Default)]
pub struct HomeData {
    pub review_requests: Load<Vec<InboxItem>>,
    pub my_prs: Load<Vec<InboxItem>>,
    pub assigned_issues: Load<Vec<InboxItem>>,
    pub notifications: Load<Vec<Notification>>,
}

/// The notifications seen so far, for deciding which ones are new.
#[derive(Default)]
pub enum Seen {
    /// No poll has finished. The first one only records what already exists, so
    /// opening the app never alerts about old notifications.
    #[default]
    Unpolled,
    /// Keys of the notifications in the latest poll.
    Polled(HashSet<String>),
}
impl Seen {
    /// Record a finished poll and return the keys that were not in the one before.
    pub fn observe(&mut self, keys: impl IntoIterator<Item = String>) -> HashSet<String> {
        let now: HashSet<String> = keys.into_iter().collect();
        match std::mem::replace(self, Seen::Polled(now.clone())) {
            Seen::Unpolled => HashSet::new(),
            Seen::Polled(before) => now.difference(&before).cloned().collect(),
        }
    }
}

/// The macOS alert for a notification as (title, body), or None when it isn't
/// one we alert about. Both come from GitHub and are only ever passed as arguments.
pub fn alert_for(n: &Notification) -> Option<(String, String)> {
    let what = match n.reason.as_str() {
        "review_requested" => "Review requested",
        "ci_activity" => "CI activity",
        _ => return None,
    };
    Some((format!("{what} · {}", n.repository.full_name), n.subject.title.clone()))
}
/// Drop control characters and cap the length of text shown in an alert.
fn clip(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).take(200).collect()
}
/// Post a macOS notification. The text goes in as arguments to a fixed script,
/// never into the script itself, so nothing in it can run as AppleScript.
fn notify_desktop(title: &str, body: &str) {
    let (title, body) = (clip(title), clip(body));
    std::thread::spawn(move || {
        let _ = std::process::Command::new("/usr/bin/osascript")
            .args([
                "-e",
                "on run argv",
                "-e",
                "display notification (item 1 of argv) with title (item 2 of argv)",
                "-e",
                "end run",
                "--",
            ])
            .arg(body)
            .arg(title)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    });
}

fn reason_label(reason: &str) -> &'static str {
    match reason {
        "review_requested" => "Review requested",
        "ci_activity" => "CI",
        "assign" => "Assigned",
        "author" => "Author",
        "comment" => "Comment",
        "mention" => "Mentioned",
        "team_mention" => "Team mentioned",
        "state_change" => "State change",
        "approval_requested" => "Approval requested",
        "security_alert" => "Security",
        "invitation" => "Invitation",
        "subscribed" => "Watching",
        "manual" => "Subscribed",
        _ => "Activity",
    }
}

impl Workspace {
    /// Leave the open tab (it stays open) and show Home.
    pub(super) fn show_home(&mut self, _: &ShowHome, window: &mut Window, cx: &mut Context<Self>) {
        self.active = None;
        self.focus.focus(window);
        cx.notify();
    }
    pub(super) fn unread_count(&self) -> usize {
        self.home.notifications.data.as_ref().map_or(0, Vec::len)
    }

    // ── Data ────────────────────────────────────────────────────────────────

    /// Like `fetch`, for data that isn't tied to a repository. `live` runs on a
    /// successful live answer, never on the cached copy shown meanwhile.
    fn fetch_home<T: Send + 'static>(
        &mut self,
        slot: impl Fn(&mut HomeData) -> &mut Load<T> + 'static,
        force: bool,
        job: impl Fn() -> anyhow::Result<T> + Send + 'static,
        live: impl FnOnce(&mut Self, &T, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) {
        let l = slot(&mut self.home);
        if l.loading || (!force && !l.pending()) {
            return;
        }
        l.loading = true;
        // With nothing to show yet, show the last session's copy first.
        let try_cache = l.data.is_none();
        cx.spawn(async move |this, cx| {
            let mut counted = false;
            let job = if try_cache {
                let (job, cached) = smol::unblock(move || {
                    let r = crate::cache::cached_only(&job);
                    (job, r)
                })
                .await;
                if let Ok(v) = cached {
                    let _ = this.update(cx, |s, cx| {
                        let l = slot(&mut s.home);
                        if l.loading && l.data.is_none() {
                            l.data = Some(v);
                            l.stale = true;
                            s.refreshing += 1;
                            counted = true;
                            cx.notify();
                        }
                    });
                }
                job
            } else {
                job
            };
            let result = smol::unblock(job).await;
            let _ = this.update(cx, |s, cx| {
                let l = slot(&mut s.home);
                l.loading = false;
                l.stale = false;
                if counted {
                    s.refreshing = s.refreshing.saturating_sub(1);
                }
                match result {
                    Ok(v) => {
                        live(s, &v, cx);
                        let l = slot(&mut s.home);
                        l.data = Some(v);
                        l.error = None;
                    }
                    Err(e) => l.error = Some(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }
    /// Request whatever Home shows. With `force`, re-fetch it.
    pub(super) fn ensure_home(&mut self, force: bool, cx: &mut Context<Self>) {
        self.fetch_home(|h| &mut h.review_requests, force, gh::review_requests, |_, _, _| {}, cx);
        self.fetch_home(|h| &mut h.my_prs, force, gh::my_prs, |_, _, _| {}, cx);
        self.fetch_home(|h| &mut h.assigned_issues, force, gh::assigned_issues, |_, _, _| {}, cx);
        self.fetch_notifications(force, cx);
    }
    fn fetch_notifications(&mut self, force: bool, cx: &mut Context<Self>) {
        self.fetch_home(
            |h| &mut h.notifications,
            force,
            gh::notifications,
            |s, list, cx| s.observe_notifications(list, cx),
            cx,
        );
    }
    /// The periodic check, from `tick`.
    pub(super) fn poll_notifications(&mut self, cx: &mut Context<Self>) {
        self.last_notif_poll = Some(Instant::now());
        self.fetch_notifications(true, cx);
    }
    /// A live poll finished: alert about what is new since the one before.
    fn observe_notifications(&mut self, list: &[Notification], cx: &mut Context<Self>) {
        let fresh = self.seen.observe(list.iter().map(Notification::key));
        if !settings::get(cx).desktop_alerts {
            return;
        }
        let alerts = list.iter().filter(|n| fresh.contains(&n.key())).filter_map(alert_for);
        for (title, body) in alerts.take(MAX_ALERTS) {
            notify_desktop(&title, &body);
        }
    }

    // ── Actions ─────────────────────────────────────────────────────────────

    /// Open a repository on what an inbox row or notification points at, adding
    /// the repository to the sidebar first when it isn't there.
    fn open_target(&mut self, slug: &str, target: Target, window: &mut Window, cx: &mut Context<Self>) {
        let Some((owner, name)) = gh::parse_slug(slug) else { return };
        let repo = Repo { owner, name, path: None };
        let key = repo.key();
        if self.repo(&key).is_some() {
            self.open_repo(&key, window, cx);
        } else {
            self.add_repo(repo, window, cx);
        }
        let Some(t) = self.current_mut() else { return };
        match target {
            Target::Pull(n) => {
                t.view = View::Pulls;
                t.pr = Some(n);
                t.line_target = None;
                t.pr_page = PrPage::Conversation;
            }
            Target::Issue(n) => {
                t.view = View::Issues;
                t.issue = Some(n);
            }
            Target::CheckSuite => t.view = View::Actions,
            Target::Release => t.view = View::Releases,
            Target::Repo => {}
        }
        cx.notify();
    }
    fn open_item(&mut self, item: &InboxItem, window: &mut Window, cx: &mut Context<Self>) {
        let target = if item.is_pull_request { Target::Pull(item.number) } else { Target::Issue(item.number) };
        self.open_target(item.slug(), target, window, cx);
    }
    fn open_notification(&mut self, n: &Notification, window: &mut Window, cx: &mut Context<Self>) {
        self.open_target(&n.repository.full_name, n.target(), window, cx);
        self.mark_read(n.id.clone(), cx);
    }
    fn mark_read(&mut self, id: String, cx: &mut Context<Self>) {
        let gone = id.clone();
        self.mutate(
            "Marking as read".into(),
            move || gh::mark_read(&id),
            move |s, cx| {
                if let Some(list) = s.home.notifications.data.as_mut() {
                    list.retain(|n| n.id != gone);
                }
                s.poll_notifications(cx);
            },
            cx,
        );
    }
    pub(super) fn mark_all_read(&mut self, cx: &mut Context<Self>) {
        self.mutate(
            "Marking all notifications as read".into(),
            gh::mark_all_read,
            |s, cx| {
                s.home.notifications.data = Some(vec![]);
                s.poll_notifications(cx);
            },
            cx,
        );
    }

    // ── View ────────────────────────────────────────────────────────────────

    pub(super) fn home_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = palette(cx);
        let header = row()
            .h(px(BAR + 6.))
            .flex_shrink_0()
            .px_3()
            .gap_2()
            .border_b_1()
            .border_color(rgb(colors.row_border))
            .child(icon(IconName::Inbox).text_color(rgb(colors.blue)))
            .child(div().font_weight(FontWeight::MEDIUM).child("Home"));
        let no_notifications = self.home.notifications.data.as_ref().is_none_or(Vec::is_empty);
        let mark_all = button("mark-all-read", "Mark all as read")
            .disabled(no_notifications || self.busy.is_some())
            .on_click(cx.listener(|s, _, _, cx| s.mark_all_read(cx)))
            .into_any_element();
        let body = col()
            .child(self.home_section(
                "Review requested",
                &self.home.review_requests,
                "Loading review requests…",
                (Glyph::PullRequest.into(), "No review requests", "Nobody is waiting on you."),
                None,
                Self::inbox_row,
                cx,
            ))
            .child(self.home_section(
                "Notifications",
                &self.home.notifications,
                "Loading notifications…",
                (IconName::Inbox.into(), "No unread notifications", "You’re all caught up."),
                Some(mark_all),
                Self::notification_row,
                cx,
            ))
            .child(self.home_section(
                "Your pull requests",
                &self.home.my_prs,
                "Loading pull requests…",
                (Glyph::PullRequest.into(), "No open pull requests", "Pull requests you open show up here."),
                None,
                Self::inbox_row,
                cx,
            ))
            .child(self.home_section(
                "Assigned issues",
                &self.home.assigned_issues,
                "Loading issues…",
                (Glyph::CircleDot.into(), "No assigned issues", "Issues assigned to you show up here."),
                None,
                Self::inbox_row,
                cx,
            ));
        col()
            .size_full()
            .bg(rgb(colors.background))
            .child(header)
            .child(div().id("home-scroll").flex_1().min_h_0().overflow_y_scroll().child(body))
            .into_any_element()
    }

    /// A titled list with its count; loading, failure and empty are placeholders.
    #[allow(clippy::too_many_arguments)]
    fn home_section<T>(
        &self,
        title: &'static str,
        load: &Load<Vec<T>>,
        loading: &str,
        (glyph, nothing, detail): (Icon, &'static str, &'static str),
        action: Option<AnyElement>,
        row_for: fn(&Self, &'static str, usize, &T, &mut Context<Self>) -> AnyElement,
        cx: &mut Context<Self>,
    ) -> Div {
        let colors = palette(cx);
        let items = load.data.as_deref();
        let content = match state(colors, Some(load), loading) {
            Err(placeholder) => div().h(px(PLACEHOLDER)).child(placeholder),
            Ok(()) if items.is_none_or(<[T]>::is_empty) => div().h(px(PLACEHOLDER)).child(empty(colors, glyph, nothing, detail)),
            Ok(()) => {
                let mut list = col();
                for (i, item) in items.unwrap_or_default().iter().enumerate() {
                    list = list.child(row_for(self, title, i, item, cx));
                }
                list
            }
        };
        col()
            .child(
                section(colors, title)
                    .px_3()
                    .bg(rgb(colors.chrome))
                    .border_b_1()
                    .border_color(rgb(colors.row_border))
                    .when_some(items.filter(|i| !i.is_empty()), |s, i| s.child(count_badge(colors, i.len())))
                    .child(div().flex_1())
                    .when_some(action, |s, a| s.child(a)),
            )
            .child(content)
    }

    /// The row shape shared with the pull request list: icon, title over details,
    /// and what to know at a glance on the right.
    fn home_row(&self, id: (&'static str, usize), colors: Palette) -> Stateful<Div> {
        row()
            .id(id)
            .w_full()
            .h(px(ROW_HEIGHT))
            .px_3()
            .py_2()
            .gap_3()
            .items_start()
            .cursor_pointer()
            .when(id.1 > 0, |s| s.border_t_1().border_color(rgb(colors.row_border)))
            .hover(|s| s.bg(rgb(colors.hover)))
    }
    fn inbox_row(&self, section: &'static str, i: usize, item: &InboxItem, cx: &mut Context<Self>) -> AnyElement {
        let colors = palette(cx);
        let (glyph, color) = if item.is_pull_request {
            let (glyph, color, _, _) = pr_glyph(colors, "OPEN", item.is_draft);
            (Icon::from(glyph), color)
        } else {
            (Icon::from(Glyph::CircleDot), colors.green)
        };
        let target = item.clone();
        self.home_row((section, i), colors)
            .child(div().pt(px(2.)).child(icon(glyph).size(px(16.)).text_color(rgb(color))))
            .child(
                col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(4.))
                    .child(div().min_w_0().truncate().font_weight(FontWeight::MEDIUM).child(item.title.clone()))
                    .child(
                        row()
                            .w_full()
                            .gap(px(6.))
                            .overflow_hidden()
                            .text_size(px(12.))
                            .text_color(rgb(colors.muted))
                            .child(div().min_w_0().truncate().child(format!("{} #{}", item.slug(), item.number)))
                            .child(div().flex_shrink_0().child(format!(
                                "opened {} by {}",
                                self.when(util::parse_time(&item.created_at), cx),
                                item.author.login
                            ))),
                    ),
            )
            .child(
                row()
                    .flex_shrink_0()
                    .gap_3()
                    .pt(px(2.))
                    .when_some(item.checks, |s, c| s.child(check_icon(colors, c)))
                    .child(self.avatar(Some(&item.author.login), &item.author.login, 20., cx)),
            )
            .on_click(cx.listener(move |s, _, w, cx| s.open_item(&target, w, cx)))
            .into_any_element()
    }
    fn notification_row(&self, section: &'static str, i: usize, n: &Notification, cx: &mut Context<Self>) -> AnyElement {
        let colors = palette(cx);
        let glyph: Icon = match n.subject.kind.as_str() {
            "PullRequest" => Glyph::PullRequest.into(),
            "Issue" => Glyph::CircleDot.into(),
            "CheckSuite" => Glyph::CirclePlay.into(),
            "Release" => Glyph::Tag.into(),
            "Commit" => Glyph::Commit.into(),
            _ => Glyph::Message.into(),
        };
        let (open, read) = (n.clone(), n.id.clone());
        self.home_row((section, i), colors)
            .child(div().pt(px(2.)).child(icon(glyph).size(px(16.)).text_color(rgb(colors.blue))))
            .child(
                col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(4.))
                    .child(div().min_w_0().truncate().font_weight(FontWeight::MEDIUM).child(n.subject.title.clone()))
                    .child(
                        row()
                            .w_full()
                            .gap(px(6.))
                            .overflow_hidden()
                            .text_size(px(12.))
                            .text_color(rgb(colors.muted))
                            .child(pill(reason_label(&n.reason), colors.blue, colors.blue_tint))
                            .child(div().min_w_0().truncate().child(n.repository.full_name.clone()))
                            .child(div().text_color(rgb(colors.quiet)).child("·"))
                            .child(div().flex_shrink_0().child(self.when(util::parse_time(&n.updated_at), cx))),
                    ),
            )
            .child(
                icon_button(("notification-read", i), IconName::Check, "Mark as read").on_click(cx.listener(move |s, _, _, cx| {
                    cx.stop_propagation();
                    s.mark_read(read.clone(), cx);
                })),
            )
            .on_click(cx.listener(move |s, _, w, cx| s.open_notification(&open, w, cx)))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: that glob also brings in gpui's `test` attribute.
    use super::{Seen, alert_for, clip};
    use crate::gh::Notification;
    use std::collections::HashSet;

    fn keys(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }
    fn notification(reason: &str, title: &str) -> Notification {
        serde_json::from_str(&format!(
            r#"{{"id":"1","reason":"{reason}","updated_at":"2026-11-16T10:00:00Z",
               "subject":{{"title":"{title}","url":null,"type":"PullRequest"}},
               "repository":{{"full_name":"acme/hello"}}}}"#
        ))
        .unwrap()
    }

    #[test]
    fn the_first_poll_alerts_about_nothing() {
        let mut seen = Seen::default();
        assert!(seen.observe(keys(&["a@1", "b@1"])).is_empty());
    }
    #[test]
    fn later_polls_report_only_what_is_new() {
        let mut seen = Seen::default();
        seen.observe(keys(&["a@1", "b@1"]));
        let fresh = seen.observe(keys(&["a@1", "b@2", "c@1"]));
        assert_eq!(fresh, HashSet::from(["b@2".to_string(), "c@1".to_string()]));
        // Once reported, they are old news.
        assert!(seen.observe(keys(&["a@1", "b@2", "c@1"])).is_empty());
    }
    #[test]
    fn an_empty_first_poll_still_ends_the_first_poll() {
        let mut seen = Seen::default();
        seen.observe(keys(&[]));
        assert_eq!(seen.observe(keys(&["a@1"])).len(), 1);
    }
    #[test]
    fn alerts_only_for_review_requests_and_ci() {
        assert_eq!(
            alert_for(&notification("review_requested", "Add retry")),
            Some(("Review requested · acme/hello".into(), "Add retry".into()))
        );
        assert!(alert_for(&notification("ci_activity", "CI failed")).is_some());
        assert!(alert_for(&notification("mention", "Hey")).is_none());
    }
    #[test]
    fn alert_text_is_cleaned_before_sending() {
        assert_eq!(clip("a\nb\u{7}c"), "abc");
        assert_eq!(clip(&"x".repeat(500)).len(), 200);
        // Quotes and AppleScript stay as plain text; they are never parsed.
        assert_eq!(clip("\" & (do shell script \"x\") & \""), "\" & (do shell script \"x\") & \"");
    }
}
