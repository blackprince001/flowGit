//! Which files of a pull request the signed-in user has marked as viewed, read
//! and written through GraphQL (the REST API has no equivalent).
use super::*;
use std::collections::HashMap;

/// A pull request's files and whether the viewer has marked each as viewed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ViewedFiles {
    /// GraphQL's id for the pull request, which the mutations need.
    pub pr_id: String,
    pub files: HashMap<String, bool>,
}
impl ViewedFiles {
    /// (viewed, total) over the pull request's files.
    pub fn counts(&self) -> (usize, usize) {
        (self.files.values().filter(|v| **v).count(), self.files.len())
    }
}

/// One page of the files query.
#[derive(Debug, PartialEq, Eq)]
pub struct ViewedPage {
    pub pr_id: String,
    pub files: Vec<(String, bool)>,
    /// The cursor for the next page, when there is one.
    pub next: Option<String>,
}

/// Read a page of `pullRequest { id files { nodes { path viewerViewedState } } }`.
pub fn parse_viewed_page(text: &str) -> Result<ViewedPage> {
    let v: serde_json::Value = json(text)?;
    let pr = &v["data"]["repository"]["pullRequest"];
    let pr_id = pr["id"].as_str().ok_or_else(|| anyhow!("Pull request not found"))?.to_string();
    let files = &pr["files"];
    let nodes = files["nodes"].as_array().ok_or_else(|| anyhow!("The pull request has no file list"))?;
    let files_out = nodes
        .iter()
        .filter_map(|n| Some((n["path"].as_str()?.to_string(), n["viewerViewedState"].as_str() == Some("VIEWED"))))
        .collect();
    let next = (files["pageInfo"]["hasNextPage"].as_bool() == Some(true))
        .then(|| files["pageInfo"]["endCursor"].as_str().map(str::to_string))
        .flatten();
    Ok(ViewedPage { pr_id, files: files_out, next })
}

/// Every file's viewed state, following the cursor past 100 files.
pub fn viewed_files(slug: &str, number: u64) -> Result<ViewedFiles> {
    let (owner, name) = slug.split_once('/').ok_or_else(|| anyhow!("Bad repository name"))?;
    let query = "query($owner: String!, $name: String!, $number: Int!, $after: String) { repository(owner: $owner, name: $name) { pullRequest(number: $number) { id files(first: 100, after: $after) { pageInfo { hasNextPage endCursor } nodes { path viewerViewedState } } } } }";
    let mut out = ViewedFiles::default();
    let mut after: Option<String> = None;
    loop {
        let (q, o, n, num) = (format!("query={query}"), format!("owner={owner}"), format!("name={name}"), format!("number={number}"));
        let mut args = vec!["api", "graphql", "-f", &q, "-f", &o, "-f", &n, "-F", &num];
        let cursor = after.as_ref().map(|c| format!("after={c}"));
        if let Some(c) = &cursor {
            args.extend(["-f", c]);
        }
        let page = parse_viewed_page(&gh(&args)?)?;
        out.pr_id = page.pr_id;
        out.files.extend(page.files);
        match page.next {
            Some(next) => after = Some(next),
            None => return Ok(out),
        }
    }
}

/// Mark one file viewed or not. Not cached: the query starts with `mutation`.
pub fn set_file_viewed(pr_id: &str, path: &str, viewed: bool) -> Result<String> {
    let field = if viewed { "markFileAsViewed" } else { "unmarkFileAsViewed" };
    let query = format!("mutation($id: ID!, $path: String!) {{ {field}(input: {{ pullRequestId: $id, path: $path }}) {{ clientMutationId }} }}");
    gh(&["api", "graphql", "-f", &format!("query={query}"), "-f", &format!("id={pr_id}"), "-f", &format!("path={path}")])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_viewed_state_per_file() {
        let page = parse_viewed_page(
            r#"{"data":{"repository":{"pullRequest":{"id":"PR_1","files":{"pageInfo":{"hasNextPage":false,"endCursor":"x"},"nodes":[
                {"path":"src/a.rs","viewerViewedState":"VIEWED"},
                {"path":"src/b.rs","viewerViewedState":"UNVIEWED"},
                {"path":"src/c.rs","viewerViewedState":"DISMISSED"}]}}}}}"#,
        )
        .unwrap();
        assert_eq!(page.pr_id, "PR_1");
        assert_eq!(
            page.files,
            vec![("src/a.rs".to_string(), true), ("src/b.rs".to_string(), false), ("src/c.rs".to_string(), false)]
        );
        assert_eq!(page.next, None);
    }
    #[test]
    fn follows_the_cursor_only_when_there_is_more() {
        let page = parse_viewed_page(
            r#"{"data":{"repository":{"pullRequest":{"id":"PR_1","files":{"pageInfo":{"hasNextPage":true,"endCursor":"abc"},"nodes":[]}}}}}"#,
        )
        .unwrap();
        assert_eq!(page.next.as_deref(), Some("abc"));
    }
    #[test]
    fn a_missing_pull_request_is_an_error() {
        assert!(parse_viewed_page(r#"{"data":{"repository":{"pullRequest":null}}}"#).is_err());
        assert!(parse_viewed_page("not json").is_err());
    }
    #[test]
    fn counts_viewed_files() {
        let v = ViewedFiles { pr_id: "x".into(), files: [("a".to_string(), true), ("b".to_string(), false), ("c".to_string(), true)].into() };
        assert_eq!(v.counts(), (2, 3));
    }
}
