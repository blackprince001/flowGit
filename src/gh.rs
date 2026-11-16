//! Everything GitHub goes through the `gh` CLI, so the app inherits its login,
//! hosts and tokens and never stores a credential. Local clones add `git` for the
//! all-branch graph, fetch and checkout. Calls block; run them off the UI thread.
mod compare;
mod files;
mod inbox;
mod issues;
mod local;
mod notifications;
mod search;
pub use compare::*;
pub use files::*;
pub use inbox::*;
pub use issues::*;
pub use local::*;
pub use notifications::*;
pub use search::*;

use anyhow::{Context as _, Result, anyhow, bail};
use base64::Engine;
use serde::Deserialize;
use std::{
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::RwLock,
};

static GH_OVERRIDE: RwLock<String> = RwLock::new(String::new());

/// Use an explicit executable from preferences; empty restores discovery.
pub fn set_gh_path(path: &str) {
    if let Ok(mut p) = GH_OVERRIDE.write() {
        *p = path.trim().to_string();
    }
}

/// Finder-launched apps get a minimal PATH, so look where installers put `gh`.
pub fn gh_executable() -> PathBuf {
    let explicit = GH_OVERRIDE.read().map(|p| p.clone()).unwrap_or_default();
    if !explicit.is_empty() {
        return PathBuf::from(explicit);
    }
    ["/opt/homebrew/bin/gh", "/usr/local/bin/gh", "/usr/bin/gh"]
        .iter()
        .map(PathBuf::from)
        .find(|p| p.exists())
        .unwrap_or_else(|| PathBuf::from("gh"))
}

fn search_path() -> String {
    let mut parts = vec!["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"]
        .into_iter()
        .map(String::from)
        .collect::<Vec<_>>();
    if let Ok(p) = std::env::var("PATH") {
        parts.push(p);
    }
    parts.join(":")
}

fn run(cmd: Command, stdin: Option<&str>, what: &str) -> Result<String> {
    run_bytes(cmd, stdin, what).map(|b| String::from_utf8_lossy(&b).into_owned())
}
fn run_bytes(mut cmd: Command, stdin: Option<&str>, what: &str) -> Result<Vec<u8>> {
    cmd.env("PATH", search_path())
        .env("GH_PROMPT_DISABLED", "1")
        .env("GH_NO_UPDATE_NOTIFIER", "1")
        .env("GH_PAGER", "cat")
        .env("PAGER", "cat")
        .env("NO_COLOR", "1")
        .env("CLICOLOR", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            anyhow!("{what} was not found. Install it or set its path in Preferences › GitHub.")
        } else {
            anyhow!("Could not start {what}: {e}")
        }
    })?;
    if let (Some(body), Some(mut pipe)) = (stdin, child.stdin.take()) {
        pipe.write_all(body.as_bytes())?;
    }
    let out = child.wait_with_output()?;
    if out.status.success() {
        return Ok(out.stdout);
    }
    let err = String::from_utf8_lossy(&out.stderr);
    let err = err.trim();
    if err.contains("gh auth login") {
        bail!("GitHub CLI is not signed in. Run `gh auth login` in a terminal, then refresh.");
    }
    bail!(
        "{}",
        if err.is_empty() {
            format!("{what} exited with {}", out.status)
        } else {
            err.lines().take(6).collect::<Vec<_>>().join("\n")
        }
    )
}

pub fn gh(args: &[&str]) -> Result<String> {
    gh_in(args, None, None)
}
/// `cwd` matters for commands that act on a clone (checkout). Others run from
/// HOME so a stray repository in the working directory never changes their target.
pub fn gh_in(args: &[&str], cwd: Option<&Path>, stdin: Option<&str>) -> Result<String> {
    let cacheable = cwd.is_none() && stdin.is_none() && crate::cache::cacheable(args);
    if crate::cache::is_cached_only() {
        return cacheable.then(|| crate::cache::read(args)).flatten().ok_or_else(|| anyhow!("Not cached"));
    }
    let mut cmd = Command::new(gh_executable());
    cmd.args(args);
    match cwd {
        Some(dir) => {
            cmd.current_dir(dir);
        }
        None => {
            if let Some(home) = std::env::var_os("HOME") {
                cmd.current_dir(home);
            }
        }
    }
    let out = run(cmd, stdin, "gh")?;
    if cacheable {
        crate::cache::write(args, &out);
    }
    Ok(out)
}
/// A file's raw bytes (e.g. an image a README links by relative path), through
/// `gh` so private repositories work. Not cached on disk.
pub fn raw_file(slug: &str, path: &str, reference: &str) -> Result<Vec<u8>> {
    let mut cmd = Command::new(gh_executable());
    cmd.args(["api", "-H", "Accept: application/vnd.github.raw", &contents_url(slug, path, reference)]);
    if let Some(home) = std::env::var_os("HOME") {
        cmd.current_dir(home);
    }
    run_bytes(cmd, None, "gh")
}
/// The signed-in token, for fetching images GitHub keeps behind sign-in.
pub fn token() -> Option<String> {
    gh(&["auth", "token"]).ok().map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
}
pub fn git(path: &Path, args: &[&str]) -> Result<String> {
    // Local clones answer quickly; the cached pass leaves them to the live one.
    if crate::cache::is_cached_only() {
        return Err(anyhow!("Not cached"));
    }
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(path).args(args);
    run(cmd, None, "git")
}
fn json<T: serde::de::DeserializeOwned>(text: &str) -> Result<T> {
    serde_json::from_str(text).context("Unexpected response from gh")
}
pub fn version() -> Result<String> {
    let v = gh(&["--version"])?;
    Ok(v.split_whitespace().nth(2).unwrap_or("").to_string())
}

// ── Repositories ─────────────────────────────────────────────────────────────

/// Accepts `owner/name`, https and ssh GitHub URLs, with or without `.git`.
pub fn parse_slug(input: &str) -> Option<(String, String)> {
    let s = input.trim().trim_end_matches('/');
    let s = s.strip_suffix(".git").unwrap_or(s);
    let rest = if let Some(i) = s.find("github.com") {
        s[i + "github.com".len()..].trim_start_matches([':', '/'])
    } else if s.contains("://") || s.contains('@') {
        return None;
    } else {
        s
    };
    let mut parts = rest.split('/').filter(|p| !p.is_empty());
    let owner = parts.next()?;
    let name = parts.next()?;
    let valid = |p: &str| {
        p.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
    };
    (valid(owner) && valid(name)).then(|| (owner.to_string(), name.to_string()))
}

#[derive(Deserialize)]
struct Canonical {
    name: String,
    owner: Login,
}
/// Confirm a repository exists and take GitHub's spelling of its name.
pub fn resolve(owner: &str, name: &str) -> Result<(String, String)> {
    let c: Canonical = json(&gh(&[
        "repo",
        "view",
        &format!("{owner}/{name}"),
        "--json",
        "name,owner",
    ])?)?;
    Ok((c.owner.login, c.name))
}

/// The GitHub remote of a local clone, preferring `origin` then `upstream`.
pub fn local_remote(path: &Path) -> Result<(String, String)> {
    git(path, &["rev-parse", "--git-dir"]).map_err(|_| anyhow!("Not a git repository"))?;
    let remotes = git(path, &["remote"])?;
    let mut names = remotes.lines().map(str::trim).collect::<Vec<_>>();
    names.sort_by_key(|n| match *n {
        "origin" => 0,
        "upstream" => 1,
        _ => 2,
    });
    for remote in names {
        if let Ok(url) = git(path, &["remote", "get-url", remote])
            && let Some(slug) = parse_slug(&url)
        {
            return Ok(slug);
        }
    }
    bail!("This repository has no GitHub remote")
}

#[derive(Deserialize, Clone, Default, Debug)]
pub struct Login {
    #[serde(default)]
    pub login: String,
    #[serde(default)]
    pub name: Option<String>,
}
#[derive(Deserialize, Clone, Default, Debug)]
pub struct Named {
    #[serde(default)]
    pub name: String,
}
#[derive(Deserialize, Clone, Default, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Count {
    #[serde(default)]
    pub total_count: u64,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RepoInfo {
    #[serde(default)]
    pub description: Option<String>,
    pub url: String,
    #[serde(default)]
    pub homepage_url: Option<String>,
    #[serde(default)]
    pub stargazer_count: u64,
    #[serde(default)]
    pub fork_count: u64,
    #[serde(default)]
    pub watchers: Count,
    /// Open issues only; `gh repo view` leaves out closed ones and pull requests.
    #[serde(default)]
    pub issues: Count,
    #[serde(default)]
    pub pull_requests: Count,
    #[serde(default)]
    pub primary_language: Option<Named>,
    #[serde(default)]
    pub license_info: Option<Named>,
    #[serde(default)]
    pub default_branch_ref: Option<Named>,
    #[serde(default)]
    pub repository_topics: Option<Vec<Named>>,
    #[serde(default)]
    pub is_private: bool,
    #[serde(default)]
    pub is_fork: bool,
    #[serde(default)]
    pub is_archived: bool,
    #[serde(default)]
    pub pushed_at: Option<String>,
}
impl RepoInfo {
    pub fn default_branch(&self) -> String {
        self.default_branch_ref
            .as_ref()
            .map(|b| b.name.clone())
            .filter(|b| !b.is_empty())
            .unwrap_or_else(|| "main".into())
    }
}
pub fn repo_info(slug: &str) -> Result<RepoInfo> {
    json(&gh(&[
        "repo",
        "view",
        slug,
        "--json",
        "description,url,homepageUrl,stargazerCount,forkCount,watchers,issues,pullRequests,primaryLanguage,licenseInfo,defaultBranchRef,repositoryTopics,isPrivate,isFork,isArchived,pushedAt",
    ])?)
}

#[derive(Deserialize, Clone, Debug)]
pub struct User {
    pub login: String,
    #[serde(default)]
    pub name: Option<String>,
}
pub fn current_user() -> Result<User> {
    json(&gh(&["api", "user"])?)
}

#[derive(Deserialize, Clone, Debug)]
pub struct Sha {
    pub sha: String,
}
#[derive(Deserialize, Clone, Debug)]
pub struct Branch {
    pub name: String,
    pub commit: Sha,
    #[serde(default)]
    pub protected: bool,
}
pub fn branches(slug: &str) -> Result<Vec<Branch>> {
    json(&gh(&["api", &format!("repos/{slug}/branches?per_page=100")])?)
}

#[derive(Deserialize, Clone, Debug)]
pub struct Contributor {
    pub login: String,
    #[serde(default)]
    pub contributions: u64,
}
pub fn contributors(slug: &str) -> Result<Vec<Contributor>> {
    // GitHub answers 204 with an empty body while it computes stats for new repos.
    let text = gh(&["api", &format!("repos/{slug}/contributors?per_page=24")])?;
    if text.trim().is_empty() {
        return Ok(vec![]);
    }
    json(&text)
}

// ── Code ─────────────────────────────────────────────────────────────────────

#[derive(Deserialize, Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub path: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub size: u64,
}
impl Entry {
    pub fn is_dir(&self) -> bool {
        self.kind == "dir"
    }
}
fn contents_url(slug: &str, path: &str, reference: &str) -> String {
    format!(
        "repos/{slug}/contents/{}?ref={}",
        path.trim_matches('/'),
        encode(reference)
    )
}
fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}
/// A directory listing: folders first, then files, each alphabetical.
pub fn tree(slug: &str, path: &str, reference: &str) -> Result<Vec<Entry>> {
    let mut entries: Vec<Entry> = json(&gh(&["api", &contents_url(slug, path, reference)])?)
        .context("That path is a file, not a folder")?;
    entries.sort_by_key(|e| (!e.is_dir(), e.name.to_lowercase()));
    Ok(entries)
}

pub enum Blob {
    Text(String),
    Binary(u64),
    TooLarge(u64),
}
#[derive(Deserialize)]
struct BlobJson {
    #[serde(default)]
    content: String,
    #[serde(default)]
    encoding: String,
    #[serde(default)]
    size: u64,
}
pub fn blob(slug: &str, path: &str, reference: &str) -> Result<Blob> {
    let b: BlobJson = json(&gh(&["api", &contents_url(slug, path, reference)])?)?;
    if b.encoding != "base64" {
        return Ok(Blob::TooLarge(b.size));
    }
    let raw: String = b.content.split_whitespace().collect();
    let bytes = base64::engine::general_purpose::STANDARD.decode(raw)?;
    if bytes.iter().take(8000).any(|b| *b == 0) {
        return Ok(Blob::Binary(b.size));
    }
    Ok(Blob::Text(String::from_utf8_lossy(&bytes).into_owned()))
}

/// A commit as the Code view shows it: headline, short hash and age.
#[derive(Clone, Debug)]
pub struct Brief {
    pub sha: String,
    pub message: String,
    pub time: i64,
}
#[derive(Clone, Debug)]
pub struct Head {
    pub commit: Brief,
    pub author: String,
    pub login: Option<String>,
    /// GitHub's combined check state for the commit, if any checks ran.
    pub checks: Option<CheckState>,
}
#[derive(Clone, Debug)]
pub struct FolderInfo {
    pub head: Option<Head>,
    /// Workflow runs for the head commit, newest first.
    pub runs: Vec<Run>,
    /// Each entry's last commit, by path.
    pub files: std::collections::HashMap<String, Brief>,
}
const HISTORY_ALIASES: usize = 100;
/// The branch head, its checks and runs, and the last commit touching each path,
/// in one GraphQL query (one aliased `history` per path) plus one run listing.
pub fn folder_info(slug: &str, branch: &str, paths: &[String]) -> Result<FolderInfo> {
    let (owner, name) = slug.split_once('/').ok_or_else(|| anyhow!("Bad repository name"))?;
    let mut fields = String::new();
    for (i, p) in paths.iter().take(HISTORY_ALIASES).enumerate() {
        // JSON string escaping is valid GraphQL string syntax.
        let path = serde_json::to_string(p)?;
        fields.push_str(&format!(
            " f{i}: history(first: 1, path: {path}) {{ nodes {{ oid messageHeadline committedDate }} }}"
        ));
    }
    let query = format!(
        "query($owner: String!, $name: String!, $expr: String!) {{ repository(owner: $owner, name: $name) {{ object(expression: $expr) {{ ... on Commit {{ oid messageHeadline committedDate author {{ name user {{ login }} }} statusCheckRollup {{ state }}{fields} }} }} }} }}"
    );
    let text = gh(&[
        "api",
        "graphql",
        "-f",
        &format!("query={query}"),
        "-f",
        &format!("owner={owner}"),
        "-f",
        &format!("name={name}"),
        "-f",
        &format!("expr={branch}"),
    ])?;
    let v: serde_json::Value = json(&text)?;
    let obj = &v["data"]["repository"]["object"];
    let brief = |n: &serde_json::Value| -> Option<Brief> {
        Some(Brief {
            sha: n["oid"].as_str()?.to_string(),
            message: n["messageHeadline"].as_str().unwrap_or("").to_string(),
            time: crate::util::parse_time(n["committedDate"].as_str().unwrap_or("")),
        })
    };
    let head = brief(obj).map(|commit| Head {
        commit,
        author: obj["author"]["name"].as_str().unwrap_or("").to_string(),
        login: obj["author"]["user"]["login"].as_str().map(String::from),
        checks: obj["statusCheckRollup"]["state"].as_str().map(|s| {
            Check {
                state: Some(s.to_string()),
                ..Default::default()
            }
            .outcome()
        }),
    });
    let mut files = std::collections::HashMap::new();
    for (i, p) in paths.iter().take(HISTORY_ALIASES).enumerate() {
        if let Some(b) = obj[format!("f{i}")]["nodes"].get(0).and_then(brief) {
            files.insert(p.clone(), b);
        }
    }
    // Runs are a nicety; a repository without Actions still shows its commits.
    let runs = match &head {
        Some(h) => gh(&[
            "run",
            "list",
            "-R",
            slug,
            "--commit",
            &h.commit.sha,
            "-L",
            "10",
            "--json",
            "databaseId,displayTitle,status,conclusion,workflowName,headBranch,event,createdAt,updatedAt,startedAt,number,url",
        ])
        .ok()
        .and_then(|t| json::<Vec<Run>>(&t).ok())
        .unwrap_or_default(),
        None => vec![],
    };
    Ok(FolderInfo { head, runs, files })
}

/// Who last changed each line of a file.
#[derive(Clone, Debug)]
pub struct BlameCommit {
    pub sha: String,
    pub message: String,
    pub time: i64,
    pub author: String,
}
#[derive(Clone, Debug, Default)]
pub struct Blame {
    pub commits: Vec<BlameCommit>,
    /// Per line (0-based), an index into `commits`.
    pub lines: Vec<Option<usize>>,
}
impl Blame {
    pub fn at(&self, line: usize) -> Option<&BlameCommit> {
        self.lines.get(line).copied().flatten().and_then(|i| self.commits.get(i))
    }
}
pub fn blame(slug: &str, branch: &str, path: &str) -> Result<Blame> {
    let (owner, name) = slug.split_once('/').ok_or_else(|| anyhow!("Bad repository name"))?;
    let query = "query($owner: String!, $name: String!, $expr: String!, $path: String!) { repository(owner: $owner, name: $name) { object(expression: $expr) { ... on Commit { blame(path: $path) { ranges { startingLine endingLine commit { oid messageHeadline committedDate author { name user { login } } } } } } } } }";
    let text = gh(&[
        "api",
        "graphql",
        "-f",
        &format!("query={query}"),
        "-f",
        &format!("owner={owner}"),
        "-f",
        &format!("name={name}"),
        "-f",
        &format!("expr={branch}"),
        "-f",
        &format!("path={path}"),
    ])?;
    let v: serde_json::Value = json(&text)?;
    let ranges = v["data"]["repository"]["object"]["blame"]["ranges"]
        .as_array()
        .ok_or_else(|| anyhow!("No blame for this file"))?;
    let mut out = Blame::default();
    let mut index: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for r in ranges {
        let c = &r["commit"];
        let Some(sha) = c["oid"].as_str() else { continue };
        let i = *index.entry(sha.to_string()).or_insert_with(|| {
            out.commits.push(BlameCommit {
                sha: sha.to_string(),
                message: c["messageHeadline"].as_str().unwrap_or("").to_string(),
                time: crate::util::parse_time(c["committedDate"].as_str().unwrap_or("")),
                author: c["author"]["name"].as_str().unwrap_or("").to_string(),
            });
            out.commits.len() - 1
        });
        let (from, to) = (r["startingLine"].as_u64().unwrap_or(0), r["endingLine"].as_u64().unwrap_or(0));
        for line in from.max(1)..=to {
            let at = line as usize - 1;
            if out.lines.len() <= at {
                out.lines.resize(at + 1, None);
            }
            out.lines[at] = Some(i);
        }
    }
    Ok(out)
}

// ── Commits ──────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefKind {
    Head,
    Branch,
    Remote,
    Tag,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Ref {
    pub name: String,
    pub kind: RefKind,
}
#[derive(Clone, Debug)]
pub struct Commit {
    pub sha: String,
    pub parents: Vec<String>,
    pub author: String,
    pub login: Option<String>,
    pub time: i64,
    pub subject: String,
    pub refs: Vec<Ref>,
}

/// Parse `git log --format=%D` decorations, e.g. `HEAD -> main, origin/main, tag: v1`.
pub fn parse_refs(decorations: &str) -> Vec<Ref> {
    let mut out = vec![];
    for part in decorations.split(", ").map(str::trim).filter(|p| !p.is_empty()) {
        if let Some(branch) = part.strip_prefix("HEAD -> ") {
            out.push(Ref {
                name: branch.into(),
                kind: RefKind::Head,
            });
        } else if part == "HEAD" {
            out.push(Ref {
                name: "HEAD".into(),
                kind: RefKind::Head,
            });
        } else if let Some(tag) = part.strip_prefix("tag: ") {
            out.push(Ref {
                name: tag.into(),
                kind: RefKind::Tag,
            });
        } else if part.ends_with("/HEAD") {
            continue;
        } else if part.contains('/') && !part.starts_with("refs/") {
            out.push(Ref {
                name: part.into(),
                kind: RefKind::Remote,
            });
        } else {
            out.push(Ref {
                name: part.into(),
                kind: RefKind::Branch,
            });
        }
    }
    out
}

/// Every local and remote-tracking branch, newest first, in a graph-friendly order.
pub fn local_log(path: &Path, reference: Option<&str>, limit: u32) -> Result<Vec<Commit>> {
    let n = format!("-n{limit}");
    let mut args = vec![
        "log",
        "--date-order",
        &n,
        "--format=%H%x1f%P%x1f%an%x1f%at%x1f%s%x1f%D%x1e",
    ];
    match reference {
        Some(r) => args.push(r),
        None => args.extend(["--branches", "--remotes", "--tags", "HEAD"]),
    }
    let text = git(path, &args)?;
    Ok(text
        .split('\x1e')
        .filter_map(|rec| {
            let f = rec.trim_start_matches('\n').split('\x1f').collect::<Vec<_>>();
            (f.len() >= 6).then(|| Commit {
                sha: f[0].into(),
                parents: f[1].split_whitespace().map(String::from).collect(),
                author: f[2].into(),
                login: None,
                time: f[3].trim().parse().unwrap_or(0),
                subject: f[4].into(),
                refs: parse_refs(f[5]),
            })
        })
        .collect())
}

#[derive(Deserialize)]
struct ApiCommit {
    sha: String,
    parents: Vec<Sha>,
    #[serde(default)]
    author: Option<Login>,
    commit: ApiCommitBody,
}
#[derive(Deserialize)]
struct ApiCommitBody {
    message: String,
    author: ApiSignature,
}
#[derive(Deserialize)]
struct ApiSignature {
    name: String,
    date: String,
}
/// First-parent history of one branch from the REST API, for repositories without
/// a local clone. Merge commits keep their second parents, so side lanes still show.
pub fn remote_log(slug: &str, reference: &str, limit: u32) -> Result<Vec<Commit>> {
    remote_commits(slug, reference, limit, "")
}
fn remote_commits(slug: &str, reference: &str, limit: u32, extra: &str) -> Result<Vec<Commit>> {
    let mut out = vec![];
    let mut page = 1;
    while (out.len() as u32) < limit {
        let per = 100.min(limit - out.len() as u32);
        let batch: Vec<ApiCommit> = json(&gh(&[
            "api",
            &format!(
                "repos/{slug}/commits?sha={}&per_page={per}&page={page}{extra}",
                encode(reference)
            ),
        ])?)?;
        let done = (batch.len() as u32) < per;
        out.extend(batch.into_iter().map(|c| Commit {
            sha: c.sha,
            parents: c.parents.into_iter().map(|p| p.sha).collect(),
            author: c.commit.author.name,
            login: c.author.map(|a| a.login).filter(|l| !l.is_empty()),
            time: crate::util::parse_time(&c.commit.author.date),
            subject: c.commit.message.lines().next().unwrap_or("").to_string(),
            refs: vec![],
        }));
        if done {
            break;
        }
        page += 1;
    }
    Ok(out)
}

pub const ACTIVITY_DAYS: usize = 30;
/// Commits per local calendar day for the last 30 days (oldest first), newest
/// commit first within a day.
pub struct Activity {
    pub days: Vec<(i64, Vec<Commit>)>,
}
pub fn activity(slug: &str, branch: &str) -> Result<Activity> {
    use chrono::{Duration, Local, TimeZone};
    let today = Local::now().date_naive();
    let first = today - Duration::days(ACTIVITY_DAYS as i64 - 1);
    let start = Local
        .from_local_datetime(&first.and_hms_opt(0, 0, 0).unwrap())
        .earliest()
        .ok_or_else(|| anyhow!("Bad local date"))?;
    let since = start.with_timezone(&chrono::Utc).format("%Y-%m-%dT%H:%M:%SZ");
    let commits = remote_commits(slug, branch, 1000, &format!("&since={since}"))?;
    let mut days = (0..ACTIVITY_DAYS)
        .map(|i| {
            let d = first + Duration::days(i as i64);
            let ts = Local
                .from_local_datetime(&d.and_hms_opt(12, 0, 0).unwrap())
                .earliest()
                .map(|t| t.timestamp())
                .unwrap_or(0);
            (ts, vec![])
        })
        .collect::<Vec<(i64, Vec<Commit>)>>();
    for c in commits {
        let Some(t) = Local.timestamp_opt(c.time, 0).single() else { continue };
        let i = (t.date_naive() - first).num_days();
        if (0..ACTIVITY_DAYS as i64).contains(&i) {
            days[i as usize].1.push(c);
        }
    }
    Ok(Activity { days })
}
/// The URL a local clone fetches from: `origin`, else its first remote.
pub fn origin_url(path: &Path) -> Result<String> {
    if let Ok(url) = git(path, &["remote", "get-url", "origin"]) {
        return Ok(url.trim().to_string());
    }
    let first = git(path, &["remote"])?;
    let name = first.lines().next().ok_or_else(|| anyhow!("No remotes"))?.to_string();
    Ok(git(path, &["remote", "get-url", &name])?.trim().to_string())
}

pub struct CommitDetail {
    pub message: String,
    pub files: Vec<FileDiff>,
}
pub fn commit_detail(slug: &str, path: Option<&Path>, sha: &str) -> Result<CommitDetail> {
    if let Some(path) = path
        && let Ok(message) = git(path, &["log", "-1", "--format=%B", sha])
    {
        let diff = git(path, &["show", "--format=", "--patch", "-M", "--no-color", sha])?;
        return Ok(CommitDetail {
            message: message.trim_end().to_string(),
            files: parse_diff(&diff),
        });
    }
    #[derive(Deserialize)]
    struct Body {
        commit: ApiCommitBody,
    }
    let body: Body = json(&gh(&["api", &format!("repos/{slug}/commits/{sha}")])?)?;
    let diff = gh(&[
        "api",
        "-H",
        "Accept: application/vnd.github.diff",
        &format!("repos/{slug}/commits/{sha}"),
    ])?;
    Ok(CommitDetail {
        message: body.commit.message.trim_end().to_string(),
        files: parse_diff(&diff),
    })
}
pub fn fetch(path: &Path) -> Result<()> {
    git(path, &["fetch", "--all", "--prune", "--tags"]).map(|_| ())
}

// ── Diffs ────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    Hunk,
    Context,
    Add,
    Del,
}
#[derive(Clone, Debug)]
pub struct DiffLine {
    pub kind: LineKind,
    pub old: Option<u32>,
    pub new: Option<u32>,
    pub text: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileStatus {
    Added,
    Deleted,
    Modified,
    Renamed,
}
#[derive(Clone, Debug)]
pub struct FileDiff {
    pub path: String,
    pub old_path: Option<String>,
    pub status: FileStatus,
    pub additions: u32,
    pub deletions: u32,
    pub binary: bool,
    pub lines: Vec<DiffLine>,
}

/// Parse `git diff`-style unified output into files and numbered lines.
pub fn parse_diff(text: &str) -> Vec<FileDiff> {
    let mut files: Vec<FileDiff> = vec![];
    let (mut old, mut new) = (0u32, 0u32);
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            // `a/x b/x`; the real names come from ---/+++ or rename lines when present.
            let path = rest
                .rsplit_once(" b/")
                .map(|(_, b)| b.to_string())
                .unwrap_or_else(|| rest.to_string());
            files.push(FileDiff {
                path,
                old_path: None,
                status: FileStatus::Modified,
                additions: 0,
                deletions: 0,
                binary: false,
                lines: vec![],
            });
            continue;
        }
        let Some(f) = files.last_mut() else { continue };
        if f.lines.is_empty() {
            if line.starts_with("new file mode") {
                f.status = FileStatus::Added;
                continue;
            } else if line.starts_with("deleted file mode") {
                f.status = FileStatus::Deleted;
                continue;
            } else if let Some(from) = line.strip_prefix("rename from ") {
                f.status = FileStatus::Renamed;
                f.old_path = Some(from.into());
                continue;
            } else if let Some(to) = line.strip_prefix("rename to ") {
                f.path = to.into();
                continue;
            } else if line.starts_with("Binary files") || line.starts_with("GIT binary patch") {
                f.binary = true;
                continue;
            } else if line.starts_with("--- ") || line.starts_with("+++ ") {
                if let Some(p) = line.strip_prefix("+++ b/") {
                    f.path = p.into();
                }
                continue;
            } else if !line.starts_with("@@") {
                continue;
            }
        }
        if let Some(h) = line.strip_prefix("@@ ") {
            // @@ -a,b +c,d @@ context
            let mut nums = h.split_whitespace();
            let parse = |s: Option<&str>| {
                s.and_then(|s| s[1..].split(',').next())
                    .and_then(|n| n.parse::<u32>().ok())
                    .unwrap_or(1)
            };
            old = parse(nums.next());
            new = parse(nums.next());
            f.lines.push(DiffLine {
                kind: LineKind::Hunk,
                old: None,
                new: None,
                text: line.to_string(),
            });
        } else if let Some(t) = line.strip_prefix('+') {
            f.additions += 1;
            f.lines.push(DiffLine {
                kind: LineKind::Add,
                old: None,
                new: Some(new),
                text: t.to_string(),
            });
            new += 1;
        } else if let Some(t) = line.strip_prefix('-') {
            f.deletions += 1;
            f.lines.push(DiffLine {
                kind: LineKind::Del,
                old: Some(old),
                new: None,
                text: t.to_string(),
            });
            old += 1;
        } else if line.starts_with('\\') {
            continue;
        } else {
            f.lines.push(DiffLine {
                kind: LineKind::Context,
                old: Some(old),
                new: Some(new),
                text: line.strip_prefix(' ').unwrap_or(line).to_string(),
            });
            old += 1;
            new += 1;
        }
    }
    files
}

// ── Pull requests ────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, serde::Serialize, Deserialize)]
pub enum PrFilter {
    #[default]
    Open,
    Merged,
    Closed,
    All,
}
impl PrFilter {
    pub const ALL: [PrFilter; 4] = [
        PrFilter::Open,
        PrFilter::Merged,
        PrFilter::Closed,
        PrFilter::All,
    ];
    pub fn arg(self) -> &'static str {
        match self {
            PrFilter::Open => "open",
            PrFilter::Merged => "merged",
            PrFilter::Closed => "closed",
            PrFilter::All => "all",
        }
    }
    pub fn title(self) -> &'static str {
        match self {
            PrFilter::Open => "Open",
            PrFilter::Merged => "Merged",
            PrFilter::Closed => "Closed",
            PrFilter::All => "All",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckState {
    Pass,
    Fail,
    Pending,
    Skipped,
}
#[derive(Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub context: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub conclusion: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub workflow_name: Option<String>,
    #[serde(default)]
    pub details_url: Option<String>,
    #[serde(default)]
    pub target_url: Option<String>,
}
impl Check {
    pub fn title(&self) -> String {
        self.name
            .clone()
            .or_else(|| self.context.clone())
            .unwrap_or_else(|| "Check".into())
    }
    pub fn url(&self) -> Option<String> {
        self.details_url.clone().or_else(|| self.target_url.clone())
    }
    pub fn outcome(&self) -> CheckState {
        // CheckRun has status/conclusion; StatusContext has state.
        let s = self
            .conclusion
            .as_deref()
            .filter(|c| !c.is_empty())
            .or(self.state.as_deref())
            .unwrap_or("")
            .to_ascii_uppercase();
        let running = self
            .status
            .as_deref()
            .is_some_and(|s| !s.eq_ignore_ascii_case("COMPLETED"));
        match s.as_str() {
            _ if running && self.state.is_none() => CheckState::Pending,
            "SUCCESS" => CheckState::Pass,
            "FAILURE" | "ERROR" | "TIMED_OUT" | "STARTUP_FAILURE" | "ACTION_REQUIRED" => {
                CheckState::Fail
            }
            "PENDING" | "EXPECTED" | "QUEUED" | "IN_PROGRESS" | "" => CheckState::Pending,
            _ => CheckState::Skipped,
        }
    }
}
/// One state for a whole rollup: any failure fails, then anything pending.
pub fn rollup(checks: &[Check]) -> Option<CheckState> {
    if checks.is_empty() {
        return None;
    }
    let states = checks.iter().map(Check::outcome).collect::<Vec<_>>();
    Some(if states.contains(&CheckState::Fail) {
        CheckState::Fail
    } else if states.contains(&CheckState::Pending) {
        CheckState::Pending
    } else if states.contains(&CheckState::Pass) {
        CheckState::Pass
    } else {
        CheckState::Skipped
    })
}

#[derive(Deserialize, Clone, Debug)]
pub struct Label {
    pub name: String,
    #[serde(default)]
    pub color: String,
}
#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Pull {
    pub number: u64,
    pub title: String,
    #[serde(default)]
    pub author: Login,
    pub head_ref_name: String,
    pub base_ref_name: String,
    #[serde(default)]
    pub is_draft: bool,
    pub state: String,
    pub created_at: String,
    #[serde(default)]
    pub review_decision: Option<String>,
    #[serde(default)]
    pub additions: u64,
    #[serde(default)]
    pub deletions: u64,
    #[serde(default)]
    pub labels: Vec<Label>,
    #[serde(default)]
    pub status_check_rollup: Vec<Check>,
}
const PULL_FIELDS: &str = "number,title,author,headRefName,baseRefName,isDraft,state,createdAt,updatedAt,reviewDecision,additions,deletions,labels,statusCheckRollup";
/// Check rollups are expensive for GitHub to compute in bulk (large repositories
/// time out), and only matter while a pull request is open.
pub fn pulls(slug: &str, filter: PrFilter) -> Result<Vec<Pull>> {
    let fields = if filter == PrFilter::Open {
        PULL_FIELDS.to_string()
    } else {
        PULL_FIELDS.replace(",statusCheckRollup", "")
    };
    json(&gh(&[
        "pr", "list", "-R", slug, "--state", filter.arg(), "-L", "50", "--json", &fields,
    ])?)
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Comment {
    #[serde(default)]
    pub author: Login,
    #[serde(default)]
    pub body: String,
    pub created_at: String,
}
#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Review {
    #[serde(default)]
    pub author: Login,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub submitted_at: Option<String>,
}
#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PrCommit {
    pub oid: String,
    pub message_headline: String,
    #[serde(default)]
    pub authored_date: String,
    #[serde(default)]
    pub authors: Vec<Login>,
}
#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PullDetail {
    #[serde(flatten)]
    pub summary: Pull,
    #[serde(default)]
    pub body: String,
    pub url: String,
    #[serde(default)]
    pub merged_at: Option<String>,
    #[serde(default)]
    pub closed_at: Option<String>,
    #[serde(default)]
    pub changed_files: u64,
    #[serde(default)]
    pub comments: Vec<Comment>,
    #[serde(default)]
    pub reviews: Vec<Review>,
    #[serde(default)]
    pub commits: Vec<PrCommit>,
    #[serde(default)]
    pub mergeable: String,
    #[serde(default)]
    pub merge_state_status: String,
    /// The head commit, which new review comments attach to.
    #[serde(default)]
    pub head_ref_oid: String,
    /// Inline code-review comments, grouped into threads (filled separately).
    #[serde(skip)]
    pub threads: Vec<Thread>,
}

/// An inline code-review comment.
#[derive(Deserialize, Clone, Debug)]
pub struct ReviewComment {
    pub id: u64,
    #[serde(default)]
    pub in_reply_to_id: Option<u64>,
    #[serde(default)]
    pub user: Option<Login>,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub line: Option<u64>,
    #[serde(default)]
    pub original_line: Option<u64>,
    /// "RIGHT" (new code) or "LEFT" (removed code).
    #[serde(default)]
    pub side: Option<String>,
    /// The first line and side of a multi-line comment.
    #[serde(default)]
    pub start_line: Option<u64>,
    #[serde(default)]
    pub start_side: Option<String>,
    #[serde(default)]
    pub created_at: String,
}
impl ReviewComment {
    pub fn login(&self) -> String {
        self.user.as_ref().map(|u| u.login.clone()).unwrap_or_default()
    }
}
/// A review comment and its replies, oldest first.
#[derive(Clone, Debug)]
pub struct Thread {
    pub root: ReviewComment,
    pub replies: Vec<ReviewComment>,
}
/// Group review comments by the comment they reply to.
pub fn threads(comments: Vec<ReviewComment>) -> Vec<Thread> {
    let mut out: Vec<Thread> = vec![];
    let mut sorted = comments;
    sorted.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    for c in sorted {
        match c.in_reply_to_id.and_then(|parent| out.iter_mut().find(|t| t.root.id == parent)) {
            Some(thread) => thread.replies.push(c),
            None => out.push(Thread {
                root: c,
                replies: vec![],
            }),
        }
    }
    out
}
pub fn pull(slug: &str, number: u64) -> Result<PullDetail> {
    let mut detail: PullDetail = json(&gh(&[
        "pr",
        "view",
        &number.to_string(),
        "-R",
        slug,
        "--json",
        &format!(
            "{PULL_FIELDS},body,url,mergedAt,closedAt,changedFiles,comments,reviews,commits,mergeable,mergeStateStatus,headRefOid"
        ),
    ])?)?;
    // Review threads are optional: a failure here still shows the conversation.
    if let Ok(text) = gh(&["api", &format!("repos/{slug}/pulls/{number}/comments?per_page=100")])
        && let Ok(comments) = json::<Vec<ReviewComment>>(&text)
    {
        detail.threads = threads(comments);
    }
    Ok(detail)
}
pub fn pull_diff(slug: &str, number: u64) -> Result<Vec<FileDiff>> {
    Ok(parse_diff(&gh(&[
        "pr",
        "diff",
        &number.to_string(),
        "-R",
        slug,
        "--color",
        "never",
    ])?))
}

/// Mutations on a pull request. Each maps to one `gh pr` invocation.
#[derive(Clone, Debug)]
pub enum PrAction {
    Comment(String),
    Approve,
    RequestChanges(String),
    Merge {
        method: crate::settings::MergeMethod,
        delete_branch: bool,
    },
    Close,
    Reopen,
    Ready,
    Draft,
    Checkout(PathBuf),
    /// Start a review thread on one line of the diff.
    LineComment {
        commit: String,
        /// The first line and side of a multi-line comment.
        start: Option<(u32, bool)>,
        path: String,
        line: u32,
        right: bool,
        body: String,
    },
    /// Answer an existing review thread.
    Reply { to: u64, body: String },
    /// Post a review: a summary, a verdict and every pending line comment in one call.
    SubmitReview {
        commit: String,
        event: ReviewEvent,
        body: String,
        comments: Vec<DraftComment>,
    },
}

/// What a review says about the pull request as a whole.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewEvent {
    Comment,
    Approve,
    RequestChanges,
}
impl ReviewEvent {
    pub const ALL: [Self; 3] = [Self::Comment, Self::Approve, Self::RequestChanges];
    /// The value GitHub's reviews endpoint expects.
    pub fn api(self) -> &'static str {
        match self {
            Self::Comment => "COMMENT",
            Self::Approve => "APPROVE",
            Self::RequestChanges => "REQUEST_CHANGES",
        }
    }
    pub fn title(self) -> &'static str {
        match self {
            Self::Comment => "Comment",
            Self::Approve => "Approve",
            Self::RequestChanges => "Request changes",
        }
    }
}

/// A line comment waiting for its review to be sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DraftComment {
    pub path: String,
    pub line: u32,
    /// The new (right) side for additions and context, old (left) for deletions.
    pub right: bool,
    /// The first line and side of a multi-line comment.
    pub start: Option<(u32, bool)>,
    pub body: String,
}

/// The body of `POST pulls/{n}/reviews`.
pub fn review_payload(commit: &str, event: ReviewEvent, body: &str, comments: &[DraftComment]) -> serde_json::Value {
    let side = |right: bool| if right { "RIGHT" } else { "LEFT" };
    let comments: Vec<serde_json::Value> = comments
        .iter()
        .map(|c| {
            let mut v = serde_json::json!({
                "path": c.path,
                "line": c.line,
                "side": side(c.right),
                "body": c.body,
            });
            if let Some((first, first_right)) = c.start {
                v["start_line"] = first.into();
                v["start_side"] = side(first_right).into();
            }
            v
        })
        .collect();
    serde_json::json!({
        "commit_id": commit,
        "event": event.api(),
        "body": body,
        "comments": comments,
    })
}
pub fn pr_action(slug: &str, number: u64, action: &PrAction) -> Result<String> {
    let n = number.to_string();
    let base = |verb: &'static str| vec!["pr", verb, n.as_str(), "-R", slug];
    match action {
        PrAction::Comment(body) => {
            let mut a = base("comment");
            a.extend(["--body-file", "-"]);
            gh_in(&a, None, Some(body))
        }
        PrAction::Approve => {
            let mut a = base("review");
            a.push("--approve");
            gh(&a)
        }
        PrAction::RequestChanges(body) => {
            let mut a = base("review");
            a.extend(["--request-changes", "--body-file", "-"]);
            gh_in(&a, None, Some(body))
        }
        PrAction::Merge {
            method,
            delete_branch,
        } => {
            let mut a = base("merge");
            a.push(method.flag());
            if *delete_branch {
                a.push("--delete-branch");
            }
            gh(&a)
        }
        PrAction::Close => gh(&base("close")),
        PrAction::Reopen => gh(&base("reopen")),
        PrAction::Ready => gh(&base("ready")),
        PrAction::Draft => {
            let mut a = base("ready");
            a.push("--undo");
            gh(&a)
        }
        PrAction::Checkout(path) => gh_in(&["pr", "checkout", &n], Some(path), None),
        PrAction::LineComment {
            commit,
            start,
            path,
            line,
            right,
            body,
        } => {
            let side = |right: bool| if right { "RIGHT" } else { "LEFT" };
            let mut input = serde_json::json!({
                "body": body,
                "commit_id": commit,
                "path": path,
                "line": line,
                "side": side(*right),
            });
            if let Some((first, first_right)) = start {
                input["start_line"] = (*first).into();
                input["start_side"] = side(*first_right).into();
            }
            let endpoint = format!("repos/{slug}/pulls/{number}/comments");
            gh_in(&["api", "-X", "POST", &endpoint, "--input", "-"], None, Some(&input.to_string())).map(|_| "Comment added.".into())
        }
        PrAction::Reply { to, body } => {
            let input = serde_json::json!({ "body": body });
            let endpoint = format!("repos/{slug}/pulls/{number}/comments/{to}/replies");
            gh_in(&["api", "-X", "POST", &endpoint, "--input", "-"], None, Some(&input.to_string())).map(|_| "Reply added.".into())
        }
        PrAction::SubmitReview {
            commit,
            event,
            body,
            comments,
        } => {
            let input = review_payload(commit, *event, body, comments);
            let endpoint = format!("repos/{slug}/pulls/{number}/reviews");
            gh_in(&["api", "-X", "POST", &endpoint, "--input", "-"], None, Some(&input.to_string())).map(|_| "Review submitted.".into())
        }
    }
}

// ── Actions ──────────────────────────────────────────────────────────────────

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    pub database_id: u64,
    #[serde(default)]
    pub display_title: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub conclusion: String,
    #[serde(default)]
    pub workflow_name: String,
    #[serde(default)]
    pub head_branch: String,
    #[serde(default)]
    pub event: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub number: u64,
    #[serde(default)]
    pub url: String,
}
impl Run {
    pub fn active(&self) -> bool {
        !self.status.eq_ignore_ascii_case("completed")
    }
}
pub fn runs(slug: &str, workflow: Option<u64>) -> Result<Vec<Run>> {
    let w = workflow.map(|w| w.to_string());
    let mut args = vec![
        "run",
        "list",
        "-R",
        slug,
        "-L",
        "60",
        "--json",
        "databaseId,displayTitle,status,conclusion,workflowName,headBranch,event,createdAt,updatedAt,startedAt,number,url",
    ];
    if let Some(w) = &w {
        args.extend(["--workflow", w.as_str()]);
    }
    json(&gh(&args)?)
}

/// How many recent runs are queued or in progress, for the sidebar indicator.
pub fn active_runs(slug: &str) -> Result<usize> {
    #[derive(Deserialize)]
    struct Status {
        #[serde(default)]
        status: String,
    }
    let runs: Vec<Status> = json(&gh(&["run", "list", "-R", slug, "-L", "20", "--json", "status"])?)?;
    Ok(runs.iter().filter(|r| !r.status.eq_ignore_ascii_case("completed")).count())
}

#[derive(Deserialize, Clone, Debug)]
pub struct Workflow {
    pub id: u64,
    pub name: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub path: String,
    /// Declares a `workflow_dispatch` trigger, so it can be run by hand (filled separately).
    #[serde(skip)]
    pub dispatchable: bool,
}
/// Every workflow, with whether it can be started manually. GitHub only offers
/// "Run workflow" for files on the default branch that declare
/// `workflow_dispatch`, so each active file is read (in parallel) and checked.
pub fn workflows(slug: &str) -> Result<Vec<Workflow>> {
    let mut list: Vec<Workflow> = json(&gh(&[
        "workflow", "list", "-R", slug, "--all", "-L", "100", "--json", "id,name,state,path",
    ])?)?;
    let cached = crate::cache::is_cached_only();
    std::thread::scope(|scope| {
        let checks = list
            .iter()
            .map(|w| {
                // Dynamic workflows (Dependabot, Pages, Copilot…) have no file to dispatch.
                let readable = w.state == "active" && w.path.starts_with(".github/workflows/");
                let path = w.path.clone();
                scope.spawn(move || {
                    crate::cache::inherit(cached, || {
                        readable
                            && gh(&[
                            "api",
                            "-H",
                            "Accept: application/vnd.github.raw",
                            &format!("repos/{slug}/contents/{path}"),
                        ])
                        .is_ok_and(|yaml| declares_dispatch(&yaml))
                    })
                })
            })
            .collect::<Vec<_>>();
        for (w, check) in list.iter_mut().zip(checks) {
            w.dispatchable = check.join().unwrap_or(false);
        }
    });
    Ok(list)
}
/// Whether a workflow file lists `workflow_dispatch` among its triggers, in any
/// of YAML's forms (`on: workflow_dispatch`, a list, or a mapping key).
/// Comments are ignored.
pub fn declares_dispatch(yaml: &str) -> bool {
    yaml.lines().any(|line| {
        let code = line.split('#').next().unwrap_or("");
        code.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .any(|word| word == "workflow_dispatch")
    })
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    pub name: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub conclusion: String,
    #[serde(default)]
    pub number: u64,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub completed_at: Option<String>,
}
#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub database_id: u64,
    pub name: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub conclusion: String,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub completed_at: Option<String>,
    #[serde(default)]
    pub steps: Vec<Step>,
}
pub fn jobs(slug: &str, run: u64) -> Result<Vec<Job>> {
    #[derive(Deserialize)]
    struct Jobs {
        jobs: Vec<Job>,
    }
    let j: Jobs = json(&gh(&[
        "run",
        "view",
        &run.to_string(),
        "-R",
        slug,
        "--json",
        "jobs",
    ])?)?;
    Ok(j.jobs)
}

#[derive(Clone, Debug)]
pub struct LogLine {
    pub step: String,
    pub text: String,
}
pub const LOG_LIMIT: usize = 20_000;
/// A job's log as `job \t step \t timestamp text` lines, ANSI stripped. Long logs
/// keep their tail, where failures usually are.
pub fn job_log(slug: &str, job: u64) -> Result<Vec<LogLine>> {
    let text = gh(&["run", "view", "-R", slug, "--job", &job.to_string(), "--log"])?;
    let mut lines = text
        .lines()
        .map(|l| {
            let mut parts = l.splitn(3, '\t');
            let _job = parts.next();
            let step = parts.next().unwrap_or("").to_string();
            let rest = parts.next().unwrap_or(l);
            // Drop the ISO timestamp GitHub prefixes to every line.
            let rest = match rest.split_once(' ') {
                Some((ts, tail)) if ts.len() >= 20 && ts.as_bytes()[4] == b'-' => tail,
                _ => rest,
            };
            LogLine {
                step,
                text: crate::util::strip_ansi(rest),
            }
        })
        .collect::<Vec<_>>();
    if lines.len() > LOG_LIMIT {
        lines.drain(..lines.len() - LOG_LIMIT);
    }
    Ok(lines)
}

#[derive(Clone, Debug)]
pub enum RunAction {
    Rerun,
    RerunFailed,
    Cancel,
    Dispatch { workflow: u64, reference: String },
}
pub fn run_action(slug: &str, run: u64, action: &RunAction) -> Result<String> {
    let id = run.to_string();
    match action {
        RunAction::Rerun => gh(&["run", "rerun", &id, "-R", slug]),
        RunAction::RerunFailed => gh(&["run", "rerun", &id, "--failed", "-R", slug]),
        RunAction::Cancel => gh(&["run", "cancel", &id, "-R", slug]),
        RunAction::Dispatch {
            workflow,
            reference,
        } => gh(&[
            "workflow",
            "run",
            &workflow.to_string(),
            "--ref",
            reference,
            "-R",
            slug,
        ]),
    }
}

// ── Releases ─────────────────────────────────────────────────────────────────

#[derive(Deserialize, Clone, Debug)]
pub struct Asset {
    pub name: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub download_count: u64,
    #[serde(default)]
    pub browser_download_url: String,
}
#[derive(Deserialize, Clone, Debug)]
pub struct Release {
    pub tag_name: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub published_at: Option<String>,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub html_url: String,
    #[serde(default)]
    pub author: Option<Login>,
    #[serde(default)]
    pub assets: Vec<Asset>,
}
impl Release {
    pub fn title(&self) -> String {
        self.name
            .clone()
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| self.tag_name.clone())
    }
    pub fn time(&self) -> i64 {
        crate::util::parse_time(self.published_at.as_deref().unwrap_or(&self.created_at))
    }
}
pub fn releases(slug: &str) -> Result<Vec<Release>> {
    json(&gh(&["api", &format!("repos/{slug}/releases?per_page=30")])?)
}

/// Release notes GitHub writes for a tag, from the pull requests merged since the
/// previous release.
#[derive(Deserialize, Clone, Debug, Default)]
pub struct GeneratedNotes {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub body: String,
}
pub fn generate_notes(slug: &str, tag: &str, branch: &str) -> Result<GeneratedNotes> {
    json(&gh(&[
        "api",
        &format!("repos/{slug}/releases/generate-notes"),
        "-f",
        &format!("tag_name={tag}"),
        "-f",
        &format!("target_commitish={branch}"),
    ])?)
}
/// Create a draft release. The notes go in on stdin, so they need no quoting.
pub fn create_draft(slug: &str, tag: &str, branch: &str, title: &str, notes: &str) -> Result<String> {
    gh_in(
        &[
            "release",
            "create",
            tag,
            "--repo",
            slug,
            "--draft",
            "--target",
            branch,
            "--title",
            title,
            "--notes-file",
            "-",
        ],
        None,
        Some(notes),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_repository_slugs() {
        let s = |a: &str, b: &str| Some((a.to_string(), b.to_string()));
        assert_eq!(parse_slug("cli/cli"), s("cli", "cli"));
        assert_eq!(parse_slug("https://github.com/zed-industries/zed"), s("zed-industries", "zed"));
        assert_eq!(parse_slug("https://github.com/a/b.git/"), s("a", "b"));
        assert_eq!(parse_slug("git@github.com:owner/repo.git"), s("owner", "repo"));
        assert_eq!(parse_slug("ssh://git@github.com/o/r"), s("o", "r"));
        assert_eq!(parse_slug("https://gitlab.com/a/b"), None);
        assert_eq!(parse_slug("nonsense"), None);
    }
    #[test]
    fn parses_decorations() {
        let refs = parse_refs("HEAD -> main, origin/main, origin/HEAD, tag: v1.2, feature");
        assert_eq!(
            refs.iter().map(|r| (r.name.as_str(), r.kind)).collect::<Vec<_>>(),
            vec![
                ("main", RefKind::Head),
                ("origin/main", RefKind::Remote),
                ("v1.2", RefKind::Tag),
                ("feature", RefKind::Branch)
            ]
        );
    }
    #[test]
    fn parses_unified_diffs() {
        let diff = "diff --git a/src/a.rs b/src/a.rs\nindex 1..2 100644\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,3 +1,4 @@ fn main\n keep\n-old\n+new\n+more\n tail\ndiff --git a/new.txt b/new.txt\nnew file mode 100644\n--- /dev/null\n+++ b/new.txt\n@@ -0,0 +1 @@\n+hello\n\\ No newline at end of file\ndiff --git a/x b/y\nsimilarity index 100%\nrename from x\nrename to y\n";
        let files = parse_diff(diff);
        assert_eq!(files.len(), 3);
        let a = &files[0];
        assert_eq!((a.path.as_str(), a.additions, a.deletions), ("src/a.rs", 2, 1));
        assert_eq!(a.lines[0].kind, LineKind::Hunk);
        assert_eq!((a.lines[2].kind, a.lines[2].old), (LineKind::Del, Some(2)));
        assert_eq!((a.lines[3].kind, a.lines[3].new), (LineKind::Add, Some(2)));
        assert_eq!((a.lines[5].old, a.lines[5].new), (Some(3), Some(4)));
        assert_eq!(files[1].status, FileStatus::Added);
        assert_eq!(files[1].lines.len(), 2);
        assert_eq!(files[2].status, FileStatus::Renamed);
        assert_eq!(files[2].old_path.as_deref(), Some("x"));
        assert_eq!(files[2].path, "y");
    }
    #[test]
    fn detects_manual_triggers() {
        assert!(declares_dispatch("on:\n  push:\n  workflow_dispatch:\n    inputs: {}"));
        assert!(declares_dispatch("on: [push, workflow_dispatch]"));
        assert!(declares_dispatch("on: workflow_dispatch"));
        assert!(!declares_dispatch("on:\n  push:\n  # workflow_dispatch:\n"));
        assert!(!declares_dispatch("on: workflow_dispatched_elsewhere"));
        assert!(!declares_dispatch("on: [push, pull_request]"));
    }
    #[test]
    fn groups_review_threads() {
        let c = |id: u64, reply: Option<u64>, at: &str| ReviewComment {
            id,
            in_reply_to_id: reply,
            user: None,
            body: String::new(),
            path: "a.rs".into(),
            line: Some(1),
            original_line: None,
            side: None,
            start_line: None,
            start_side: None,
            created_at: at.into(),
        };
        let t = threads(vec![c(2, Some(1), "2026-01-02"), c(1, None, "2026-01-01"), c(3, None, "2026-01-03")]);
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].root.id, 1);
        assert_eq!(t[0].replies.iter().map(|r| r.id).collect::<Vec<_>>(), vec![2]);
        assert!(t[1].replies.is_empty());
    }
    #[test]
    fn builds_review_payloads() {
        let single = DraftComment {
            path: "a.rs".into(),
            line: 7,
            right: true,
            start: None,
            body: "nit".into(),
        };
        let range = DraftComment {
            path: "b.rs".into(),
            line: 12,
            right: false,
            start: Some((9, false)),
            body: "why?".into(),
        };
        let v = review_payload("abc123", ReviewEvent::RequestChanges, "needs work", &[single, range]);
        assert_eq!(v["commit_id"], "abc123");
        assert_eq!(v["event"], "REQUEST_CHANGES");
        assert_eq!(v["body"], "needs work");
        let c = v["comments"].as_array().unwrap();
        assert_eq!(c.len(), 2);
        assert_eq!((c[0]["path"].as_str(), c[0]["line"].as_u64(), c[0]["side"].as_str()), (Some("a.rs"), Some(7), Some("RIGHT")));
        assert!(c[0].get("start_line").is_none());
        assert_eq!((c[1]["start_line"].as_u64(), c[1]["start_side"].as_str(), c[1]["side"].as_str()), (Some(9), Some("LEFT"), Some("LEFT")));
        assert_eq!(review_payload("x", ReviewEvent::Approve, "", &[])["comments"], serde_json::json!([]));
    }
    #[test]
    fn rolls_up_checks() {
        let c = |status: &str, conclusion: &str| Check {
            status: Some(status.into()),
            conclusion: Some(conclusion.into()),
            ..Default::default()
        };
        assert_eq!(rollup(&[]), None);
        assert_eq!(rollup(&[c("COMPLETED", "SUCCESS")]), Some(CheckState::Pass));
        assert_eq!(
            rollup(&[c("COMPLETED", "SUCCESS"), c("IN_PROGRESS", "")]),
            Some(CheckState::Pending)
        );
        assert_eq!(
            rollup(&[c("IN_PROGRESS", ""), c("COMPLETED", "FAILURE")]),
            Some(CheckState::Fail)
        );
        let status = Check {
            state: Some("SUCCESS".into()),
            ..Default::default()
        };
        assert_eq!(status.outcome(), CheckState::Pass);
        assert_eq!(c("COMPLETED", "SKIPPED").outcome(), CheckState::Skipped);
    }
    #[test]
    fn parses_generated_release_notes() {
        let sample = r#"{"name":"v1.2.0","body":"What's Changed\n* Fix crash by @someone in https://github.com/o/r/pull/12\n\n**Full Changelog**: https://github.com/o/r/compare/v1.1.0...v1.2.0"}"#;
        let notes: GeneratedNotes = json(sample).unwrap();
        assert_eq!(notes.name, "v1.2.0");
        assert!(notes.body.starts_with("What's Changed"));
        assert!(notes.body.contains("pull/12"));
        let bare: GeneratedNotes = json(r#"{"name":"v2"}"#).unwrap();
        assert_eq!((bare.name.as_str(), bare.body.as_str()), ("v2", ""));
    }
}

/// Exercises every fetcher against real GitHub data through the signed-in gh.
/// Run with `cargo test -- --ignored live`.
#[cfg(test)]
mod live {
    use super::*;
    #[test]
    #[ignore]
    fn live_fetchers_parse_real_responses() {
        let slug = "cli/cli";
        let info = repo_info(slug).unwrap();
        let branch = info.default_branch();
        assert!(!branches(slug).unwrap().is_empty());
        let root = tree(slug, "", &branch).unwrap();
        let readme = root.iter().find(|e| e.name.to_lowercase().starts_with("readme")).unwrap();
        assert!(matches!(blob(slug, &readme.path, &branch).unwrap(), Blob::Text(_)));
        let b = blame(slug, &branch, &readme.path).unwrap();
        assert!(b.at(0).is_some());
        let paths = root.iter().map(|e| e.path.clone()).collect::<Vec<_>>();
        let folder = folder_info(slug, &branch, &paths).unwrap();
        assert!(folder.head.is_some());
        assert!(folder.files.contains_key(&readme.path));
        let act = activity(slug, &branch).unwrap();
        assert_eq!(act.days.len(), ACTIVITY_DAYS);
        let log = remote_log(slug, &branch, 120).unwrap();
        assert_eq!(log.len(), 120);
        let detail = commit_detail(slug, None, &log[0].sha).unwrap();
        assert!(!detail.message.is_empty());
        let prs = pulls(slug, PrFilter::Open).unwrap();
        if let Some(p) = prs.first() {
            let d = pull(slug, p.number).unwrap();
            assert_eq!(d.summary.number, p.number);
            pull_diff(slug, p.number).unwrap();
        }
        pulls(slug, PrFilter::Merged).unwrap();
        assert!(!workflows(slug).unwrap().is_empty());
        let r = runs(slug, None).unwrap();
        let done = r.iter().find(|r| !r.active()).unwrap();
        let j = jobs(slug, done.database_id).unwrap();
        assert!(!j.is_empty());
        let lines = job_log(slug, j[0].database_id).unwrap();
        assert!(!lines.is_empty());
        assert!(!releases(slug).unwrap().is_empty());
        contributors(slug).unwrap();
        current_user().unwrap();
        if let Some(home) = std::env::var_os("HOME") {
            let local = std::path::Path::new(&home).join("Documents/codes/mine/cloudwatch");
            if local.exists() {
                assert_eq!(local_remote(&local).unwrap().1, "cloudwatch");
                let l = local_log(&local, None, 200).unwrap();
                assert!(!l.is_empty());
                let d = commit_detail("blackmann/cloudwatch", Some(&local), &l[0].sha).unwrap();
                assert!(!d.files.is_empty());
                let g = crate::graph::layout(l.iter().map(|c| (c.sha.as_str(), c.parents.as_slice())));
                assert_eq!(g.rows.len(), l.len());
            }
        }
    }
}
