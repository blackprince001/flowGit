//! GitHub calls for the issues feature: the list, one issue, and the actions
//! that change it.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, serde::Serialize, Deserialize)]
pub enum IssueFilter {
    #[default]
    Open,
    Closed,
    All,
}
impl IssueFilter {
    pub const ALL: [IssueFilter; 3] = [IssueFilter::Open, IssueFilter::Closed, IssueFilter::All];
    pub fn arg(self) -> &'static str {
        match self {
            IssueFilter::Open => "open",
            IssueFilter::Closed => "closed",
            IssueFilter::All => "all",
        }
    }
    pub fn title(self) -> &'static str {
        match self {
            IssueFilter::Open => "Open",
            IssueFilter::Closed => "Closed",
            IssueFilter::All => "All",
        }
    }
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Issue {
    pub number: u64,
    pub title: String,
    #[serde(default)]
    pub author: Login,
    #[serde(default)]
    pub labels: Vec<Label>,
    #[serde(default)]
    pub comments: Vec<Comment>,
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
    pub state: String,
    /// "COMPLETED", "NOT_PLANNED" or "REOPENED" once an issue is closed or reopened.
    #[serde(default)]
    pub state_reason: Option<String>,
}
const ISSUE_LIST_FIELDS: &str =
    "number,title,author,labels,comments,createdAt,updatedAt,state,stateReason";
const ISSUE_DETAIL_FIELDS: &str =
    "number,title,body,author,labels,assignees,comments,createdAt,state,stateReason,url";

pub fn issues(slug: &str, filter: IssueFilter) -> Result<Vec<Issue>> {
    json(&gh(&[
        "issue",
        "list",
        "--repo",
        slug,
        "--state",
        filter.arg(),
        "--limit",
        "100",
        "--json",
        ISSUE_LIST_FIELDS,
    ])?)
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct IssueDetail {
    #[serde(flatten)]
    pub summary: Issue,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub assignees: Vec<Login>,
    pub url: String,
}
pub fn issue(slug: &str, number: u64) -> Result<IssueDetail> {
    json(&gh(&[
        "issue",
        "view",
        &number.to_string(),
        "--repo",
        slug,
        "--json",
        ISSUE_DETAIL_FIELDS,
    ])?)
}

/// Mutations on an issue. Each maps to one `gh issue` invocation.
#[derive(Clone, Debug)]
pub enum IssueAction {
    Comment(String),
    Close { not_planned: bool },
    Reopen,
}
pub fn issue_action(slug: &str, number: u64, action: &IssueAction) -> Result<String> {
    let n = number.to_string();
    let base = |verb: &'static str| vec!["issue", verb, n.as_str(), "-R", slug];
    match action {
        IssueAction::Comment(body) => {
            let mut a = base("comment");
            a.extend(["--body-file", "-"]);
            gh_in(&a, None, Some(body))
        }
        IssueAction::Close { not_planned } => {
            let mut a = base("close");
            a.extend([
                "--reason",
                if *not_planned {
                    "not planned"
                } else {
                    "completed"
                },
            ]);
            gh(&a)
        }
        IssueAction::Reopen => gh(&base("reopen")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_issue_list() {
        let text = r#"[
            {"author":{"is_bot":false,"login":"alice"},
             "comments":[{"id":"IC_1","author":{"login":"bob"},"body":"Thanks","createdAt":"2026-11-16T11:40:57Z"}],
             "createdAt":"2026-11-16T11:34:32Z",
             "labels":[{"id":"LA_1","name":"bug","description":"Something is broken","color":"d73a4a"}],
             "number":42,"state":"CLOSED","stateReason":"NOT_PLANNED","title":"Crash on start",
             "updatedAt":"2026-11-16T11:41:00Z"},
            {"author":{"login":"carol"},"comments":[],"createdAt":"2026-11-15T09:00:00Z",
             "labels":[],"number":41,"state":"OPEN","stateReason":null,"title":"Add dark mode",
             "updatedAt":"2026-11-15T09:00:00Z"}
        ]"#;
        let issues: Vec<Issue> = json(text).unwrap();
        assert_eq!(issues.len(), 2);
        assert_eq!(issues[0].number, 42);
        assert_eq!(issues[0].author.login, "alice");
        assert_eq!(issues[0].labels[0].name, "bug");
        assert_eq!(issues[0].comments.len(), 1);
        assert_eq!(issues[0].state_reason.as_deref(), Some("NOT_PLANNED"));
        assert_eq!(issues[1].state, "OPEN");
        assert_eq!(issues[1].state_reason, None);
        assert!(issues[1].comments.is_empty());
    }
    #[test]
    fn parses_issue_detail() {
        let text = r#"{
            "assignees":[{"id":"U_1","login":"dave","name":"Dave"}],
            "author":{"login":"alice"},
            "body":"Steps to reproduce:\n1. Open the app",
            "comments":[{"author":{"login":"bob"},"body":"Me too","createdAt":"2026-11-16T12:00:00Z"}],
            "createdAt":"2026-11-16T11:34:32Z",
            "labels":[],
            "number":42,"state":"OPEN","stateReason":"REOPENED","title":"Crash on start",
            "url":"https://github.com/owner/repo/issues/42"
        }"#;
        let d: IssueDetail = json(text).unwrap();
        assert_eq!(d.summary.number, 42);
        assert_eq!(d.summary.title, "Crash on start");
        assert_eq!(d.summary.comments.len(), 1);
        assert_eq!(d.body, "Steps to reproduce:\n1. Open the app");
        assert_eq!(d.assignees[0].login, "dave");
        assert_eq!(d.url, "https://github.com/owner/repo/issues/42");
    }
    #[test]
    fn filters_map_to_gh_states() {
        assert_eq!(
            IssueFilter::ALL.map(IssueFilter::arg),
            ["open", "closed", "all"]
        );
        assert_eq!(IssueFilter::default(), IssueFilter::Open);
    }
}
