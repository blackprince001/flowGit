//! Preferences and the saved repository list, stored as JSON in Application Support.
//! No credentials live here: GitHub access is whatever `gh auth` already holds.
use anyhow::Result;
use gpui::{App, Global, SharedString};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const MONO_SIZES: std::ops::RangeInclusive<u32> = 10..=18;
pub const COMMIT_LIMITS: &[u32] = &[100, 250, 500, 1000, 2000];
pub const REFRESH_CHOICES: &[(u32, &str)] = &[
    (15, "15 seconds"),
    (30, "30 seconds"),
    (60, "1 minute"),
    (120, "2 minutes"),
    (300, "5 minutes"),
    (600, "10 minutes"),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ThemePref {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum View {
    #[default]
    Code,
    Commits,
    Issues,
    Pulls,
    Actions,
    Releases,
    Changes,
}
impl View {
    pub const ALL: [View; 7] = [
        View::Code,
        View::Commits,
        View::Issues,
        View::Pulls,
        View::Actions,
        View::Releases,
        View::Changes,
    ];
    /// Changes reads the working tree, so only a local clone has it.
    pub fn available(self, local: bool) -> bool {
        local || self != View::Changes
    }
    pub fn title(self) -> &'static str {
        match self {
            View::Code => "Code",
            View::Commits => "Commits",
            View::Issues => "Issues",
            View::Pulls => "Pull requests",
            View::Actions => "Actions",
            View::Releases => "Releases",
            View::Changes => "Changes",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum MergeMethod {
    Merge,
    #[default]
    Squash,
    Rebase,
}
impl MergeMethod {
    pub const ALL: [MergeMethod; 3] = [MergeMethod::Merge, MergeMethod::Squash, MergeMethod::Rebase];
    pub fn title(self) -> &'static str {
        match self {
            MergeMethod::Merge => "Create a merge commit",
            MergeMethod::Squash => "Squash and merge",
            MergeMethod::Rebase => "Rebase and merge",
        }
    }
    pub fn short(self) -> &'static str {
        match self {
            MergeMethod::Merge => "Merge",
            MergeMethod::Squash => "Squash and merge",
            MergeMethod::Rebase => "Rebase and merge",
        }
    }
    pub fn flag(self) -> &'static str {
        match self {
            MergeMethod::Merge => "--merge",
            MergeMethod::Squash => "--squash",
            MergeMethod::Rebase => "--rebase",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: ThemePref,
    pub mono_font: String,
    pub mono_size: u32,
    /// The view a newly opened repository tab starts on.
    pub default_view: View,
    /// How many commits the graph loads.
    pub commit_limit: u32,
    /// Re-read the visible view (and poll each repository's running Actions).
    pub auto_refresh: bool,
    /// How often auto-refresh runs, in seconds.
    pub refresh_secs: u32,
    pub merge_method: MergeMethod,
    pub delete_branch: bool,
    /// Show "3h ago" rather than calendar dates.
    pub relative_dates: bool,
    /// Load contributor and author avatars from GitHub.
    pub avatars: bool,
    /// An explicit `gh` executable; empty means search the usual install locations.
    pub gh_path: String,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: ThemePref::System,
            mono_font: "Menlo".into(),
            mono_size: 12,
            default_view: View::Code,
            commit_limit: 500,
            auto_refresh: true,
            refresh_secs: 60,
            merge_method: MergeMethod::Squash,
            delete_branch: true,
            relative_dates: true,
            avatars: true,
            gh_path: String::new(),
        }
    }
}
impl Global for Settings {}

pub fn support_dir() -> Option<PathBuf> {
    // Lets development runs use a scratch configuration.
    if let Some(dir) = std::env::var_os("GITFLOW_SUPPORT_DIR") {
        return Some(PathBuf::from(dir));
    }
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join("Library/Application Support/Gitflow"))
}

pub fn write(name: &str, bytes: Vec<u8>) -> Result<()> {
    let dir = support_dir().ok_or_else(|| anyhow::anyhow!("HOME is not set"))?;
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join(name), bytes)?;
    Ok(())
}

impl Settings {
    /// Missing or unreadable settings fall back to defaults; they hold nothing critical.
    pub fn load() -> Self {
        support_dir()
            .and_then(|d| std::fs::read(d.join("settings.json")).ok())
            .and_then(|b| serde_json::from_slice::<Self>(&b).ok())
            .map(Self::sanitized)
            .unwrap_or_default()
    }
    pub fn save(&self) -> Result<()> {
        write("settings.json", serde_json::to_vec_pretty(self)?)
    }
    pub fn sanitized(mut self) -> Self {
        self.mono_size = self.mono_size.clamp(*MONO_SIZES.start(), *MONO_SIZES.end());
        if self.mono_font.trim().is_empty() {
            self.mono_font = Self::default().mono_font;
        }
        // Older settings used 0 seconds for "off".
        if self.refresh_secs == 0 {
            self.auto_refresh = false;
            self.refresh_secs = 60;
        }
        if !REFRESH_CHOICES.iter().any(|(s, _)| *s == self.refresh_secs) {
            self.refresh_secs = 60;
        }
        if !COMMIT_LIMITS.contains(&self.commit_limit) {
            self.commit_limit = Self::default().commit_limit;
        }
        self.gh_path = self.gh_path.trim().to_string();
        self
    }
}

pub fn get(cx: &App) -> &Settings {
    cx.global::<Settings>()
}
pub fn mono(cx: &App) -> SharedString {
    get(cx).mono_font.clone().into()
}

/// A repository in the sidebar. `path` is set when it was added from a local clone,
/// which enables the all-branches graph, fetch and PR checkout.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Repo {
    pub owner: String,
    pub name: String,
    #[serde(default)]
    pub path: Option<PathBuf>,
}
impl Repo {
    pub fn slug(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }
    /// Stable identity for tabs and caches; GitHub names are case-insensitive.
    pub fn key(&self) -> String {
        self.slug().to_lowercase()
    }
}

#[derive(Default, Serialize, Deserialize)]
pub struct Saved {
    #[serde(default)]
    pub repos: Vec<Repo>,
}
impl Saved {
    pub fn load() -> Self {
        support_dir()
            .and_then(|d| std::fs::read(d.join("repositories.json")).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }
    pub fn save(repos: &[Repo]) -> Result<()> {
        write(
            "repositories.json",
            serde_json::to_vec_pretty(&Saved {
                repos: repos.to_vec(),
            })?,
        )
    }
}

/// Fonts worth offering first; only those installed are shown.
pub const SUGGESTED_MONO: &[&str] = &[
    "Menlo",
    "SF Mono",
    "Monaco",
    "JetBrains Mono",
    "Fira Code",
    "IBM Plex Mono",
    "Source Code Pro",
    "Cascadia Code",
    "Iosevka",
    "Hack",
    "Geist Mono",
    "Berkeley Mono",
    "Lilex",
    "Zed Mono",
];

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sanitizes_and_defaults() {
        let s: Settings =
            serde_json::from_str(r#"{"mono_size": 99, "commit_limit": 7}"#).unwrap();
        let s = s.sanitized();
        assert_eq!(s.mono_size, 18);
        assert_eq!(s.commit_limit, 500);
        assert_eq!(s.mono_font, "Menlo");
        assert_eq!(s.merge_method, MergeMethod::Squash);
        let old: Settings = serde_json::from_str(r#"{"refresh_secs": 0}"#).unwrap();
        let old = old.sanitized();
        assert!(!old.auto_refresh);
        assert_eq!(old.refresh_secs, 60);
    }
    #[test]
    fn repo_keys_ignore_case() {
        let r = Repo {
            owner: "Cli".into(),
            name: "CLI".into(),
            path: None,
        };
        assert_eq!(r.key(), "cli/cli");
        assert_eq!(r.slug(), "Cli/CLI");
    }
}
