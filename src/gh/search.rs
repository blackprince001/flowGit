//! Code search: `git grep` in a local clone, GitHub's code search otherwise.
//! Both answer with the same shape, so the view doesn't care which ran.
use super::*;
use std::{
    io::{BufRead, BufReader, Read},
    ops::Range,
};

/// `git grep` stops after this many matching lines.
const MAX_LINES: usize = 500;
/// GitHub code search returns this many files.
const MAX_FILES: usize = 50;
/// Longer lines (minified files) are cut to a window around the first match.
const MAX_TEXT: usize = 240;

pub struct CodeMatches {
    pub files: Vec<FileMatches>,
    /// More matches exist than were returned.
    pub truncated: bool,
}
pub struct FileMatches {
    pub path: String,
    pub lines: Vec<MatchLine>,
}
pub struct MatchLine {
    /// 1-based. GitHub's search doesn't say where in the file a snippet came from.
    pub line: Option<u32>,
    pub text: String,
    /// Byte ranges of `text` that matched, sorted and not overlapping.
    pub ranges: Vec<Range<usize>>,
}

/// Search `slug`. With a local clone, grep its working tree; without, ask GitHub
/// (default branch only).
pub fn search_code(slug: &str, local: Option<&Path>, query: &str) -> Result<CodeMatches> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(CodeMatches { files: vec![], truncated: false });
    }
    match local {
        Some(path) => grep(path, query),
        None => {
            // `--` so a query that starts with a dash is never read as a flag.
            let out = gh(&["search", "code", "--repo", slug, "--json", "path,textMatches", "--limit", &MAX_FILES.to_string(), "--", query])?;
            parse_remote(&out)
        }
    }
}

fn grep(path: &Path, query: &str) -> Result<CodeMatches> {
    // Local clones answer quickly; the cached pass leaves them to the live one.
    if crate::cache::is_cached_only() {
        bail!("Not cached");
    }
    let mut cmd = Command::new("git");
    // Literal and case-insensitive, like GitHub's search, over tracked and new files. `-z` splits path, line
    // number and text with NULs, so a colon in a path can't confuse the parser.
    cmd.arg("-C")
        .arg(path)
        .args(["grep", "-n", "-I", "-i", "-F", "--full-name", "-z", "--no-color", "--untracked", "-e", query])
        .env("PATH", search_path())
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            anyhow!("git was not found. Install it or set its path in Preferences › GitHub.")
        } else {
            anyhow!("Could not start git: {e}")
        }
    })?;
    let mut stdout = BufReader::new(child.stdout.take().context("git gave no output")?);
    let (mut text, mut count, mut truncated) = (String::new(), 0, false);
    let mut buf = vec![];
    loop {
        buf.clear();
        if stdout.read_until(b'\n', &mut buf)? == 0 {
            break;
        }
        if count == MAX_LINES {
            truncated = true;
            break;
        }
        text.push_str(&String::from_utf8_lossy(&buf));
        count += 1;
    }
    if truncated {
        // A query like "e" would otherwise make git read the whole repository.
        let _ = child.kill();
        let _ = child.wait();
    } else {
        let status = child.wait()?;
        // 1 means "no matches", which is an answer, not a failure.
        if !matches!(status.code(), Some(0 | 1)) {
            let mut err = String::new();
            if let Some(mut pipe) = child.stderr.take() {
                let _ = pipe.read_to_string(&mut err);
            }
            let err = err.trim();
            bail!("{}", if err.is_empty() { format!("git grep exited with {status}") } else { err.lines().take(6).collect::<Vec<_>>().join("\n") });
        }
    }
    let mut found = parse_grep(&text, query);
    found.truncated = truncated;
    Ok(found)
}

/// Every non-overlapping, case-insensitive (ASCII) occurrence of `needle`.
fn find_all(text: &str, needle: &str) -> Vec<Range<usize>> {
    if needle.is_empty() {
        return vec![];
    }
    let needle = needle.to_ascii_lowercase();
    text.to_ascii_lowercase()
        .match_indices(&needle)
        .map(|(i, m)| i..i + m.len())
        .collect()
}

/// Trim indentation and cut very long lines, moving the ranges along with the text.
fn fit(text: &str, ranges: Vec<Range<usize>>) -> (String, Vec<Range<usize>>) {
    const DOTS: &str = "…";
    let body = text.trim_end();
    let lead = body.len() - body.trim_start().len();
    let mut start = lead;
    if body.len() - start > MAX_TEXT
        && let Some(first) = ranges.first()
    {
        start = start.max(first.start.saturating_sub(40));
    }
    while !body.is_char_boundary(start) {
        start += 1;
    }
    let mut end = (start + MAX_TEXT).min(body.len());
    while !body.is_char_boundary(end) {
        end -= 1;
    }
    let before = if start > lead { DOTS } else { "" };
    let after = if end < body.len() { DOTS } else { "" };
    let out = format!("{before}{}{after}", &body[start..end]);
    let ranges = ranges
        .into_iter()
        .filter(|r| r.start >= start && r.end <= end)
        .map(|r| r.start - start + before.len()..r.end - start + before.len())
        .collect();
    (out, ranges)
}

/// `git grep -n -z` output: `path NUL line NUL text` per line, grouped by file.
fn parse_grep(output: &str, query: &str) -> CodeMatches {
    let mut files: Vec<FileMatches> = vec![];
    for row in output.lines() {
        let mut parts = row.splitn(3, '\0');
        let (Some(path), Some(line), Some(text)) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        let Ok(line) = line.trim().parse::<u32>() else { continue };
        // Tabs become one space, so ranges found now still fit.
        let text = text.trim_end_matches('\r').replace('\t', " ");
        let (text, ranges) = fit(&text, find_all(&text, query));
        if files.last().is_none_or(|f| f.path != path) {
            files.push(FileMatches { path: path.to_string(), lines: vec![] });
        }
        if let Some(f) = files.last_mut() {
            f.lines.push(MatchLine { line: Some(line), text, ranges });
        }
    }
    CodeMatches { files, truncated: false }
}

#[derive(Deserialize)]
struct RemoteItem {
    path: String,
    #[serde(default, rename = "textMatches")]
    text_matches: Option<Vec<RemoteSnippet>>,
}
#[derive(Deserialize)]
struct RemoteSnippet {
    #[serde(default)]
    fragment: Option<String>,
    #[serde(default)]
    property: Option<String>,
    #[serde(default)]
    matches: Option<Vec<RemoteMark>>,
}
#[derive(Deserialize)]
struct RemoteMark {
    #[serde(default)]
    text: String,
    #[serde(default)]
    indices: Vec<usize>,
}

/// `gh search code --json path,textMatches`: a few lines of context per hit,
/// with the matched text marked by offsets into the snippet.
fn parse_remote(output: &str) -> Result<CodeMatches> {
    let items: Vec<RemoteItem> = json(output)?;
    let truncated = items.len() >= MAX_FILES;
    let files = items
        .into_iter()
        .map(|item| {
            let mut lines: Vec<MatchLine> = vec![];
            for snippet in item.text_matches.unwrap_or_default() {
                // A match on the file's name has no text to show.
                if snippet.property.as_deref() == Some("path") {
                    continue;
                }
                let fragment = snippet.fragment.unwrap_or_default();
                let marks = snippet.matches.unwrap_or_default();
                let mut offset = 0;
                for raw in fragment.split('\n') {
                    let span = offset..offset + raw.len();
                    offset = span.end + 1;
                    let mut ranges: Vec<Range<usize>> = vec![];
                    for mark in &marks {
                        let Some(r) = locate(&fragment, mark) else { continue };
                        if r.start >= span.start && r.start < span.end {
                            ranges.push(r.start - span.start..r.end.min(span.end) - span.start);
                        }
                    }
                    if ranges.is_empty() {
                        continue;
                    }
                    ranges.sort_by_key(|r| r.start);
                    ranges.dedup_by(|b, a| b.start < a.end);
                    let raw = raw.trim_end_matches('\r').replace('\t', " ");
                    let (text, ranges) = fit(&raw, ranges);
                    if !lines.iter().any(|l| l.text == text) {
                        lines.push(MatchLine { line: None, text, ranges });
                    }
                }
            }
            FileMatches { path: item.path, lines }
        })
        .collect();
    Ok(CodeMatches { files, truncated })
}

/// Where a mark sits in its snippet: the given offsets when they hold the marked
/// text, else the first place that text occurs.
fn locate(fragment: &str, mark: &RemoteMark) -> Option<Range<usize>> {
    if let [s, e, ..] = mark.indices[..]
        && s < e
        && fragment.get(s..e).is_some_and(|t| t.eq_ignore_ascii_case(&mark.text))
    {
        return Some(s..e);
    }
    find_all(fragment, &mark.text).into_iter().next()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_grep_output_by_file() {
        let out = "src/gh.rs\0363\0fn encode(s: &str) -> String {\nsrc/gh.rs\0400\0    let x = ENCODE(1); encode(2)\nREADME.md\07\0no colons: here? encode\n";
        let found = parse_grep(out, "encode");
        assert_eq!(found.files.len(), 2);
        assert_eq!(found.files[0].path, "src/gh.rs");
        assert_eq!(found.files[0].lines.len(), 2);
        let first = &found.files[0].lines[0];
        assert_eq!(first.line, Some(363));
        assert_eq!(&first.text[first.ranges[0].clone()], "encode");
        // Indentation is trimmed, both spellings match, and ranges moved with the text.
        let second = &found.files[0].lines[1];
        assert_eq!(second.text, "let x = ENCODE(1); encode(2)");
        let hits: Vec<_> = second.ranges.iter().map(|r| &second.text[r.clone()]).collect();
        assert_eq!(hits, ["ENCODE", "encode"]);
        assert_eq!(found.files[1].lines[0].line, Some(7));
    }

    #[test]
    fn no_grep_output_is_no_matches() {
        assert!(parse_grep("", "x").files.is_empty());
        // A line that isn't path, number, text is skipped rather than failing the search.
        assert!(parse_grep("garbage\n", "x").files.is_empty());
    }

    #[test]
    fn long_lines_keep_the_match_in_view() {
        let text = format!("{}needle{}", "a".repeat(600), "b".repeat(600));
        let (out, ranges) = fit(&text, find_all(&text, "needle"));
        assert!(out.len() < 300);
        assert_eq!(&out[ranges[0].clone()], "needle");
        assert!(out.starts_with('…') && out.ends_with('…'));
    }

    #[test]
    fn long_lines_cut_on_character_boundaries() {
        let text = "é".repeat(400);
        let (out, _) = fit(&text, vec![]);
        assert!(out.ends_with('…'));
    }

    #[test]
    fn reads_github_snippets() {
        let json = r#"[
            {"path":"src/lib.rs","textMatches":[
                {"fragment":"fn one() {}\nfn parse_slug(s: &str) {\n    parse_slug(s)\n}","property":"content","type":"FileContent",
                 "matches":[{"text":"parse_slug","indices":[15,25]},{"text":"parse_slug","indices":[41,51]}]}
            ]},
            {"path":"docs/parse_slug.md","textMatches":[{"fragment":"docs/parse_slug.md","property":"path","matches":[{"text":"parse_slug","indices":[5,15]}]}]},
            {"path":"odd.rs","textMatches":null}
        ]"#;
        let found = parse_remote(json).unwrap();
        assert!(!found.truncated);
        assert_eq!(found.files.len(), 3);
        let lines = &found.files[0].lines;
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "fn parse_slug(s: &str) {");
        assert_eq!(lines[0].line, None);
        assert_eq!(&lines[0].text[lines[0].ranges[0].clone()], "parse_slug");
        assert_eq!(lines[1].text, "parse_slug(s)");
        assert_eq!(&lines[1].text[lines[1].ranges[0].clone()], "parse_slug");
        // A name match lists the file with nothing to show under it.
        assert!(found.files[1].lines.is_empty());
        assert!(found.files[2].lines.is_empty());
    }

    #[test]
    fn bad_offsets_fall_back_to_the_marked_text() {
        let json = r#"[{"path":"a.rs","textMatches":[{"fragment":"let value = 1;","matches":[{"text":"value","indices":[99,104]}]}]}]"#;
        let found = parse_remote(json).unwrap();
        let line = &found.files[0].lines[0];
        assert_eq!(&line.text[line.ranges[0].clone()], "value");
    }

    #[test]
    fn a_full_page_of_files_may_have_more() {
        let item = r#"{"path":"a","textMatches":[]}"#;
        let json = format!("[{}]", vec![item; MAX_FILES].join(","));
        assert!(parse_remote(&json).unwrap().truncated);
        assert!(!parse_remote("[]").unwrap().truncated);
        assert!(parse_remote("not json").is_err());
    }

    /// Against a real repository, so the git arguments are checked too.
    #[test]
    fn greps_a_real_repository() {
        let dir = std::env::temp_dir().join(format!("gitflow-grep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        git(&dir, &["init", "-q", "-b", "main"]).unwrap();
        std::fs::write(dir.join("src/a b.rs"), "fn main() {\n\tprintln!(\"Hello: world\");\n}\n").unwrap();
        std::fs::write(dir.join("bin.dat"), b"hello\0binary").unwrap();
        git(&dir, &["add", "."]).unwrap();

        let found = search_code("o/r", Some(&dir), "hello").unwrap();
        assert_eq!(found.files.len(), 1, "binary files are skipped");
        assert_eq!(found.files[0].path, "src/a b.rs");
        let hit = &found.files[0].lines[0];
        assert_eq!(hit.line, Some(2));
        assert_eq!(&hit.text[hit.ranges[0].clone()], "Hello");
        // A dash is part of the query, and exit status 1 is an empty answer.
        assert!(search_code("o/r", Some(&dir), "-nothing-").unwrap().files.is_empty());

        std::fs::write(dir.join("many.txt"), "x\n".repeat(MAX_LINES + 50)).unwrap();
        git(&dir, &["add", "."]).unwrap();
        let found = search_code("o/r", Some(&dir), "x").unwrap();
        assert!(found.truncated);
        assert_eq!(found.files.iter().map(|f| f.lines.len()).sum::<usize>(), MAX_LINES);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
