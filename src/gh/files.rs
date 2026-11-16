//! Every file path in a repository, for the file finder.
use super::*;

/// Tracked files of a local clone, or every blob on `reference` remotely.
pub fn files(slug: &str, path: Option<&Path>, reference: &str) -> Result<Vec<String>> {
    let text = match path {
        Some(p) => git(p, &["ls-files"])?,
        None => gh(&[
            "api",
            &format!("repos/{slug}/git/trees/{}?recursive=1", encode(reference)),
            "--jq",
            ".tree[] | select(.type == \"blob\") | .path",
        ])?,
    };
    Ok(text.lines().filter(|l| !l.is_empty()).map(str::to_string).collect())
}
