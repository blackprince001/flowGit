//! GitHub calls for the compare page: what `head` adds on top of `base`, and
//! creating a pull request from it.
use super::{ApiCommit, Commit, FileDiff, Result, bail, encode, gh, gh_in, json, parse_diff};
use serde::Deserialize;

/// The summary half of `compare/{base}...{head}`: how far apart two branches
/// are, and the commits `head` adds.
#[derive(Clone, Debug, Default)]
pub struct Compare {
    /// "ahead", "behind", "identical" or "diverged".
    pub status: String,
    pub ahead_by: u32,
    pub behind_by: u32,
    /// Every commit in the range; `commits` holds at most the first 250.
    pub total_commits: u32,
    /// Oldest first, as GitHub lists them.
    pub commits: Vec<Commit>,
}

#[derive(Deserialize)]
struct ApiCompare {
    #[serde(default)]
    status: String,
    #[serde(default)]
    ahead_by: u32,
    #[serde(default)]
    behind_by: u32,
    #[serde(default)]
    total_commits: u32,
    #[serde(default)]
    commits: Vec<ApiCommit>,
}

pub fn parse_compare(text: &str) -> Result<Compare> {
    let c: ApiCompare = json(text)?;
    let commits: Vec<Commit> = c
        .commits
        .into_iter()
        .map(|c| Commit {
            sha: c.sha,
            parents: c.parents.into_iter().map(|p| p.sha).collect(),
            author: c.commit.author.name,
            login: c.author.map(|a| a.login).filter(|l| !l.is_empty()),
            time: crate::util::parse_time(&c.commit.author.date),
            subject: c.commit.message.lines().next().unwrap_or("").to_string(),
            refs: vec![],
        })
        .collect();
    Ok(Compare {
        total_commits: c.total_commits.max(commits.len() as u32),
        status: c.status,
        ahead_by: c.ahead_by,
        behind_by: c.behind_by,
        commits,
    })
}

fn compare_url(slug: &str, base: &str, head: &str) -> String {
    format!("repos/{slug}/compare/{}...{}", encode(base), encode(head))
}

/// Ahead/behind and the commits `head` adds to `base`.
pub fn compare(slug: &str, base: &str, head: &str) -> Result<Compare> {
    parse_compare(&gh(&["api", &format!("{}?per_page=100", compare_url(slug, base, head))])?)
}

/// The full diff of `base...head`. The JSON form caps at 300 files; the diff
/// media type does not.
pub fn compare_diff(slug: &str, base: &str, head: &str) -> Result<Vec<FileDiff>> {
    Ok(parse_diff(&gh(&["api", "-H", "Accept: application/vnd.github.diff", &compare_url(slug, base, head)])?))
}

/// The pull request number at the end of the URL `gh pr create` prints.
pub fn parse_created(output: &str) -> Option<u64> {
    output.lines().rev().find_map(|l| l.trim().rsplit_once("/pull/")?.1.trim_end_matches('/').parse().ok())
}

/// Open a pull request and return its number. The body goes through stdin so
/// its length and content never reach a command line.
pub fn create_pr(slug: &str, base: &str, head: &str, title: &str, body: &str, draft: bool) -> Result<u64> {
    let mut args = vec!["pr", "create", "--repo", slug, "--base", base, "--head", head, "--title", title, "--body-file", "-"];
    if draft {
        args.push("--draft");
    }
    let out = gh_in(&args, None, Some(body))?;
    match parse_created(&out) {
        Some(n) => Ok(n),
        None => bail!("Created the pull request, but could not read its number from: {}", out.trim()),
    }
}

/// A title from a branch name: `fix/login-bug` becomes "Fix login bug".
pub fn humanize_branch(name: &str) -> String {
    let words = name.split(['/', '-', '_']).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ");
    let mut chars = words.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// The title and body a new pull request starts with: a lone commit's subject,
/// otherwise the branch name, and the commit subjects as a list.
pub fn prefill(head: &str, commits: &[Commit]) -> (String, String) {
    let title = match commits {
        [only] => only.subject.clone(),
        _ => humanize_branch(head),
    };
    let body = commits.iter().map(|c| format!("- {}", c.subject)).collect::<Vec<_>>().join("\n");
    (title, body)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
        "status": "diverged",
        "ahead_by": 2,
        "behind_by": 5,
        "total_commits": 2,
        "commits": [
            {"sha": "aaa", "parents": [{"sha": "p0"}],
             "author": {"login": "ada"},
             "commit": {"message": "Add parser\n\nLonger text.", "author": {"name": "Ada", "date": "2026-01-02T03:04:05Z"}}},
            {"sha": "bbb", "parents": [{"sha": "aaa"}],
             "author": null,
             "commit": {"message": "Fix typo", "author": {"name": "Bob", "date": "2026-01-03T03:04:05Z"}}}
        ],
        "files": []
    }"#;

    #[test]
    fn parses_compare() {
        let c = parse_compare(SAMPLE).unwrap();
        assert_eq!((c.status.as_str(), c.ahead_by, c.behind_by, c.total_commits), ("diverged", 2, 5, 2));
        assert_eq!(c.commits.len(), 2);
        assert_eq!(c.commits[0].subject, "Add parser");
        assert_eq!(c.commits[0].login.as_deref(), Some("ada"));
        assert_eq!(c.commits[0].parents, vec!["p0".to_string()]);
        assert_eq!(c.commits[1].login, None);
        assert_eq!(c.commits[1].author, "Bob");
        assert!(c.commits[1].time > c.commits[0].time);
    }
    #[test]
    fn identical_branches_have_no_commits() {
        let c = parse_compare(r#"{"status":"identical","ahead_by":0,"behind_by":0,"total_commits":0,"commits":[]}"#).unwrap();
        assert_eq!((c.status.as_str(), c.commits.len()), ("identical", 0));
    }
    #[test]
    fn rejects_garbage() {
        assert!(parse_compare("not json").is_err());
    }
    #[test]
    fn reads_the_number_gh_prints() {
        assert_eq!(parse_created("https://github.com/o/r/pull/42\n"), Some(42));
        assert_eq!(parse_created("Creating pull request...\nhttps://github.com/o/r/pull/7"), Some(7));
        assert_eq!(parse_created("nothing useful"), None);
    }
    #[test]
    fn humanizes_branch_names() {
        assert_eq!(humanize_branch("fix/login-bug"), "Fix login bug");
        assert_eq!(humanize_branch("add_dark--mode"), "Add dark mode");
        assert_eq!(humanize_branch("main"), "Main");
        assert_eq!(humanize_branch(""), "");
    }
    #[test]
    fn prefills_from_commits() {
        let c = |s: &str| Commit {
            sha: String::new(),
            parents: vec![],
            author: String::new(),
            login: None,
            time: 0,
            subject: s.into(),
            refs: vec![],
        };
        assert_eq!(prefill("feat/x", &[c("Add x")]), ("Add x".to_string(), "- Add x".to_string()));
        assert_eq!(
            prefill("feat/new-thing", &[c("One"), c("Two")]),
            ("Feat new thing".to_string(), "- One\n- Two".to_string())
        );
        assert_eq!(prefill("feat/y", &[]), ("Feat y".to_string(), String::new()));
    }
}
