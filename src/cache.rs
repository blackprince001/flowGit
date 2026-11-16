//! A disk cache of `gh` read output, so a restart shows the last known state at
//! once while the live request runs. Entries are raw `gh` stdout under
//! `~/Library/Caches/Gitflow`, one folder per signed-in account, keyed by the
//! command's arguments. Only reads are cached; writes never touch it.
use std::{
    cell::Cell,
    collections::HashSet,
    fs,
    path::PathBuf,
    sync::{
        LazyLock, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::SystemTime,
};

/// Bump when cached output would no longer parse the same way.
const VERSION: &str = "v1";
/// Larger responses (big logs, huge diffs) are fetched live each time.
const MAX_ENTRY: usize = 5 << 20;
/// Total size before the oldest entries are dropped, and the size pruned to.
const MAX_TOTAL: u64 = 200 << 20;
const PRUNE_TO: u64 = 150 << 20;

thread_local! {
    /// Set on the thread running the cached pass of a fetch.
    static CACHED_ONLY: Cell<bool> = const { Cell::new(false) };
}

/// Keys fetched live this session. Their disk copy may predate a change made
/// here (a comment, a merge), so the cached pass skips them.
static LIVE: LazyLock<Mutex<HashSet<u64>>> = LazyLock::new(Default::default);
static WRITES: AtomicUsize = AtomicUsize::new(0);

/// Run `job` reading only from the cache: `gh` reads come from disk, a miss
/// fails, and nothing else (writes, git) runs.
pub fn cached_only<T>(job: impl FnOnce() -> T) -> T {
    CACHED_ONLY.with(|c| c.set(true));
    let out = job();
    CACHED_ONLY.with(|c| c.set(false));
    out
}
pub fn is_cached_only() -> bool {
    CACHED_ONLY.with(|c| c.get())
}
/// Carry the current mode into a helper thread (see `gh::workflows`).
pub fn inherit<T>(cached: bool, job: impl FnOnce() -> T) -> T {
    if cached { cached_only(job) } else { job() }
}

/// Whether a `gh` invocation only reads, and so may be cached.
pub fn cacheable(args: &[&str]) -> bool {
    match args {
        ["api", rest @ ..] => {
            let graphql = rest.contains(&"graphql");
            let mut it = rest.iter();
            while let Some(a) = it.next() {
                match *a {
                    "-X" | "--method" | "--input" => return false,
                    "-f" | "-F" | "--field" | "--raw-field" => {
                        // GraphQL reads pass their query and variables as fields;
                        // on any other endpoint, fields make it a write.
                        let value = it.next().copied().unwrap_or("");
                        let mutation = value.strip_prefix("query=").is_some_and(|q| q.trim_start().starts_with("mutation"));
                        if !graphql || mutation {
                            return false;
                        }
                    }
                    _ => {}
                }
            }
            true
        }
        ["pr" | "run" | "repo" | "workflow" | "release" | "issue", verb, ..] => {
            matches!(*verb, "view" | "list" | "diff" | "checks")
        }
        ["search", ..] => true,
        _ => false,
    }
}

/// Stable FNV-1a, so keys survive toolchain updates.
fn hash(parts: &[&str]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for p in parts {
        for b in p.bytes().chain([0]) {
            h ^= b as u64;
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    }
    h
}

/// The signed-in `gh` accounts, read from its hosts file, so switching accounts
/// never shows another account's data.
fn account() -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    let dir = std::env::var("GH_CONFIG_DIR").ok().map(PathBuf::from).unwrap_or_else(|| {
        std::env::var("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(&home).join(".config"))
            .join("gh")
    });
    let hosts = fs::read_to_string(dir.join("hosts.yml")).unwrap_or_default();
    let mut who: Vec<String> = vec![];
    let mut host = "";
    for line in hosts.lines() {
        if !line.starts_with(' ') && line.ends_with(':') {
            host = line.trim_end_matches(':');
        } else if let Some(user) = line.trim().strip_prefix("user:") {
            who.push(format!("{host}/{}", user.trim()));
        }
    }
    format!("{:016x}", hash(&who.iter().map(String::as_str).collect::<Vec<_>>()))
}
static DIR: LazyLock<Option<PathBuf>> = LazyLock::new(|| {
    let home = std::env::var_os("HOME")?;
    let dir = PathBuf::from(home).join("Library/Caches/Gitflow").join(VERSION).join(account());
    fs::create_dir_all(&dir).ok()?;
    Some(dir)
});

fn key(args: &[&str]) -> u64 {
    hash(args)
}
fn file(key: u64) -> Option<PathBuf> {
    Some(DIR.as_ref()?.join(format!("{key:016x}")))
}

/// A cached response, unless it was already fetched live this session.
pub fn read(args: &[&str]) -> Option<String> {
    let k = key(args);
    if LIVE.lock().ok()?.contains(&k) {
        return None;
    }
    let path = file(k)?;
    let text = fs::read_to_string(&path).ok()?;
    // Reads count as use, so pruning keeps what is opened often.
    if let Ok(f) = fs::File::options().write(true).open(&path) {
        let _ = f.set_modified(SystemTime::now());
    }
    Some(text)
}

/// Store a live response.
pub fn write(args: &[&str], text: &str) {
    let k = key(args);
    if let Ok(mut live) = LIVE.lock() {
        live.insert(k);
    }
    if text.len() > MAX_ENTRY {
        return;
    }
    let Some(path) = file(k) else { return };
    // Write then rename, so a crash never leaves half an entry.
    let tmp = path.with_extension("tmp");
    if fs::write(&tmp, text).is_ok() {
        let _ = fs::rename(&tmp, &path);
    }
    if WRITES.fetch_add(1, Ordering::Relaxed).is_multiple_of(100) {
        prune();
    }
}

/// Drop the least recently used entries once the cache outgrows its budget.
pub fn prune() {
    let Some(dir) = DIR.as_ref() else { return };
    let Ok(read) = fs::read_dir(dir) else { return };
    let mut entries: Vec<(SystemTime, u64, PathBuf)> = read
        .flatten()
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            Some((meta.modified().ok()?, meta.len(), e.path()))
        })
        .collect();
    let mut total: u64 = entries.iter().map(|e| e.1).sum();
    if total <= MAX_TOTAL {
        return;
    }
    entries.sort_by_key(|e| e.0);
    for (_, len, path) in entries {
        if total <= PRUNE_TO {
            break;
        }
        if fs::remove_file(&path).is_ok() {
            total -= len;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::cacheable;
    #[test]
    fn caches_reads_only() {
        assert!(cacheable(&["api", "repos/a/b/branches?per_page=100"]));
        assert!(cacheable(&["api", "-H", "Accept: application/vnd.github.diff", "repos/a/b/commits/x"]));
        assert!(cacheable(&["api", "graphql", "-f", "query={ viewer { login } }"]));
        assert!(cacheable(&["api", "graphql", "-f", "query=query($o: String!) { x }", "-f", "owner=a"]));
        assert!(cacheable(&["pr", "view", "1", "-R", "a/b", "--json", "title"]));
        assert!(cacheable(&["run", "view", "-R", "a/b", "--job", "1", "--log"]));
        assert!(!cacheable(&["api", "-X", "POST", "repos/a/b/pulls/1/comments", "--input", "-"]));
        assert!(!cacheable(&["api", "graphql", "-f", "query=mutation { x }"]));
        assert!(!cacheable(&["api", "repos/a/b/issues", "-f", "title=x"]));
        assert!(cacheable(&["search", "prs", "--review-requested=@me", "--json", "number"]));
        assert!(!cacheable(&["pr", "merge", "1", "--squash"]));
        assert!(!cacheable(&["run", "rerun", "1"]));
        assert!(!cacheable(&["workflow", "run", "1", "--ref", "main"]));
        assert!(!cacheable(&["--version"]));
    }
}
