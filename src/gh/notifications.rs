//! Your GitHub notifications, through `gh api`.
use super::{gh, json};
use anyhow::Result;
use serde::Deserialize;

#[derive(Deserialize, Clone, Debug)]
pub struct Subject {
    pub title: String,
    /// API URL of the pull request or issue; some kinds (check suites) have none.
    #[serde(default)]
    pub url: Option<String>,
    #[serde(rename = "type", default)]
    pub kind: String,
}
#[derive(Deserialize, Clone, Debug)]
pub struct NotificationRepo {
    pub full_name: String,
}
#[derive(Deserialize, Clone, Debug)]
pub struct Notification {
    pub id: String,
    /// Why you got it: `review_requested`, `ci_activity`, `mention`, …
    pub reason: String,
    pub updated_at: String,
    pub subject: Subject,
    pub repository: NotificationRepo,
}

/// What a notification points at, as far as flowGit can open it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Pull(u64),
    Issue(u64),
    CheckSuite,
    Release,
    /// Commits, discussions and anything else: just the repository.
    Repo,
}
impl Notification {
    pub fn target(&self) -> Target {
        let number = || {
            let url = self.subject.url.as_deref()?;
            url.rsplit('/').next()?.parse::<u64>().ok()
        };
        match self.subject.kind.as_str() {
            "PullRequest" => number().map_or(Target::Repo, Target::Pull),
            "Issue" => number().map_or(Target::Repo, Target::Issue),
            "CheckSuite" => Target::CheckSuite,
            "Release" => Target::Release,
            _ => Target::Repo,
        }
    }
    /// Identifies this state of the thread. A thread you already had gets a new
    /// `updated_at` when something new happens, and that counts as new.
    pub fn key(&self) -> String {
        format!("{}@{}", self.id, self.updated_at)
    }
}

/// Unread notifications you take part in (reviews, mentions, your own threads).
pub fn notifications() -> Result<Vec<Notification>> {
    json(&gh(&["api", "notifications?participating=true&per_page=50"])?)
}
pub fn mark_read(thread: &str) -> Result<String> {
    gh(&["api", "--silent", "-X", "PATCH", &format!("notifications/threads/{thread}")])
}
/// Marks everything read, not only what the list shows.
pub fn mark_all_read() -> Result<String> {
    gh(&["api", "--silent", "-X", "PUT", "notifications"])
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"[
      {"id":"1","unread":true,"reason":"review_requested","updated_at":"2026-11-16T10:00:00Z","last_read_at":null,
       "subject":{"title":"Add retry to uploads","url":"https://api.github.com/repos/acme/hello/pulls/12","latest_comment_url":"https://api.github.com/repos/acme/hello/issues/comments/9","type":"PullRequest"},
       "repository":{"id":5,"name":"hello","full_name":"acme/hello","private":true,"owner":{"login":"acme"}},
       "url":"https://api.github.com/notifications/threads/1","subscription_url":"https://api.github.com/notifications/threads/1/subscription"},
      {"id":"2","unread":true,"reason":"ci_activity","updated_at":"2026-11-16T10:05:00Z","last_read_at":null,
       "subject":{"title":"CI failed on main","url":null,"latest_comment_url":null,"type":"CheckSuite"},
       "repository":{"id":5,"name":"hello","full_name":"acme/hello"}},
      {"id":"3","unread":true,"reason":"mention","updated_at":"2026-11-16T10:06:00Z",
       "subject":{"title":"Crash on start","url":"https://api.github.com/repos/acme/hello/issues/40","type":"Issue"},
       "repository":{"full_name":"acme/hello"}},
      {"id":"4","unread":true,"reason":"subscribed","updated_at":"2026-11-16T10:07:00Z",
       "subject":{"title":"Release 1.2","url":"https://api.github.com/repos/acme/hello/releases/77","type":"Release"},
       "repository":{"full_name":"acme/hello"}},
      {"id":"5","unread":true,"reason":"comment","updated_at":"2026-11-16T10:08:00Z",
       "subject":{"title":"A commit","url":"https://api.github.com/repos/acme/hello/commits/abc123","type":"Commit"},
       "repository":{"full_name":"acme/hello"}}
    ]"#;

    #[test]
    fn parses_notifications_and_targets() {
        let list: Vec<Notification> = json(SAMPLE).unwrap();
        assert_eq!(list.len(), 5);
        assert_eq!((list[0].reason.as_str(), list[0].repository.full_name.as_str()), ("review_requested", "acme/hello"));
        assert_eq!(list[0].target(), Target::Pull(12));
        assert_eq!(list[1].target(), Target::CheckSuite);
        assert_eq!(list[2].target(), Target::Issue(40));
        assert_eq!(list[3].target(), Target::Release);
        // A commit's URL ends in a hash, not a number.
        assert_eq!(list[4].target(), Target::Repo);
    }
    #[test]
    fn a_pull_request_without_a_url_falls_back_to_the_repository() {
        let n: Notification = json(
            r#"{"id":"9","reason":"mention","updated_at":"x","subject":{"title":"t","url":null,"type":"PullRequest"},"repository":{"full_name":"a/b"}}"#,
        )
        .unwrap();
        assert_eq!(n.target(), Target::Repo);
    }
    #[test]
    fn a_new_update_changes_the_key() {
        let list: Vec<Notification> = json(SAMPLE).unwrap();
        let mut again = list[0].clone();
        assert_eq!(list[0].key(), again.key());
        again.updated_at = "2026-11-16T11:00:00Z".into();
        assert_ne!(list[0].key(), again.key());
    }
}
