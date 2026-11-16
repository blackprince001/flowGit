//! Answers MCP tool calls (see `crate::mcp`) from live workspace state.
use super::*;
use crate::mcp::{Call, Reply};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OpenArgs {
    repo: String,
    view: Option<String>,
    pull_request: Option<u64>,
    file: Option<String>,
}

/// The name a view has in tool arguments: its title, lowercase, underscored.
fn view_name(view: View) -> String {
    view.title().to_lowercase().replace(' ', "_")
}

impl Workspace {
    /// Take MCP calls from the socket and answer each on the UI thread.
    pub(super) fn serve_mcp(window: &mut Window, cx: &mut Context<Self>) {
        let Some(calls) = crate::mcp::listen() else { return };
        cx.spawn_in(window, async move |this, cx| {
            while let Ok((call, reply)) = calls.recv().await {
                let answer = this
                    .update_in(cx, |s, w, cx| s.mcp_call(call, w, cx))
                    .unwrap_or(Reply::Error {
                        message: "Gitflow is closing.".into(),
                    });
                let _ = reply.send(answer).await;
            }
        })
        .detach();
    }

    fn mcp_call(&mut self, call: Call, window: &mut Window, cx: &mut Context<Self>) -> Reply {
        let result = match call.tool.as_str() {
            "get_context" => Ok(self.mcp_context().to_string()),
            "list_tabs" => Ok(self.mcp_tabs().to_string()),
            "open" => serde_json::from_value(call.args)
                .map_err(|e| e.to_string())
                .and_then(|a| self.mcp_open(a, window, cx)),
            "draft_review_comment" => Err("Review drafts aren't available in this build yet.".into()),
            other => Err(format!("Unknown tool {other}")),
        };
        match result {
            Ok(text) => Reply::Ok { text },
            Err(message) => Reply::Error { message },
        }
    }

    fn slug_of(&self, key: &str) -> String {
        self.repo(key).map(Repo::slug).unwrap_or_else(|| key.to_string())
    }

    fn mcp_tab(&self, t: &Tab) -> Value {
        let repo = self.repo(&t.repo);
        let mut v = json!({
            "repo": self.slug_of(&t.repo),
            "local_path": repo.and_then(|r| r.path.as_ref()).map(|p| p.display().to_string()),
            "view": view_name(t.view),
            "branch": self.branch_of(t),
        });
        match t.view {
            View::Code => {
                v["directory"] = json!(t.path);
                v["file"] = json!(t.file);
            }
            View::Commits => v["commit"] = json!(t.commit),
            View::Issues => v["issue"] = json!(t.issue),
            View::Pulls => {
                if let Some(n) = t.pr {
                    let title = self
                        .data(&t.repo)
                        .and_then(|d| d.pull.get(&n))
                        .and_then(|l| l.data.as_ref())
                        .map(|p| p.summary.title.clone());
                    let selection = t.line_target.as_ref().map(|end| {
                        let start = t.line_start.as_ref().filter(|s| s.path == end.path).unwrap_or(end);
                        json!({
                            "path": end.path,
                            "start_line": start.line,
                            "line": end.line,
                            "side": if end.right { "RIGHT" } else { "LEFT" },
                        })
                    });
                    v["pull_request"] = json!({
                        "number": n,
                        "title": title,
                        "page": format!("{:?}", t.pr_page).to_lowercase(),
                        "selected_lines": selection,
                    });
                }
            }
            View::Actions => {
                v["run"] = json!(t.run);
                v["job"] = json!(t.job);
            }
            _ => {}
        }
        v
    }

    fn mcp_context(&self) -> Value {
        json!({
            "tab": self.current().map(|t| self.mcp_tab(t)),
            "open_tabs": self.tabs.len(),
        })
    }

    fn mcp_tabs(&self) -> Value {
        json!({
            "tabs": self.tabs.iter().enumerate().map(|(i, t)| {
                let mut v = self.mcp_tab(t);
                v["active"] = json!(self.active == Some(i));
                v
            }).collect::<Vec<_>>(),
            "repositories": self.repos.iter().map(Repo::slug).collect::<Vec<_>>(),
        })
    }

    fn mcp_open(&mut self, a: OpenArgs, window: &mut Window, cx: &mut Context<Self>) -> Result<String, String> {
        let key = self
            .repos
            .iter()
            .find(|r| r.slug().eq_ignore_ascii_case(&a.repo))
            .map(Repo::key)
            .ok_or_else(|| format!("{} isn't in the Gitflow sidebar. Ask the user to add it.", a.repo))?;
        let view = match a.view.as_deref() {
            Some(name) => Some(
                View::ALL
                    .into_iter()
                    .find(|v| view_name(*v) == name)
                    .ok_or_else(|| format!("Unknown view {name}"))?,
            ),
            None => None,
        };
        self.open_repo(&key, window, cx);
        if let Some(view) = view {
            self.set_view(view, cx);
        }
        if let Some(n) = a.pull_request {
            self.show_pull(n, cx);
        }
        if let Some(path) = a.file.as_deref() {
            self.show_file(path.trim_start_matches('/'), cx);
        }
        cx.activate(true);
        Ok(format!("Showing {}.", a.repo))
    }
}
