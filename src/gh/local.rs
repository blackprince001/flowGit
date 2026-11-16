//! `git` calls for the Changes view: the working tree's status, one file's diff,
//! and the writes (stage, unstage, commit, push, pull). All of them act on a local
//! clone. Paths always follow `--` as separate arguments, never through a shell.
use super::*;

/// Files per `git add` / `git restore`, so a huge change list stays under the
/// argument size limit.
const BATCH: usize = 500;
/// Untracked files larger than this aren't diffed against /dev/null.
const MAX_UNTRACKED: u64 = 2 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
    Conflicted,
    Untracked,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub path: String,
    /// Where a renamed file came from.
    pub old_path: Option<String>,
    pub kind: ChangeKind,
}
impl Change {
    /// The paths git must be given to act on this change as a whole: a rename
    /// is a deletion of the old path and an addition of the new one.
    pub fn paths(&self) -> Vec<String> {
        let mut out = vec![self.path.clone()];
        out.extend(self.old_path.clone());
        out
    }
}

/// Which list a file is shown in. A file can be in both `Staged` and `Unstaged`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, Deserialize)]
pub enum Area {
    Staged,
    Unstaged,
    Untracked,
}

/// A file in one of the lists: what the diff on the right shows.
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, Deserialize)]
pub struct ChangeRef {
    pub path: String,
    pub area: Area,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    /// None on a detached HEAD.
    pub branch: Option<String>,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub staged: Vec<Change>,
    pub unstaged: Vec<Change>,
    pub untracked: Vec<Change>,
}
impl Status {
    /// Changed files, counting one that is both staged and unstaged once.
    pub fn count(&self) -> usize {
        let mut paths: Vec<&str> = self.staged.iter().chain(&self.unstaged).chain(&self.untracked).map(|c| c.path.as_str()).collect();
        paths.sort_unstable();
        paths.dedup();
        paths.len()
    }
    /// A branch with no upstream can be published; one with commits ahead can be pushed.
    pub fn can_push(&self) -> bool {
        self.branch.is_some() && (self.upstream.is_none() || self.ahead > 0)
    }
    pub fn can_pull(&self) -> bool {
        self.upstream.is_some()
    }
    pub fn list(&self, area: Area) -> &[Change] {
        match area {
            Area::Staged => &self.staged,
            Area::Unstaged => &self.unstaged,
            Area::Untracked => &self.untracked,
        }
    }
    /// Where a selected file is now: staging or unstaging moves it to another
    /// list. None once it has no changes left (committed, or reverted outside).
    pub fn locate(&self, pick: &ChangeRef) -> Option<ChangeRef> {
        let has = |area| self.list(area).iter().any(|c| c.path == pick.path);
        if has(pick.area) {
            return Some(pick.clone());
        }
        [Area::Staged, Area::Unstaged, Area::Untracked]
            .into_iter()
            .find(|a| has(*a))
            .map(|area| ChangeRef { path: pick.path.clone(), area })
    }
}

fn kind_of(code: char) -> Option<ChangeKind> {
    match code {
        '.' | ' ' => None,
        'A' | 'C' => Some(ChangeKind::Added),
        'D' => Some(ChangeKind::Deleted),
        'R' => Some(ChangeKind::Renamed),
        _ => Some(ChangeKind::Modified),
    }
}

/// Parse `git status --porcelain=v2 --branch -z`. Entries are NUL separated and
/// paths are raw (no quoting), so names can hold spaces and any other character.
pub fn parse_status(text: &str) -> Status {
    let mut status = Status::default();
    let mut fields = text.split('\0');
    while let Some(f) = fields.next() {
        if let Some(header) = f.strip_prefix("# ") {
            let (key, value) = header.split_once(' ').unwrap_or((header, ""));
            match key {
                "branch.head" => status.branch = (value != "(detached)").then(|| value.to_string()),
                "branch.upstream" => status.upstream = Some(value.to_string()),
                "branch.ab" => {
                    // +ahead -behind
                    let mut n = value.split(' ').map(|p| p.trim_start_matches(['+', '-']).parse::<u32>().unwrap_or(0));
                    status.ahead = n.next().unwrap_or(0);
                    status.behind = n.next().unwrap_or(0);
                }
                _ => {}
            }
        } else if let Some(path) = f.strip_prefix("? ") {
            status.untracked.push(Change { path: path.to_string(), old_path: None, kind: ChangeKind::Untracked });
        } else if f.starts_with("1 ") || f.starts_with("2 ") {
            // 1 XY sub mH mI mW hH hI path
            // 2 XY sub mH mI mW hH hI Xscore path, then the original path as the next field
            let renamed = f.starts_with("2 ");
            let parts: Vec<&str> = f.splitn(if renamed { 10 } else { 9 }, ' ').collect();
            let (Some(xy), Some(path)) = (parts.get(1), parts.last()) else { continue };
            let old = renamed.then(|| fields.next().unwrap_or("").to_string());
            let mut codes = xy.chars();
            let (x, y) = (codes.next().unwrap_or('.'), codes.next().unwrap_or('.'));
            let make = |code: char| {
                kind_of(code).map(|kind| Change {
                    path: path.to_string(),
                    old_path: old.clone().filter(|_| matches!(code, 'R' | 'C')),
                    kind,
                })
            };
            status.staged.extend(make(x));
            status.unstaged.extend(make(y));
        } else if f.starts_with("u ") {
            // u XY sub m1 m2 m3 mW h1 h2 h3 path
            if let Some(path) = f.splitn(11, ' ').nth(10) {
                status.unstaged.push(Change { path: path.to_string(), old_path: None, kind: ChangeKind::Conflicted });
            }
        }
    }
    status
}

/// The working tree of a clone. `--no-optional-locks` keeps this polling from
/// taking the index lock a commit or `git add` in a terminal needs.
pub fn status(path: &Path) -> Result<Status> {
    let out = git(path, &["--no-optional-locks", "status", "--porcelain=v2", "--branch", "-z", "--untracked-files=all"])?;
    Ok(parse_status(&out))
}

/// One file's changes as a parsed diff: the index against HEAD (staged), the
/// working tree against the index, or a new file against nothing.
pub fn file_diff(path: &Path, file: &str, area: Area) -> Result<Vec<FileDiff>> {
    let text = match area {
        Area::Staged | Area::Unstaged => {
            let mut args = vec!["-c", "core.quotePath=false", "diff"];
            if area == Area::Staged {
                args.push("--cached");
            }
            args.extend(["--no-color", "--no-ext-diff", "--", file]);
            git(path, &args)?
        }
        Area::Untracked => {
            if crate::cache::is_cached_only() {
                bail!("Not cached");
            }
            let size = std::fs::metadata(path.join(file)).map(|m| m.len()).unwrap_or(0);
            if size > MAX_UNTRACKED {
                bail!("{file} is too large to preview ({} MB).", size / (1024 * 1024));
            }
            // `--no-index` exits 1 when the files differ, which is the normal case here.
            let mut cmd = Command::new("git");
            cmd.arg("-C")
                .arg(path)
                .args(["-c", "core.quotePath=false", "diff", "--no-index", "--no-color", "--no-ext-diff"])
                .args(["--", "/dev/null", file])
                .env("PATH", search_path())
                .env("GIT_TERMINAL_PROMPT", "0")
                .stdin(Stdio::null());
            let out = cmd.output().map_err(|e| anyhow!("Could not start git: {e}"))?;
            match out.status.code() {
                Some(0 | 1) => String::from_utf8_lossy(&out.stdout).into_owned(),
                _ => bail!("{}", String::from_utf8_lossy(&out.stderr).lines().take(6).collect::<Vec<_>>().join("\n")),
            }
        }
    };
    Ok(parse_diff(&text))
}

pub fn stage(path: &Path, files: &[String]) -> Result<String> {
    for batch in files.chunks(BATCH) {
        let mut args = vec!["add", "--"];
        args.extend(batch.iter().map(String::as_str));
        git(path, &args)?;
    }
    Ok(String::new())
}

/// Take files out of the index, keeping their working tree changes. Before the
/// first commit there is no HEAD to restore from, so the entries are dropped.
pub fn unstage(path: &Path, files: &[String]) -> Result<String> {
    let has_head = git(path, &["rev-parse", "--verify", "--quiet", "HEAD"]).is_ok();
    for batch in files.chunks(BATCH) {
        let mut args = if has_head {
            vec!["restore", "--staged", "--"]
        } else {
            vec!["rm", "--cached", "-r", "-q", "--ignore-unmatch", "--"]
        };
        args.extend(batch.iter().map(String::as_str));
        git(path, &args)?;
    }
    Ok(String::new())
}

/// Commit what is staged. The message goes in on stdin so it can be any text.
pub fn commit(path: &Path, message: &str) -> Result<String> {
    let message = message.trim();
    if message.is_empty() {
        bail!("Write a commit message first.");
    }
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(path).args(["commit", "-F", "-"]);
    run(cmd, Some(&format!("{message}\n")), "git")?;
    Ok(format!("Committed: {}", message.lines().next().unwrap_or_default()))
}

/// Push the current branch, publishing it to `origin` first when it has no upstream.
pub fn push_branch(path: &Path) -> Result<String> {
    let tracked = git(path, &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"]).is_ok();
    if tracked {
        git(path, &["push"])
    } else {
        git(path, &["push", "-u", "origin", "HEAD"])
    }
}

/// Named apart from `pull`, which fetches a pull request.
/// Fast-forward only: never creates a merge or rewrites local commits.
pub fn pull_branch(path: &Path) -> Result<String> {
    git(path, &["pull", "--ff-only"])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nul(entries: &[&str]) -> String {
        entries.iter().map(|e| format!("{e}\0")).collect()
    }

    #[test]
    fn reads_branch_headers() {
        let s = parse_status(&nul(&[
            "# branch.oid 3f2a1bd9c1b0",
            "# branch.head main",
            "# branch.upstream origin/main",
            "# branch.ab +2 -1",
        ]));
        assert_eq!(s.branch.as_deref(), Some("main"));
        assert_eq!(s.upstream.as_deref(), Some("origin/main"));
        assert_eq!((s.ahead, s.behind), (2, 1));
        assert_eq!(s.count(), 0);
        assert!(s.can_push() && s.can_pull());
    }

    #[test]
    fn unpublished_and_detached_branches() {
        let s = parse_status(&nul(&["# branch.oid abc", "# branch.head topic"]));
        assert_eq!(s.upstream, None);
        assert!(s.can_push() && !s.can_pull());
        let s = parse_status(&nul(&["# branch.oid abc", "# branch.head (detached)"]));
        assert_eq!(s.branch, None);
        assert!(!s.can_push());
    }

    #[test]
    fn splits_staged_unstaged_and_untracked() {
        let s = parse_status(&nul(&[
            "# branch.head main",
            "1 .M N... 100644 100644 100644 aaaa bbbb src/lib.rs",
            "1 A. N... 000000 100644 100644 0000 cccc src/new file.rs",
            "1 D. N... 100644 000000 000000 dddd 0000 gone.txt",
            "1 MM N... 100644 100644 100644 eeee ffff both.rs",
            "1 .D N... 100644 100644 000000 1111 1111 lost.txt",
            "? notes.txt",
            "? docs/draft.md",
        ]));
        let paths = |list: &[Change]| list.iter().map(|c| (c.path.clone(), c.kind)).collect::<Vec<_>>();
        assert_eq!(
            paths(&s.staged),
            vec![
                ("src/new file.rs".to_string(), ChangeKind::Added),
                ("gone.txt".to_string(), ChangeKind::Deleted),
                ("both.rs".to_string(), ChangeKind::Modified),
            ]
        );
        assert_eq!(
            paths(&s.unstaged),
            vec![
                ("src/lib.rs".to_string(), ChangeKind::Modified),
                ("both.rs".to_string(), ChangeKind::Modified),
                ("lost.txt".to_string(), ChangeKind::Deleted),
            ]
        );
        assert_eq!(
            paths(&s.untracked),
            vec![("notes.txt".to_string(), ChangeKind::Untracked), ("docs/draft.md".to_string(), ChangeKind::Untracked)]
        );
        // both.rs is in two lists but is one changed file.
        assert_eq!(s.count(), 7);
    }

    #[test]
    fn reads_renames_with_their_original_path() {
        let s = parse_status(&nul(&[
            "2 R. N... 100644 100644 100644 aaaa aaaa R100 new name.rs",
            "old name.rs",
            "2 RM N... 100644 100644 100644 bbbb cccc R90 moved.rs",
            "was.rs",
            "1 .M N... 100644 100644 100644 dddd eeee after.rs",
        ]));
        assert_eq!(s.staged.len(), 2);
        assert_eq!(s.staged[0], Change { path: "new name.rs".into(), old_path: Some("old name.rs".into()), kind: ChangeKind::Renamed });
        assert_eq!(s.staged[0].paths(), vec!["new name.rs", "old name.rs"]);
        assert_eq!(s.staged[1].old_path.as_deref(), Some("was.rs"));
        // The content edit on top of a staged rename is a plain modification.
        assert_eq!(s.unstaged[0], Change { path: "moved.rs".into(), old_path: None, kind: ChangeKind::Modified });
        // The entry after a rename isn't swallowed by its original-path field.
        assert_eq!(s.unstaged[1].path, "after.rs");
    }

    #[test]
    fn marks_conflicts() {
        let s = parse_status(&nul(&["u UU N... 100644 100644 100644 100644 aaaa bbbb cccc clash.rs"]));
        assert_eq!(s.unstaged, vec![Change { path: "clash.rs".into(), old_path: None, kind: ChangeKind::Conflicted }]);
        assert!(s.staged.is_empty());
    }

    #[test]
    fn selection_follows_a_file_between_lists() {
        let s = parse_status(&nul(&["1 A. N... 000000 100644 100644 0000 aaaa a.rs", "1 .M N... 100644 100644 100644 bbbb cccc b.rs", "? c.rs"]));
        let pick = |p: &str, area| ChangeRef { path: p.into(), area };
        // Staged since it was picked.
        assert_eq!(s.locate(&pick("a.rs", Area::Unstaged)), Some(pick("a.rs", Area::Staged)));
        // Still where it was.
        assert_eq!(s.locate(&pick("b.rs", Area::Unstaged)), Some(pick("b.rs", Area::Unstaged)));
        // Unstaged since it was picked; never tracked, so now untracked.
        assert_eq!(s.locate(&pick("c.rs", Area::Staged)), Some(pick("c.rs", Area::Untracked)));
        // Committed.
        assert_eq!(s.locate(&pick("d.rs", Area::Staged)), None);
    }

    /// The whole round trip against a real repository, so the git arguments are checked too.
    #[test]
    fn stages_diffs_and_commits_in_a_real_repository() {
        let dir = std::env::temp_dir().join(format!("gitflow-changes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let g = |args: &[&str]| git(&dir, args).unwrap();
        g(&["init", "-q", "-b", "main"]);
        g(&["config", "user.email", "t@example.com"]);
        g(&["config", "user.name", "T"]);
        g(&["config", "commit.gpgsign", "false"]);
        std::fs::write(dir.join("old name.txt"), "one\ntwo\nthree\nfour\n").unwrap();
        // Before the first commit there is nothing to restore from.
        stage(&dir, &["old name.txt".to_string()]).unwrap();
        assert_eq!(status(&dir).unwrap().staged.len(), 1);
        unstage(&dir, &["old name.txt".to_string()]).unwrap();
        let s = status(&dir).unwrap();
        assert!(s.staged.is_empty() && s.untracked.len() == 1);
        assert_eq!(s.branch.as_deref(), Some("main"));

        stage(&dir, &["old name.txt".to_string()]).unwrap();
        commit(&dir, "First\n\nwith a body").unwrap();
        assert_eq!(status(&dir).unwrap().count(), 0);
        assert_eq!(g(&["log", "-1", "--format=%B"]).trim(), "First\n\nwith a body");

        // Untracked: diffed against nothing, and git's exit status 1 is not an error.
        std::fs::write(dir.join("-odd file.txt"), "hello\n").unwrap();
        let d = file_diff(&dir, "-odd file.txt", Area::Untracked).unwrap();
        assert_eq!((d.len(), d[0].status, d[0].additions), (1, FileStatus::Added, 1));

        // Unstaged then staged edit of a tracked file.
        std::fs::write(dir.join("old name.txt"), "one\ntwo\nTHREE\nfour\n").unwrap();
        let d = file_diff(&dir, "old name.txt", Area::Unstaged).unwrap();
        assert_eq!((d[0].additions, d[0].deletions), (1, 1));
        assert!(file_diff(&dir, "old name.txt", Area::Staged).unwrap().is_empty());
        stage(&dir, &["old name.txt".to_string()]).unwrap();
        assert_eq!(file_diff(&dir, "old name.txt", Area::Staged).unwrap()[0].additions, 1);
        commit(&dir, "Edit").unwrap();

        // A staged rename moves as one change, and unstaging it takes both paths.
        g(&["mv", "old name.txt", "new name.txt"]);
        let s = status(&dir).unwrap();
        assert_eq!(s.staged[0].kind, ChangeKind::Renamed);
        assert_eq!(s.staged[0].old_path.as_deref(), Some("old name.txt"));
        unstage(&dir, &s.staged[0].paths()).unwrap();
        let s = status(&dir).unwrap();
        assert!(s.staged.is_empty());
        assert_eq!(s.unstaged[0].kind, ChangeKind::Deleted);

        // No upstream here, so there is nothing to pull and publishing needs a remote.
        assert!(!status(&dir).unwrap().can_pull());
        assert!(push_branch(&dir).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
