//! The inbox: pull requests and issues that are waiting on you, found with
//! `gh search` across every repository, not just the ones in the sidebar.
use super::{CheckState, Login, gh, json};
use anyhow::Result;
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct InboxRepo {
    pub name_with_owner: String,
}
/// A pull request or issue from a search. Both searches return the same shape.
#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct InboxItem {
    pub number: u64,
    pub title: String,
    #[serde(default)]
    pub author: Login,
    pub repository: InboxRepo,
    /// Only pull request searches return this; issues read as false.
    #[serde(default)]
    pub is_draft: bool,
    #[serde(default)]
    pub is_pull_request: bool,
    pub created_at: String,
    /// CI state of the head commit. Only filled in for your own pull requests.
    #[serde(skip)]
    pub checks: Option<CheckState>,
}
impl InboxItem {
    pub fn slug(&self) -> &str {
        &self.repository.name_with_owner
    }
}

const PR_FIELDS: &str = "number,title,author,repository,isDraft,isPullRequest,createdAt";
const ISSUE_FIELDS: &str = "number,title,author,repository,isPullRequest,createdAt";

fn search(kind: &str, fields: &str, filter: &str) -> Result<Vec<InboxItem>> {
    json(&gh(&[
        "search", kind, filter, "--state=open", "--archived=false", "--sort=updated", "--limit=50", "--json", fields,
    ])?)
}

/// Open pull requests where you, personally, are asked to review.
pub fn review_requests() -> Result<Vec<InboxItem>> {
    search("prs", PR_FIELDS, "--review-requested=@me")
}
/// Your open pull requests, with CI state when GitHub will give it.
pub fn my_prs() -> Result<Vec<InboxItem>> {
    let mut items = search("prs", PR_FIELDS, "--author=@me")?;
    // `gh search` can't return check results. One GraphQL query covers every
    // pull request at once, so it stays a single extra call. It is a nicety:
    // when it fails the list still shows, just without CI.
    if let Ok(states) = my_pr_checks() {
        for item in &mut items {
            item.checks = states.get(&(item.slug().to_lowercase(), item.number)).copied();
        }
    }
    Ok(items)
}
/// Open issues assigned to you.
pub fn assigned_issues() -> Result<Vec<InboxItem>> {
    search("issues", ISSUE_FIELDS, "--assignee=@me")
}

fn my_pr_checks() -> Result<HashMap<(String, u64), CheckState>> {
    let query = "query { search(query: \"is:pr is:open archived:false author:@me sort:updated-desc\", type: ISSUE, first: 50) { nodes { ... on PullRequest { number repository { nameWithOwner } commits(last: 1) { nodes { commit { statusCheckRollup { state } } } } } } } }";
    parse_checks(&gh(&["api", "graphql", "-f", &format!("query={query}")])?)
}

#[derive(Deserialize)]
struct Graph {
    data: GraphData,
}
#[derive(Deserialize)]
struct GraphData {
    search: GraphSearch,
}
#[derive(Deserialize)]
struct GraphSearch {
    nodes: Vec<GraphPull>,
}
/// Everything is optional: a node that isn't a pull request arrives as `{}`.
#[derive(Deserialize, Default)]
#[serde(default)]
struct GraphPull {
    number: Option<u64>,
    repository: Option<InboxRepo>,
    commits: Option<GraphCommits>,
}
#[derive(Deserialize, Default)]
struct GraphCommits {
    nodes: Vec<GraphCommit>,
}
#[derive(Deserialize)]
struct GraphCommit {
    commit: GraphRollup,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphRollup {
    status_check_rollup: Option<GraphState>,
}
#[derive(Deserialize)]
struct GraphState {
    state: String,
}

/// Check state by (lowercased repository, number). Pull requests with no checks
/// are left out.
fn parse_checks(text: &str) -> Result<HashMap<(String, u64), CheckState>> {
    let graph: Graph = json(text)?;
    let mut out = HashMap::new();
    for pull in graph.data.search.nodes {
        let (Some(number), Some(repo)) = (pull.number, pull.repository) else { continue };
        let state = pull
            .commits
            .and_then(|c| c.nodes.into_iter().next())
            .and_then(|c| c.commit.status_check_rollup)
            .map(|s| s.state);
        let state = match state.as_deref() {
            Some("SUCCESS") => CheckState::Pass,
            Some("FAILURE" | "ERROR") => CheckState::Fail,
            Some("PENDING" | "EXPECTED") => CheckState::Pending,
            _ => continue,
        };
        out.insert((repo.name_with_owner.to_lowercase(), number), state);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pull_request_search() {
        let text = r#"[{"author":{"id":"U_1","is_bot":false,"login":"octocat","type":"User","url":"https://github.com/octocat"},"createdAt":"2026-11-10T22:30:47Z","isDraft":true,"isPullRequest":true,"number":381,"repository":{"name":"hello","nameWithOwner":"acme/hello"},"title":"Card rail","updatedAt":"2026-11-16T09:14:19Z"}]"#;
        let items: Vec<InboxItem> = json(text).unwrap();
        assert_eq!(items.len(), 1);
        let i = &items[0];
        assert_eq!((i.number, i.slug(), i.author.login.as_str()), (381, "acme/hello", "octocat"));
        assert!(i.is_draft && i.is_pull_request);
        assert_eq!(i.checks, None);
    }
    #[test]
    fn parses_issue_search_without_draft_flag() {
        let text = r#"[{"author":{"id":"U_2","is_bot":true,"login":"app/dependabot","type":"Bot","url":"https://github.com/apps/dependabot"},"createdAt":"2026-08-14T10:00:40Z","isPullRequest":false,"number":53,"repository":{"name":"srv","nameWithOwner":"acme/srv"},"title":"Add custom domains","updatedAt":"2026-08-16T16:59:54Z"}]"#;
        let items: Vec<InboxItem> = json(text).unwrap();
        assert!(!items[0].is_draft && !items[0].is_pull_request);
        assert_eq!(items[0].author.login, "app/dependabot");
        assert!(json::<Vec<InboxItem>>("[]").unwrap().is_empty());
    }
    #[test]
    fn reads_check_states_from_graphql() {
        let text = r#"{"data":{"search":{"nodes":[
            {"number":113,"repository":{"nameWithOwner":"Acme/Hello"},"commits":{"nodes":[{"commit":{"statusCheckRollup":{"state":"SUCCESS"}}}]}},
            {"number":61,"repository":{"nameWithOwner":"acme/app"},"commits":{"nodes":[{"commit":{"statusCheckRollup":null}}]}},
            {"number":7,"repository":{"nameWithOwner":"acme/app"},"commits":{"nodes":[{"commit":{"statusCheckRollup":{"state":"FAILURE"}}}]}},
            {"number":8,"repository":{"nameWithOwner":"acme/app"},"commits":{"nodes":[{"commit":{"statusCheckRollup":{"state":"PENDING"}}}]}},
            {}
        ]}}}"#;
        let states = parse_checks(text).unwrap();
        assert_eq!(states.len(), 3);
        assert_eq!(states[&("acme/hello".to_string(), 113)], CheckState::Pass);
        assert_eq!(states[&("acme/app".to_string(), 7)], CheckState::Fail);
        assert_eq!(states[&("acme/app".to_string(), 8)], CheckState::Pending);
        assert!(!states.contains_key(&("acme/app".to_string(), 61)));
    }
}
