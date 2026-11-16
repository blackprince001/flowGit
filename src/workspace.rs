use crate::{
    avatars::Avatars,
    gh::{self, IssueFilter, PrFilter},
    graph::{self, Graph},
    menu::{self as popover, Menu},
    settings::{self, Repo, Saved, Settings, View},
    theme::*,
};
use gpui::*;
use gpui_component::{input::InputState, resizable::ResizableState};
use std::{
    cell::RefCell,
    collections::HashMap,
    path::PathBuf,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};
mod actions;
mod changes;
mod chrome;
mod code;
mod commits;
mod compare;
mod diff;
mod home;
mod issues;
mod mcp;
mod palette;
mod md;
mod syntax;
mod preferences;
mod pulls;
mod releases;
mod review;
mod timeline;
mod viewed;
mod ui;

pub use compare::NewPullRequest;
pub use diff::{NextFile, NextHunk, PreviousFile, PreviousHunk};
use diff::NavPos;
pub use review::FinishReview;

actions!(
    gitflow,
    [
        Refresh,
        ToggleSidebar,
        ToggleBrief,
        CloseTab,
        DuplicateTab,
        NextTab,
        PreviousTab,
        OpenPreferences,
        ShowHome,
        AddLocal,
        AddUrl,
        ShowCode,
        ShowCommits,
        ShowIssues,
        ShowPulls,
        ShowActions,
        ShowReleases,
        DraftRelease,
        ShowChanges,
        Submit,
        Back,
        Quit,
        CommandPalette,
        FindFile
    ]
);

/// A request's lifecycle. Data stays visible while a refresh is in flight.
pub struct Load<T> {
    pub data: Option<T>,
    pub error: Option<String>,
    pub loading: bool,
    /// `data` came from the disk cache and a live request is still running.
    pub stale: bool,
    /// When a view last asked for this, for dropping the least recently used.
    pub used: u64,
}
impl<T> Default for Load<T> {
    fn default() -> Self {
        Self {
            data: None,
            error: None,
            loading: false,
            stale: false,
            used: 0,
        }
    }
}
impl<T> Load<T> {
    /// Still waiting for the first answer.
    pub fn pending(&self) -> bool {
        self.data.is_none() && self.error.is_none()
    }
}

pub struct CommitGraph {
    pub commits: Vec<gh::Commit>,
    pub graph: Graph,
}
pub enum BlobView {
    /// Lines (tabs expanded) and their syntax highlighting, when the language is known.
    Text(Vec<SharedString>, Option<Arc<syntax::Highlights>>),
    Binary(u64),
    TooLarge(u64),
}
/// A diff flattened for a virtualized list: file headers, then numbered lines.
pub struct DiffView {
    /// Shared, so a view with some files collapsed (`collapsed`) costs only its rows.
    pub files: Arc<Vec<gh::FileDiff>>,
    /// Each file's shown lines, tabs expanded, and their syntax colours.
    pub text: Arc<Vec<Vec<SharedString>>>,
    pub highlights: Arc<Vec<Option<syntax::Highlights>>>,
    pub rows: Vec<(usize, Option<usize>)>,
    pub additions: u32,
    pub deletions: u32,
}
impl DiffView {
    pub const MAX_LINES: usize = 1500;
    pub fn new(files: Vec<gh::FileDiff>) -> Self {
        let mut rows = vec![];
        for (f, file) in files.iter().enumerate() {
            rows.push((f, None));
            rows.extend((0..file.lines.len().min(Self::MAX_LINES)).map(|l| (f, Some(l))));
        }
        let text: Vec<Vec<SharedString>> = files
            .iter()
            .map(|f| f.lines.iter().take(Self::MAX_LINES).map(|l| SharedString::from(l.text.replace('\t', "    "))).collect())
            .collect();
        let highlights = files
            .iter()
            .zip(&text)
            .map(|(f, t)| if f.binary { None } else { syntax::highlight_diff(&f.path, &f.lines[..t.len()], t) })
            .collect();
        Self {
            text: Arc::new(text),
            highlights: Arc::new(highlights),
            additions: files.iter().map(|f| f.additions).sum(),
            deletions: files.iter().map(|f| f.deletions).sum(),
            files: Arc::new(files),
            rows,
        }
    }
    /// The same diff with the lines of the files for which `hide` is true left
    /// out, so only their headers remain.
    pub fn collapsed(&self, hide: impl Fn(usize) -> bool) -> Self {
        Self {
            files: self.files.clone(),
            text: self.text.clone(),
            highlights: self.highlights.clone(),
            rows: self.rows.iter().copied().filter(|&(f, line)| line.is_none() || !hide(f)).collect(),
            additions: self.additions,
            deletions: self.deletions,
        }
    }
    pub fn header_row(&self, file: usize) -> Option<usize> {
        self.rows.iter().position(|r| *r == (file, None))
    }
}
pub struct CommitInfo {
    pub message: String,
    pub diff: Arc<DiffView>,
}
/// A job log with a header row wherever the step changes.
pub struct LogView {
    pub rows: Vec<(bool, SharedString)>,
    pub truncated: bool,
}

/// Everything fetched for one repository, shared by all of its tabs.
#[derive(Default)]
pub struct RepoData {
    pub info: Load<gh::RepoInfo>,
    pub branches: Load<Vec<gh::Branch>>,
    pub contributors: Load<Vec<gh::Contributor>>,
    /// People a new pull request can ask for review.
    pub reviewers: Load<Vec<String>>,
    pub activity: Load<Arc<gh::Activity>>,
    pub origin: Load<String>,
    /// Queued or running workflow runs, polled for every repository.
    pub running: Load<usize>,
    pub releases: Load<Vec<gh::Release>>,
    pub workflows: Load<Vec<gh::Workflow>>,
    pub runs: HashMap<Option<u64>, Load<Vec<gh::Run>>>,
    pub issues: HashMap<IssueFilter, Load<Vec<gh::Issue>>>,
    pub issue: HashMap<u64, Load<gh::IssueDetail>>,
    pub pulls: HashMap<PrFilter, Load<Vec<gh::Pull>>>,
    pub pull: HashMap<u64, Load<gh::PullDetail>>,
    pub pull_diff: HashMap<u64, Load<Arc<DiffView>>>,
    /// Which of a pull request's files the viewer has marked as viewed.
    pub viewed: HashMap<u64, Load<gh::ViewedFiles>>,
    /// The compare page's commits and diff, per (base, head).
    pub compare: HashMap<(String, String), Load<Arc<gh::Compare>>>,
    pub compare_diff: HashMap<(String, String), Load<Arc<DiffView>>>,
    pub commits: HashMap<Option<String>, Load<Arc<CommitGraph>>>,
    pub commit: HashMap<String, Load<Arc<CommitInfo>>>,
    pub tree: HashMap<(String, String), Load<Vec<gh::Entry>>>,
    /// Head commit, its runs, and each entry's last commit, per folder.
    pub folder: HashMap<(String, String), Load<Arc<gh::FolderInfo>>>,
    pub blob: HashMap<(String, String), Load<Arc<BlobView>>>,
    pub blame: HashMap<(String, String), Load<Arc<gh::Blame>>>,
    pub jobs: HashMap<u64, Load<Vec<gh::Job>>>,
    pub log: HashMap<u64, Load<Arc<LogView>>>,
    /// Every file path, per branch, for the file finder.
    pub files: HashMap<String, Load<Arc<Vec<String>>>>,
    /// A local clone's working tree.
    pub status: Load<Arc<gh::Status>>,
    /// The diff of each file in the Changes view.
    pub change_diff: HashMap<gh::ChangeRef, Load<Arc<DiffView>>>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub enum PrPage {
    #[default]
    Conversation,
    Commits,
    Files,
    Checks,
}

/// A line of a pull request diff, as GitHub addresses review comments.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LineTarget {
    pub path: String,
    pub line: u32,
    /// The new (right) side for additions and context, old (left) for deletions.
    pub right: bool,
}

/// The open tabs, saved so the next launch reopens them where they were.
#[derive(Default, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct Session {
    #[serde(default)]
    tabs: Vec<SavedTab>,
    #[serde(default)]
    active: Option<usize>,
}
#[derive(Default, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct SavedTab {
    repo: String,
    view: View,
    branch: Option<String>,
    graph_ref: Option<String>,
    path: String,
    file: Option<String>,
    commit: Option<String>,
    pr_filter: PrFilter,
    pr: Option<u64>,
    pr_page: PrPage,
    issue_filter: IssueFilter,
    issue: Option<u64>,
    workflow: Option<u64>,
    run: Option<u64>,
    job: Option<u64>,
    log_open: bool,
    change: Option<gh::ChangeRef>,
}
impl Session {
    const FILE: &str = "session.json";
    fn load() -> Self {
        settings::support_dir()
            .and_then(|d| std::fs::read(d.join(Self::FILE)).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }
    fn save(&self) {
        if let Ok(bytes) = serde_json::to_vec_pretty(self) {
            let _ = settings::write(Self::FILE, bytes);
        }
    }
}
impl SavedTab {
    fn of(t: &Tab) -> Self {
        Self {
            repo: t.repo.clone(),
            view: t.view,
            branch: t.branch.clone(),
            graph_ref: t.graph_ref.clone(),
            path: t.path.clone(),
            file: t.file.clone(),
            commit: t.commit.clone(),
            pr_filter: t.pr_filter,
            pr: t.pr,
            pr_page: t.pr_page,
            issue_filter: t.issue_filter,
            issue: t.issue,
            workflow: t.workflow,
            run: t.run,
            job: t.job,
            log_open: t.log_open,
            change: t.change.clone(),
        }
    }
    fn apply(self, t: &mut Tab) {
        t.branch = self.branch;
        t.graph_ref = self.graph_ref;
        t.path = self.path;
        t.file = self.file;
        t.commit = self.commit;
        t.pr_filter = self.pr_filter;
        t.pr = self.pr;
        t.pr_page = self.pr_page;
        t.issue_filter = self.issue_filter;
        t.issue = self.issue;
        t.workflow = self.workflow;
        t.run = self.run;
        t.job = self.job;
        t.log_open = self.log_open;
        t.change = self.change;
    }
}

pub struct Tab {
    pub id: u64,
    pub repo: String,
    pub view: View,
    /// Code view and remote history: the branch shown; None follows the default.
    pub branch: Option<String>,
    /// Local graph: a single ref, or None for every branch.
    pub graph_ref: Option<String>,
    pub path: String,
    pub file: Option<String>,
    pub commit: Option<String>,
    pub pr_filter: PrFilter,
    pub pr: Option<u64>,
    pub pr_page: PrPage,
    pub issue_filter: IssueFilter,
    pub issue: Option<u64>,
    /// The compare page for a new pull request. Not saved with the session.
    pub new_pr: compare::NewPr,
    pub workflow: Option<u64>,
    pub run: Option<u64>,
    pub job: Option<u64>,
    pub log_open: bool,
    /// Changes view: the file whose diff is shown.
    pub change: Option<gh::ChangeRef>,
    pub search: Entity<InputState>,
    /// Filters the job log shown on the Actions page.
    pub log_search: Entity<InputState>,
    pub pr_title: Entity<InputState>,
    pub pr_body: Entity<InputState>,
    pub composer: Entity<InputState>,
    /// The issue page's comment box, separate from the pull request's.
    pub issue_composer: Entity<InputState>,
    pub commit_message: Entity<InputState>,
    /// The pull request diff line being commented on, and its comment box.
    pub line_target: Option<LineTarget>,
    /// The first line when a range is selected (`line_target` is the last).
    pub line_start: Option<LineTarget>,
    /// The selected diff rows, and the row a shift-click extends from.
    pub line_rows: (usize, usize),
    pub line_anchor: usize,
    pub line_composer: Entity<InputState>,
    pub list_scroll: UniformListScrollHandle,
    pub diff_scroll: UniformListScrollHandle,
    /// The diff row j/k/n/p last scrolled to; steps start there while it is on screen.
    pub diff_nav: Option<NavPos>,
    /// Viewed files of a pull request (number, path) the reader reopened.
    pub viewed_open: std::collections::HashSet<(u64, String)>,
    pub log_scroll: UniformListScrollHandle,
    pub page_scroll: ScrollHandle,
    pub release_list: ListState,
    /// The draft release panel above the Releases list, and its fields.
    pub release_draft: ReleaseDraft,
    pub draft_tag: Entity<InputState>,
    pub draft_branch: Entity<InputState>,
    pub draft_title: Entity<InputState>,
    pub draft_notes: Entity<InputState>,
    /// Notes GitHub wrote for the draft's tag.
    pub generated_notes: Load<gh::GeneratedNotes>,
}

/// The release draft panel. Its fields live on the tab.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ReleaseDraft {
    #[default]
    Closed,
    Open,
}

/// The modal layer over the workspace. Only one is open at a time; dropdown
/// menus (`popup`) are separate and can sit on top of it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Overlay {
    #[default]
    None,
    Prefs,
    /// `resolving` while the typed repository is being looked up.
    AddUrl { resolving: bool },
    /// The dialog that sends a pending review, with the verdict picked so far.
    FinishReview { event: gh::ReviewEvent },
}

struct Notice {
    text: String,
    ok: bool,
}
impl Notice {
    fn warn(text: impl Into<String>) -> Option<Self> {
        Some(Self {
            text: text.into(),
            ok: false,
        })
    }
    fn ok(text: impl Into<String>) -> Option<Self> {
        Some(Self {
            text: text.into(),
            ok: true,
        })
    }
}

#[derive(Clone, Copy, PartialEq)]
enum PrefsPage {
    General,
    GitHub,
    Appearance,
}

pub struct Workspace {
    repos: Vec<Repo>,
    data: HashMap<String, RepoData>,
    tabs: Vec<Tab>,
    /// The tab being shown. None means Home, the screen that is not a repository.
    active: Option<usize>,
    next_id: u64,
    sidebar: bool,
    brief: bool,
    repo_search: Entity<InputState>,
    url_input: Entity<InputState>,
    overlay: Overlay,
    popup: Option<Entity<Menu>>,
    popup_position: Point<Pixels>,
    popup_anchor: Option<&'static str>,
    dismissed: Option<(&'static str, Instant)>,
    anchors: Rc<RefCell<HashMap<&'static str, Bounds<Pixels>>>>,
    font_names: Rc<std::cell::OnceCell<Vec<String>>>,
    notice: Option<Notice>,
    /// A user-started mutation in flight, e.g. "Merging #12".
    busy: Option<String>,
    /// Clear this tab's comment box on the next frame (needs a window).
    pending_clear: Option<u64>,
    /// Likewise for a tab's issue comment box.
    pending_issue_clear: Option<u64>,
    /// Likewise for a tab's line comment box.
    pending_line_clear: Option<u64>,
    /// The Home screen's inbox and notifications. Not per repository.
    home: home::HomeData,
    /// Which pinned Home tab shows while no repository tab is active.
    home_page: home::HomePage,
    /// Notifications already seen, so only new ones raise a desktop alert.
    seen: home::Seen,
    /// When notifications were last polled; None polls now.
    last_notif_poll: Option<Instant>,
    /// Clear the review summary box on the next frame.
    pending_review_clear: bool,
    /// Pending line comments for every pull request, whatever tabs are open.
    drafts: review::ReviewDrafts,
    review_body: Entity<InputState>,
    /// Likewise for a tab's commit message, once the commit is made.
    pending_commit_clear: Option<u64>,
    user: Load<gh::User>,
    gh_version: Load<String>,
    prefs_page: PrefsPage,
    prefs_search: Entity<InputState>,
    gh_path_input: Entity<InputState>,
    avatars: Avatars,
    sidebar_panels: Entity<ResizableState>,
    brief_panels: Entity<ResizableState>,
    last_refresh: Instant,
    last_poll: Instant,
    /// When the Changes view last re-read the working tree; None reads it now.
    last_changes: Option<Instant>,
    /// When every repository was last checked for running Actions; None polls now.
    last_repo_poll: Option<Instant>,
    /// The day hovered in the brief's activity strip.
    activity_hover: Option<usize>,
    /// The hovered line in the file viewer, for inline blame.
    code_hover: Option<usize>,
    /// Views showing cached data while their live request runs.
    refreshing: usize,
    /// Counts fetch requests; stamps `Load::used`.
    tick: u64,
    /// The tabs as last saved.
    session: Option<Session>,
    /// The first row a diff list last drew, so its pinned file header keeps up.
    diff_top: usize,
    focus: FocusHandle,
}

fn input(value: &str, placeholder: &str, window: &mut Window, cx: &mut App) -> Entity<InputState> {
    cx.new(|cx| {
        let mut s = InputState::new(window, cx).placeholder(placeholder.to_string());
        s.set_value(value.to_string(), window, cx);
        s
    })
}

impl Workspace {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let repo_search = input("", "Filter repositories…", window, cx);
        cx.observe(&repo_search, |_, _, cx| cx.notify()).detach();
        let prefs_search = input("", "Search settings…", window, cx);
        cx.observe(&prefs_search, |_, _, cx| cx.notify()).detach();
        let url_input = input("", "owner/name or https://github.com/owner/name", window, cx);
        let gh_path = settings::get(cx).gh_path.clone();
        gh::set_gh_path(&gh_path);
        let gh_path_input = input(&gh_path, "Search /opt/homebrew/bin, /usr/local/bin", window, cx);
        let sidebar_panels = cx.new(|_| ResizableState::default());
        let brief_panels = cx.new(|_| ResizableState::default());
        for panels in [&sidebar_panels, &brief_panels] {
            cx.observe(panels, |_, _, cx| cx.notify()).detach();
        }
        let review_body = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .auto_grow(4, 12)
                .placeholder("Leave a summary · Markdown supported")
        });
        let focus = cx.focus_handle();
        focus.focus(window);
        let mut s = Self {
            repos: Saved::load().repos,
            data: HashMap::new(),
            tabs: vec![],
            active: None,
            next_id: 0,
            sidebar: true,
            brief: true,
            repo_search,
            url_input,
            overlay: Overlay::None,
            popup: None,
            popup_position: Point::default(),
            popup_anchor: None,
            dismissed: None,
            anchors: Rc::default(),
            font_names: Rc::default(),
            notice: None,
            busy: None,
            pending_line_clear: None,
            pending_commit_clear: None,
            pending_clear: None,
            pending_issue_clear: None,
            home: home::HomeData::default(),
            home_page: home::HomePage::default(),
            seen: home::Seen::default(),
            last_notif_poll: None,
            pending_review_clear: false,
            drafts: Default::default(),
            review_body,
            user: Load::default(),
            gh_version: Load::default(),
            prefs_page: PrefsPage::General,
            prefs_search,
            gh_path_input,
            avatars: Avatars::default(),
            sidebar_panels,
            brief_panels,
            last_refresh: Instant::now(),
            last_poll: Instant::now(),
            last_changes: None,
            last_repo_poll: None,
            activity_hover: None,
            code_hover: None,
            refreshing: 0,
            tick: 0,
            session: None,
            diff_top: 0,
            focus,
        };
        // Markdown views load images through the same cache as avatars.
        cx.set_global(s.avatars.clone());
        s.load_account(cx);
        s.restore_session(window, cx);
        if s.tabs.is_empty()
            && let Some(first) = s.repos.first().map(Repo::key)
        {
            s.open_repo(&first, window, cx);
        }
        Self::serve_mcp(window, cx);
        cx.spawn_in(window, async move |this, cx| {
            loop {
                smol::Timer::after(Duration::from_millis(250)).await;
                if this.update(cx, |s, cx| s.tick(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        s
    }

    // ── Tabs ────────────────────────────────────────────────────────────────

    fn current(&self) -> Option<&Tab> {
        self.active.and_then(|i| self.tabs.get(i))
    }
    fn current_mut(&mut self) -> Option<&mut Tab> {
        self.active.and_then(|i| self.tabs.get_mut(i))
    }
    fn repo(&self, key: &str) -> Option<&Repo> {
        self.repos.iter().find(|r| r.key() == key)
    }
    fn is_local(&self, key: &str) -> bool {
        self.repo(key).is_some_and(|r| r.path.is_some())
    }
    fn new_tab(&mut self, repo: &str, view: View, window: &mut Window, cx: &mut Context<Self>) -> Tab {
        self.next_id += 1;
        // A saved or default view the repository no longer has opens on Code.
        let view = if view.available(self.is_local(repo)) { view } else { View::Code };
        let search = input("", "Filter…", window, cx);
        cx.observe(&search, |_, _, cx| cx.notify()).detach();
        let log_search = input("", "Filter log…", window, cx);
        cx.observe(&log_search, |_, _, cx| cx.notify()).detach();
        let composer = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .auto_grow(3, 12)
                .placeholder("Leave a comment · Markdown supported")
        });
        let issue_composer = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .auto_grow(3, 12)
                .placeholder("Leave a comment · Markdown supported")
        });
        let pr_title = input("", "Title", window, cx);
        let pr_body = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .auto_grow(6, 16)
                .placeholder("Describe the change · Markdown supported")
        });
        let line_composer = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .auto_grow(2, 8)
                .placeholder("Comment on this line · Markdown supported")
        });
        let draft_tag = input("", "Tag, such as v1.2.0", window, cx);
        let draft_branch = input("", "Target branch", window, cx);
        let draft_title = input("", "Title", window, cx);
        let draft_notes = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .auto_grow(4, 14)
                .placeholder("Notes · Markdown supported")
        });
        for field in [&draft_tag, &draft_branch, &draft_title, &draft_notes] {
            cx.observe(field, |_, _, cx| cx.notify()).detach();
        }
        let commit_message = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .auto_grow(3, 8)
                .placeholder("Commit message")
        });
        // The Commit button depends on whether there is a message.
        cx.observe(&commit_message, |_, _, cx| cx.notify()).detach();
        Tab {
            id: self.next_id,
            repo: repo.to_string(),
            view,
            branch: None,
            graph_ref: None,
            path: String::new(),
            file: None,
            commit: None,
            pr_filter: PrFilter::Open,
            pr: None,
            pr_page: PrPage::Conversation,
            issue_filter: IssueFilter::Open,
            issue: None,
            new_pr: compare::NewPr::Closed,
            workflow: None,
            run: None,
            job: None,
            log_open: false,
            change: None,
            search,
            log_search,
            pr_title,
            pr_body,
            composer,
            issue_composer,
            commit_message,
            line_target: None,
            line_start: None,
            line_rows: (0, 0),
            line_anchor: 0,
            line_composer,
            list_scroll: UniformListScrollHandle::new(),
            diff_scroll: UniformListScrollHandle::new(),
            diff_nav: None,
            viewed_open: Default::default(),
            log_scroll: UniformListScrollHandle::new(),
            page_scroll: ScrollHandle::new(),
            release_list: ListState::new(0, ListAlignment::Top, px(600.)),
            release_draft: ReleaseDraft::Closed,
            draft_tag,
            draft_branch,
            draft_title,
            draft_notes,
            generated_notes: Load::default(),
        }
    }
    /// Focus the repository's existing tab, or open one on the default view.
    fn open_repo(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(i) = self.tabs.iter().position(|t| t.repo == key) {
            self.active = Some(i);
        } else {
            let view = settings::get(cx).default_view;
            let tab = self.new_tab(key, view, window, cx);
            self.tabs.push(tab);
            self.active = Some(self.tabs.len() - 1);
        }
        cx.notify();
    }
    /// Always open a fresh tab, even when the repository already has one.
    fn open_repo_new_tab(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let view = settings::get(cx).default_view;
        let tab = self.new_tab(key, view, window, cx);
        let at = self.active.map(|i| i + 1).unwrap_or(self.tabs.len());
        self.tabs.insert(at, tab);
        self.active = Some(at);
        cx.notify();
    }
    /// Copy a tab with its view and selection; inputs keep their text.
    fn duplicate(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(src) = self.tabs.get(index) else {
            return;
        };
        let (repo, view) = (src.repo.clone(), src.view);
        let search = src.search.read(cx).value().to_string();
        let log_search = src.log_search.read(cx).value().to_string();
        let mut tab = self.new_tab(&repo, view, window, cx);
        let src = &self.tabs[index];
        tab.branch = src.branch.clone();
        tab.graph_ref = src.graph_ref.clone();
        tab.path = src.path.clone();
        tab.file = src.file.clone();
        tab.commit = src.commit.clone();
        tab.pr_filter = src.pr_filter;
        tab.pr = src.pr;
        tab.pr_page = src.pr_page;
        tab.issue_filter = src.issue_filter;
        tab.issue = src.issue;
        tab.workflow = src.workflow;
        tab.run = src.run;
        tab.job = src.job;
        tab.log_open = src.log_open;
        tab.change = src.change.clone();
        tab.search
            .update(cx, |s, cx| s.set_value(search, window, cx));
        tab.log_search
            .update(cx, |s, cx| s.set_value(log_search, window, cx));
        self.tabs.insert(index + 1, tab);
        self.active = Some(index + 1);
        cx.notify();
    }
    fn close(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.tabs.len() {
            return;
        }
        self.tabs.remove(index);
        self.active = match self.active {
            _ if self.tabs.is_empty() => None,
            Some(a) if a > index => Some(a - 1),
            Some(a) if a == index => Some(index.min(self.tabs.len() - 1)),
            other => other,
        };
        cx.notify();
    }
    fn close_others(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.tabs.len() {
            let keep = self.tabs.remove(index);
            self.tabs = vec![keep];
            self.active = Some(0);
            cx.notify();
        }
    }
    fn set_view(&mut self, view: View, cx: &mut Context<Self>) {
        let local = self.current().is_some_and(|t| self.is_local(&t.repo));
        if !view.available(local) {
            self.notice = Notice::warn(format!("{} needs a local clone of this repository.", view.title()));
        } else if let Some(t) = self.current_mut() {
            t.view = view;
            if view == View::Changes {
                self.last_changes = None;
            }
        }
        cx.notify();
    }

    // ── Data ────────────────────────────────────────────────────────────────

    /// Start `job` unless its slot already has an answer (or `force`) or is loading.
    fn fetch<T: Send + 'static>(
        &mut self,
        repo: &str,
        slot: impl Fn(&mut RepoData) -> &mut Load<T> + 'static,
        force: bool,
        job: impl Fn() -> anyhow::Result<T> + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        self.tick += 1;
        let l = slot(self.data.entry(repo.to_string()).or_default());
        l.used = self.tick;
        if l.loading || (!force && !l.pending()) {
            return;
        }
        l.loading = true;
        // With nothing to show yet, show the last session's copy first.
        let try_cache = l.data.is_none();
        let repo = repo.to_string();
        cx.spawn(async move |this, cx| {
            let mut counted = false;
            let job = if try_cache {
                let (job, cached) = smol::unblock(move || {
                    let r = crate::cache::cached_only(&job);
                    (job, r)
                })
                .await;
                if let Ok(v) = cached {
                    let _ = this.update(cx, |s, cx| {
                        let l = slot(s.data.entry(repo.clone()).or_default());
                        // A slot cleared or filled meanwhile is left alone.
                        if l.loading && l.data.is_none() {
                            l.data = Some(v);
                            l.stale = true;
                            s.refreshing += 1;
                            counted = true;
                            cx.notify();
                        }
                    });
                }
                job
            } else {
                job
            };
            let result = smol::unblock(job).await;
            let _ = this.update(cx, |s, cx| {
                let l = slot(s.data.entry(repo).or_default());
                l.loading = false;
                l.stale = false;
                if counted {
                    s.refreshing = s.refreshing.saturating_sub(1);
                }
                match result {
                    Ok(v) => {
                        l.data = Some(v);
                        l.error = None;
                    }
                    Err(e) => l.error = Some(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn data(&self, repo: &str) -> Option<&RepoData> {
        self.data.get(repo)
    }
    fn default_branch(&self, repo: &str) -> Option<String> {
        self.data(repo)?.info.data.as_ref().map(|i| i.default_branch())
    }
    /// The branch a Code view or remote graph shows.
    fn branch_of(&self, t: &Tab) -> Option<String> {
        t.branch.clone().or_else(|| self.default_branch(&t.repo))
    }
    /// The graph's cache key: a ref, or None for all branches of a local clone.
    fn graph_key(&self, t: &Tab) -> Option<Option<String>> {
        let local = self.repo(&t.repo).is_some_and(|r| r.path.is_some());
        if local {
            Some(t.graph_ref.clone())
        } else {
            self.branch_of(t).map(Some)
        }
    }

    /// Request whatever the active tab shows. With `force`, re-fetch it.
    /// Keep memory flat over a long session: files, diffs and logs viewed a
    /// while ago are dropped (the disk cache brings them back quickly).
    fn trim_memory(&mut self) {
        fn trim<K: Clone + Eq + std::hash::Hash, T>(map: &mut HashMap<K, Load<T>>, cap: usize) {
            if map.len() <= cap {
                return;
            }
            let mut idle: Vec<(u64, K)> = map.iter().filter(|(_, l)| !l.loading).map(|(k, l)| (l.used, k.clone())).collect();
            idle.sort_by_key(|(used, _)| *used);
            let excess = map.len() - cap;
            for (_, k) in idle.into_iter().take(excess) {
                map.remove(&k);
            }
        }
        for d in self.data.values_mut() {
            trim(&mut d.blob, 30);
            trim(&mut d.blame, 30);
            trim(&mut d.tree, 60);
            trim(&mut d.folder, 60);
            trim(&mut d.issue, 30);
            trim(&mut d.pull, 30);
            trim(&mut d.pull_diff, 10);
            trim(&mut d.viewed, 10);
            trim(&mut d.compare, 10);
            trim(&mut d.compare_diff, 5);
            trim(&mut d.commit, 30);
            trim(&mut d.jobs, 30);
            trim(&mut d.log, 10);
            trim(&mut d.change_diff, 10);
        }
    }
    fn ensure(&mut self, force: bool, cx: &mut Context<Self>) {
        self.trim_memory();
        let Some(t) = self.current() else {
            self.ensure_home(force, cx);
            return;
        };
        let key = t.repo.clone();
        let Some(repo) = self.repo(&key).cloned() else {
            return;
        };
        let slug = repo.slug();
        let view = t.view;
        let s = slug.clone();
        self.fetch(&key, |d| &mut d.info, force, move || gh::repo_info(&s), cx);
        if self.brief {
            let s = slug.clone();
            self.fetch(&key, |d| &mut d.contributors, force, move || gh::contributors(&s), cx);
            if let Some(branch) = self.default_branch(&key) {
                let s = slug.clone();
                self.fetch(&key, |d| &mut d.activity, force, move || Ok(Arc::new(gh::activity(&s, &branch)?)), cx);
            }
            if let Some(path) = repo.path.clone() {
                self.fetch(&key, |d| &mut d.origin, false, move || gh::origin_url(&path), cx);
            }
            let s = slug.clone();
            self.fetch(&key, |d| &mut d.releases, force, move || gh::releases(&s), cx);
            let s = slug.clone();
            self.fetch(
                &key,
                |d| d.runs.entry(None).or_default(),
                force,
                move || gh::runs(&s, None),
                cx,
            );
        }
        let Some(t) = self.current() else { return };
        let branch = self.branch_of(t);
        match view {
            View::Code => {
                let s = slug.clone();
                self.fetch(&key, |d| &mut d.branches, force, move || gh::branches(&s), cx);
                let Some(branch) = branch else { return };
                let t = self.current().unwrap();
                let (path, file) = (t.path.clone(), t.file.clone());
                let (s, b, p) = (slug.clone(), branch.clone(), path.clone());
                let k = (branch.clone(), path.clone());
                self.fetch(
                    &key,
                    move |d| d.tree.entry(k.clone()).or_default(),
                    force,
                    move || gh::tree(&s, &p, &b),
                    cx,
                );
                // Last commits need the folder's entries first.
                let paths = self
                    .data(&key)
                    .and_then(|d| d.tree.get(&(branch.clone(), path.clone())))
                    .and_then(|l| l.data.as_ref())
                    .map(|entries| entries.iter().map(|e| e.path.clone()).collect::<Vec<_>>());
                if let Some(paths) = paths {
                    let (s, b) = (slug.clone(), branch.clone());
                    let k = (branch.clone(), path.clone());
                    self.fetch(
                        &key,
                        move |d| d.folder.entry(k.clone()).or_default(),
                        force,
                        move || Ok(Arc::new(gh::folder_info(&s, &b, &paths)?)),
                        cx,
                    );
                }
                let readme = self
                    .data(&key)
                    .and_then(|d| d.tree.get(&(branch.clone(), path.clone())))
                    .and_then(|l| l.data.as_ref())
                    .and_then(|entries| {
                        entries
                            .iter()
                            .find(|e| !e.is_dir() && e.name.to_lowercase().starts_with("readme"))
                            .map(|e| e.path.clone())
                    });
                if let Some(file) = file.clone() {
                    let (s, b, p) = (slug.clone(), branch.clone(), file.clone());
                    let k = (branch.clone(), file);
                    self.fetch(
                        &key,
                        move |d| d.blame.entry(k.clone()).or_default(),
                        force,
                        move || Ok(Arc::new(gh::blame(&s, &b, &p)?)),
                        cx,
                    );
                }
                for target in file.into_iter().chain(readme) {
                    let (s, b, p) = (slug.clone(), branch.clone(), target.clone());
                    let k = (branch.clone(), target);
                    self.fetch(
                        &key,
                        move |d| d.blob.entry(k.clone()).or_default(),
                        force,
                        move || load_blob(&s, &p, &b),
                        cx,
                    );
                }
            }
            View::Commits => {
                let s = slug.clone();
                self.fetch(&key, |d| &mut d.branches, force, move || gh::branches(&s), cx);
                let Some(graph_key) = self.graph_key(self.current().unwrap()) else {
                    return;
                };
                let limit = settings::get(cx).commit_limit;
                let (s, path, r) = (slug.clone(), repo.path.clone(), graph_key.clone());
                let k = graph_key.clone();
                self.fetch(
                    &key,
                    move |d| d.commits.entry(k.clone()).or_default(),
                    force,
                    move || load_graph(&s, path.as_deref(), r.as_deref(), limit),
                    cx,
                );
                if let Some(sha) = self.current().unwrap().commit.clone() {
                    let (s, path, k) = (slug.clone(), repo.path.clone(), sha.clone());
                    self.fetch(
                        &key,
                        move |d| d.commit.entry(k.clone()).or_default(),
                        false,
                        move || {
                            let c = gh::commit_detail(&s, path.as_deref(), &sha)?;
                            Ok(Arc::new(CommitInfo {
                                message: c.message,
                                diff: Arc::new(DiffView::new(c.files)),
                            }))
                        },
                        cx,
                    );
                }
            }
            View::Issues => {
                let t = self.current().unwrap();
                let (filter, issue) = (t.issue_filter, t.issue);
                let s = slug.clone();
                self.fetch(
                    &key,
                    move |d| d.issues.entry(filter).or_default(),
                    force,
                    move || gh::issues(&s, filter),
                    cx,
                );
                if let Some(n) = issue {
                    let s = slug.clone();
                    self.fetch(
                        &key,
                        move |d| d.issue.entry(n).or_default(),
                        force,
                        move || gh::issue(&s, n),
                        cx,
                    );
                }
            }
            View::Pulls => {
                let t = self.current().unwrap();
                let (filter, pr, page) = (t.pr_filter, t.pr, t.pr_page);
                let s = slug.clone();
                self.fetch(
                    &key,
                    move |d| d.pulls.entry(filter).or_default(),
                    force,
                    move || gh::pulls(&s, filter),
                    cx,
                );
                if let Some(n) = pr {
                    let s = slug.clone();
                    self.fetch(
                        &key,
                        move |d| d.pull.entry(n).or_default(),
                        force,
                        move || gh::pull(&s, n),
                        cx,
                    );
                    if page == PrPage::Files {
                        let s = slug.clone();
                        self.fetch(
                            &key,
                            move |d| d.pull_diff.entry(n).or_default(),
                            force,
                            move || Ok(Arc::new(DiffView::new(gh::pull_diff(&s, n)?))),
                            cx,
                        );
                        let s = slug.clone();
                        self.fetch(&key, move |d| d.viewed.entry(n).or_default(), force, move || gh::viewed_files(&s, n), cx);
                    }
                }
                self.ensure_compare(&key, &slug, force, cx);
            }
            View::Actions => {
                let t = self.current().unwrap();
                let (workflow, run, log) = (t.workflow, t.run, t.log_open);
                let job = t.job.or_else(|| {
                    let jobs = self.data(&key)?.jobs.get(&run?)?.data.as_ref()?;
                    jobs.first().map(|j| j.database_id)
                });
                let s = slug.clone();
                self.fetch(&key, |d| &mut d.workflows, force, move || gh::workflows(&s), cx);
                let s = slug.clone();
                self.fetch(&key, |d| &mut d.branches, force, move || gh::branches(&s), cx);
                let s = slug.clone();
                self.fetch(&key, |d| &mut d.releases, force, move || gh::releases(&s), cx);
                let s = slug.clone();
                self.fetch(
                    &key,
                    move |d| d.runs.entry(workflow).or_default(),
                    force,
                    move || gh::runs(&s, workflow),
                    cx,
                );
                if let Some(run) = run {
                    let s = slug.clone();
                    self.fetch(
                        &key,
                        move |d| d.jobs.entry(run).or_default(),
                        force,
                        move || gh::jobs(&s, run),
                        cx,
                    );
                    if let (Some(job), true) = (job, log) {
                        let s = slug.clone();
                        self.fetch(
                            &key,
                            move |d| d.log.entry(job).or_default(),
                            force,
                            move || Ok(Arc::new(load_log(gh::job_log(&s, job)?))),
                            cx,
                        );
                    }
                }
            }
            View::Releases => {
                let s = slug.clone();
                self.fetch(&key, |d| &mut d.releases, force, move || gh::releases(&s), cx);
            }
            View::Changes => self.ensure_changes(&key, repo.path.clone(), force, cx),
        }
    }
    fn refresh(&mut self, _: &Refresh, _: &mut Window, cx: &mut Context<Self>) {
        self.last_refresh = Instant::now();
        // A refresh also re-reads the selected commit and the branch list.
        if let Some(t) = self.current() {
            let key = t.repo.clone();
            let sha = t.commit.clone();
            if let Some(d) = self.data.get_mut(&key)
                && let Some(sha) = sha
            {
                d.commit.remove(&sha);
            }
        }
        self.ensure(true, cx);
        self.poll_repos(cx);
        if self.user.error.is_some() {
            self.load_account(cx);
        }
        cx.notify();
    }
    /// Check every repository for queued or running workflow runs.
    fn poll_repos(&mut self, cx: &mut Context<Self>) {
        self.last_repo_poll = Some(Instant::now());
        for repo in self.repos.clone() {
            let slug = repo.slug();
            self.fetch(&repo.key(), |d| &mut d.running, true, move || gh::active_runs(&slug), cx);
        }
    }
    /// Running workflow runs for the sidebar: the open tab's run list when it has
    /// one (it is kept current), otherwise the background poll.
    pub(super) fn running_runs(&self, key: &str) -> usize {
        let Some(d) = self.data(key) else { return 0 };
        match d.runs.get(&None).and_then(|l| l.data.as_ref()) {
            Some(runs) => runs.iter().filter(|r| r.active()).count(),
            None => d.running.data.unwrap_or(0),
        }
    }
    /// Periodic work: repaint for avatars, auto-refresh, and poll running Actions.
    /// Reopen last launch's tabs, skipping repositories since removed.
    fn restore_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let saved = Session::load();
        let mut active = None;
        for (i, st) in saved.tabs.into_iter().enumerate() {
            if self.repo(&st.repo).is_none() {
                continue;
            }
            let mut tab = self.new_tab(&st.repo, st.view, window, cx);
            st.apply(&mut tab);
            if saved.active == Some(i) {
                active = Some(self.tabs.len());
            }
            self.tabs.push(tab);
        }
        // Home was showing when it was saved (no active tab, but tabs exist).
        self.active = match saved.active {
            None => None,
            Some(_) => active.or((!self.tabs.is_empty()).then_some(0)),
        };
        self.session = Some(self.session_now());
    }
    fn session_now(&self) -> Session {
        Session {
            tabs: self.tabs.iter().map(SavedTab::of).collect(),
            active: self.active,
        }
    }
    /// Save the tabs whenever they change (checked on the UI tick).
    fn save_session(&mut self) {
        let now = self.session_now();
        if self.session.as_ref() != Some(&now) {
            now.save();
            self.session = Some(now);
        }
    }
    fn tick(&mut self, cx: &mut Context<Self>) {
        self.save_session();
        if self.avatars.take_dirty() {
            cx.notify();
        }
        if self.overlay == Overlay::Prefs {
            return;
        }
        if self.last_notif_poll.is_none_or(|t| t.elapsed() > home::NOTIFICATION_POLL) {
            self.poll_notifications(cx);
        }
        self.poll_changes(cx);
        let (on, every) = (settings::get(cx).auto_refresh, settings::get(cx).refresh_secs);
        let interval = Duration::from_secs(every as u64);
        if self.last_repo_poll.is_none() || (on && self.last_repo_poll.is_some_and(|t| t.elapsed() > interval)) {
            self.poll_repos(cx);
        }
        if on && self.last_refresh.elapsed() > interval {
            self.last_refresh = Instant::now();
            self.ensure(true, cx);
            return;
        }
        let Some(t) = self.current() else { return };
        if t.view != View::Actions || self.last_poll.elapsed() < Duration::from_secs(8) {
            return;
        }
        let active = self
            .data(&t.repo)
            .and_then(|d| d.runs.get(&t.workflow))
            .and_then(|l| l.data.as_ref())
            .is_some_and(|runs| runs.iter().any(gh::Run::active));
        if active {
            self.last_poll = Instant::now();
            self.ensure(true, cx);
        }
    }
    fn load_account(&mut self, cx: &mut Context<Self>) {
        self.user = Load {
            loading: true,
            ..Default::default()
        };
        cx.spawn(async move |this, cx| {
            let (user, version) = smol::unblock(|| (gh::current_user(), gh::version())).await;
            let _ = this.update(cx, |s, cx| {
                s.user.loading = false;
                match user {
                    Ok(u) => s.user.data = Some(u),
                    Err(e) => s.user.error = Some(e.to_string()),
                }
                match version {
                    Ok(v) => s.gh_version.data = Some(v),
                    Err(e) => s.gh_version.error = Some(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Run a user-initiated change, report it, then refresh what it touched.
    fn mutate(
        &mut self,
        label: String,
        job: impl FnOnce() -> anyhow::Result<String> + Send + 'static,
        done: impl FnOnce(&mut Self, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) {
        if self.busy.is_some() {
            return;
        }
        self.busy = Some(label.clone());
        self.notice = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = smol::unblock(job).await;
            let _ = this.update(cx, |s, cx| {
                s.busy = None;
                match result {
                    Ok(out) => {
                        let out = out.trim();
                        s.notice = Notice::ok(if out.is_empty() || out.len() > 200 {
                            format!("{label} — done.")
                        } else {
                            out.lines().last().unwrap_or(out).trim_start_matches('✓').trim().to_string()
                        });
                        done(s, cx);
                    }
                    Err(e) => s.notice = Notice::warn(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn pr_action(&mut self, number: u64, action: gh::PrAction, cx: &mut Context<Self>) {
        let Some(t) = self.current() else { return };
        let Some(repo) = self.repo(&t.repo).cloned() else {
            return;
        };
        let key = repo.key();
        let slug = repo.slug();
        let label = match &action {
            gh::PrAction::Comment(_) => format!("Commenting on #{number}"),
            gh::PrAction::Approve => format!("Approving #{number}"),
            gh::PrAction::RequestChanges(_) => format!("Requesting changes on #{number}"),
            gh::PrAction::Merge { .. } => format!("Merging #{number}"),
            gh::PrAction::Close => format!("Closing #{number}"),
            gh::PrAction::Reopen => format!("Reopening #{number}"),
            gh::PrAction::Ready => format!("Marking #{number} ready"),
            gh::PrAction::Draft => format!("Converting #{number} to draft"),
            gh::PrAction::Checkout(_) => format!("Checking out #{number}"),
            gh::PrAction::LineComment { path, line, .. } => format!("Commenting on {path}:{line}"),
            gh::PrAction::Reply { .. } => format!("Commenting on #{number}"),
            gh::PrAction::SubmitReview { .. } => format!("Submitting review on #{number}"),
        };
        let clears_line = matches!(action, gh::PrAction::LineComment { .. } | gh::PrAction::Reply { .. });
        let clears_composer = matches!(
            action,
            gh::PrAction::Comment(_) | gh::PrAction::RequestChanges(_)
        );
        let tab = t.id;
        self.mutate(
            label,
            move || gh::pr_action(&slug, number, &action),
            move |s, cx| {
                if let Some(d) = s.data.get_mut(&key) {
                    d.pull.remove(&number);
                    d.pulls.clear();
                    if !clears_line {
                        d.pull_diff.remove(&number);
                    }
                    d.commits.clear();
                }
                if clears_composer {
                    s.pending_clear = Some(tab);
                }
                if clears_line {
                    // The diff itself is unchanged; keep it and its scroll position.
                    s.pending_line_clear = Some(tab);
                }
                s.ensure(false, cx);
            },
            cx,
        );
    }
    fn run_action(&mut self, run: u64, action: gh::RunAction, cx: &mut Context<Self>) {
        let Some(t) = self.current() else { return };
        let Some(repo) = self.repo(&t.repo).cloned() else {
            return;
        };
        let (key, slug) = (repo.key(), repo.slug());
        let label = match &action {
            gh::RunAction::Rerun => "Re-running all jobs".to_string(),
            gh::RunAction::RerunFailed => "Re-running failed jobs".into(),
            gh::RunAction::Cancel => "Cancelling run".into(),
            gh::RunAction::Dispatch { reference, .. } => format!("Starting workflow on {reference}"),
        };
        self.mutate(
            label,
            move || gh::run_action(&slug, run, &action),
            move |s, cx| {
                if let Some(d) = s.data.get_mut(&key) {
                    d.runs.clear();
                    d.jobs.remove(&run);
                }
                s.last_poll = Instant::now();
                s.ensure(false, cx);
            },
            cx,
        );
    }
    fn fetch_local(&mut self, cx: &mut Context<Self>) {
        let Some(t) = self.current() else { return };
        let Some(repo) = self.repo(&t.repo).cloned() else {
            return;
        };
        let Some(path) = repo.path.clone() else { return };
        let key = repo.key();
        self.mutate(
            format!("Fetching {}", repo.name),
            move || gh::fetch(&path).map(|_| String::new()),
            move |s, cx| {
                if let Some(d) = s.data.get_mut(&key) {
                    d.commits.clear();
                    d.branches = Load::default();
                }
                s.ensure(false, cx);
            },
            cx,
        );
    }

    // ── Repositories ────────────────────────────────────────────────────────

    fn add_repo(&mut self, repo: Repo, window: &mut Window, cx: &mut Context<Self>) {
        let key = repo.key();
        match self.repos.iter_mut().find(|r| r.key() == key) {
            // Re-adding a remote repo from a clone upgrades it to local.
            Some(existing) => {
                if repo.path.is_some() {
                    existing.path = repo.path;
                    self.data.remove(&key);
                }
            }
            None => self.repos.push(repo),
        }
        self.save_repos();
        self.open_repo(&key, window, cx);
    }
    fn save_repos(&mut self) {
        if let Err(e) = Saved::save(&self.repos) {
            self.notice = Notice::warn(format!("Could not save repositories: {e}"));
        }
    }
    fn remove_repo(&mut self, key: &str, cx: &mut Context<Self>) {
        self.repos.retain(|r| r.key() != key);
        self.data.remove(key);
        while let Some(i) = self.tabs.iter().position(|t| t.repo == key) {
            self.close(i, cx);
        }
        self.save_repos();
        cx.notify();
    }
    fn add_local(&mut self, _: &AddLocal, window: &mut Window, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: true,
            prompt: Some("Add repository".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = receiver.await else {
                return;
            };
            for path in paths {
                let found = smol::unblock({
                    let path = path.clone();
                    move || {
                        let top = gh::git(&path, &["rev-parse", "--show-toplevel"])
                            .map(|p| PathBuf::from(p.trim()))
                            .unwrap_or(path);
                        gh::local_remote(&top).map(|(o, n)| (o, n, top))
                    }
                })
                .await;
                let _ = this.update_in(cx, |s, w, cx| match found {
                    Ok((owner, name, top)) => s.add_repo(
                        Repo {
                            owner,
                            name,
                            path: Some(top),
                        },
                        w,
                        cx,
                    ),
                    Err(e) => {
                        s.notice = Notice::warn(format!("{}: {e}", path.display()));
                        cx.notify();
                    }
                });
            }
        })
        .detach();
    }
    fn open_add_url(&mut self, _: &AddUrl, window: &mut Window, cx: &mut Context<Self>) {
        self.popup = None;
        self.overlay = Overlay::AddUrl { resolving: false };
        self.url_input.update(cx, |s, cx| {
            s.set_value("", window, cx);
            s.focus(window, cx);
        });
        cx.notify();
    }
    /// Add the typed repository; with `clone`, clone it locally first.
    fn submit_url(&mut self, clone: bool, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.url_input.read(cx).value().to_string();
        let Some((owner, name)) = gh::parse_slug(&text) else {
            self.notice = Notice::warn("Enter owner/name or a github.com URL.");
            cx.notify();
            return;
        };
        self.overlay = Overlay::AddUrl { resolving: true };
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = smol::unblock(move || gh::resolve(&owner, &name)).await;
            let _ = this.update_in(cx, |s, w, cx| {
                if let Overlay::AddUrl { resolving } = &mut s.overlay {
                    *resolving = false;
                }
                match result {
                    Ok((owner, name)) if clone => {
if matches!(s.overlay, Overlay::AddUrl { .. }) {
                            s.overlay = Overlay::None;
                        }
                        s.clone_repo(owner, name, w, cx);
                    }
                    Ok((owner, name)) => {
if matches!(s.overlay, Overlay::AddUrl { .. }) {
                            s.overlay = Overlay::None;
                        }
                        s.add_repo(
                            Repo {
                                owner,
                                name,
                                path: None,
                            },
                            w,
                            cx,
                        );
                    }
                    Err(e) => s.notice = Notice::warn(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }
    /// Ask for a parent folder, `gh repo clone` into `<folder>/<name>`, then add
    /// (or upgrade) the repository as a local clone.
    pub(super) fn clone_repo(&mut self, owner: String, name: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Clone Here".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = receiver.await else {
                return;
            };
            let Some(parent) = paths.into_iter().next() else {
                return;
            };
            let dest = parent.join(&name);
            let slug = format!("{owner}/{name}");
            let started = this
                .update(cx, |s, cx| {
                    if s.busy.is_some() {
                        return false;
                    }
                    s.busy = Some(format!("Cloning {slug}"));
                    s.notice = None;
                    cx.notify();
                    true
                })
                .unwrap_or(false);
            if !started {
                return;
            }
            let target = dest.clone();
            let result = smol::unblock(move || {
                if target.exists() {
                    anyhow::bail!("{} already exists. Pick another folder, or add it with Add local repository.", target.display());
                }
                gh::gh(&["repo", "clone", &slug, &target.to_string_lossy()])
            })
            .await;
            let _ = this.update_in(cx, |s, w, cx| {
                s.busy = None;
                match result {
                    Ok(_) => {
                        s.notice = Notice::ok(format!("Cloned into {}", dest.display()));
                        s.add_repo(
                            Repo {
                                owner,
                                name,
                                path: Some(dest),
                            },
                            w,
                            cx,
                        );
                    }
                    Err(e) => s.notice = Notice::warn(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    // ── Preferences ─────────────────────────────────────────────────────────

    fn update_settings(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut Settings)) {
        let mut next = settings::get(cx).clone();
        f(&mut next);
        let next = next.sanitized();
        if let Err(e) = next.save() {
            self.notice = Notice::warn(format!("Could not save settings: {e}"));
        }
        let commits_changed = next.commit_limit != settings::get(cx).commit_limit;
        gh::set_gh_path(&next.gh_path);
        cx.set_global(next);
        if commits_changed {
            for d in self.data.values_mut() {
                d.commits.clear();
            }
        }
        crate::theme::refresh(cx);
        cx.notify();
    }

    // ── Popovers (as in CloudWatch v2) ──────────────────────────────────────

    #[allow(clippy::too_many_arguments)]
    fn open_popover(
        &mut self,
        anchor: &'static str,
        fallback: Point<Pixels>,
        search: Option<&str>,
        width: f32,
        build: popover::Builder,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let menu = cx.new(|cx| Menu::new(build, search, px(width), window, cx));
        menu.update(cx, |m, cx| m.focus(window, cx));
        cx.subscribe_in(&menu, window, |s, _, _: &DismissEvent, window, cx| {
            s.popup = None;
            s.dismissed = s.popup_anchor.take().map(|a| (a, Instant::now()));
            if s.overlay != Overlay::Prefs {
                s.focus.focus(window);
            }
            cx.notify();
        })
        .detach();
        self.popup_anchor = Some(anchor);
        self.popup_position = self
            .anchors
            .borrow()
            .get(anchor)
            .map(|b| b.bottom_left() + point(px(0.), px(4.)))
            .unwrap_or(fallback);
        self.popup = Some(menu);
        cx.notify();
    }

    // ── Keyboard actions ────────────────────────────────────────────────────

    fn toggle_sidebar(&mut self, _: &ToggleSidebar, _: &mut Window, cx: &mut Context<Self>) {
        self.sidebar = !self.sidebar;
        cx.notify();
    }
    fn toggle_brief(&mut self, _: &ToggleBrief, _: &mut Window, cx: &mut Context<Self>) {
        self.brief = !self.brief;
        cx.notify();
    }
    fn close_tab(&mut self, _: &CloseTab, window: &mut Window, cx: &mut Context<Self>) {
        if self.overlay != Overlay::None {
            self.overlay = Overlay::None;
        } else if let Some(i) = self.active {
            self.close(i, cx);
        } else {
            window.remove_window();
        }
        cx.notify();
    }
    fn duplicate_tab(&mut self, _: &DuplicateTab, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(i) = self.active {
            self.duplicate(i, window, cx);
        }
    }
    fn cycle(&mut self, step: isize, cx: &mut Context<Self>) {
        if let Some(i) = self.active
            && !self.tabs.is_empty()
        {
            self.active =
                Some((i as isize + step).rem_euclid(self.tabs.len() as isize) as usize);
            cx.notify();
        }
    }
    fn open_prefs(&mut self, _: &OpenPreferences, window: &mut Window, cx: &mut Context<Self>) {
        self.popup = None;
        self.overlay = Overlay::Prefs;
        self.prefs_search.update(cx, |s, cx| s.focus(window, cx));
        cx.notify();
    }
    /// Step back out of a detail page: file → folder, PR/run/commit → list.
    fn back(&mut self, _: &Back, _: &mut Window, cx: &mut Context<Self>) {
        let Some(t) = self.current_mut() else { return };
        match t.view {
            View::Code if t.file.is_some() => t.file = None,
            View::Code if !t.path.is_empty() => {
                t.path = t
                    .path
                    .rsplit_once('/')
                    .map(|(p, _)| p.to_string())
                    .unwrap_or_default()
            }
            View::Commits => t.commit = None,
            View::Issues => t.issue = None,
            View::Pulls if t.new_pr.is_open() => t.new_pr = compare::NewPr::Closed,
            View::Pulls => (t.pr, t.line_target) = (None, None),
            View::Actions if t.log_open => t.log_open = false,
            View::Actions => t.run = None,
            _ => {}
        }
        cx.notify();
    }
    fn submit(&mut self, _: &Submit, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.overlay, Overlay::AddUrl { .. }) {
            self.submit_url(false, window, cx);
            return;
        }
        if matches!(self.overlay, Overlay::FinishReview { .. }) {
            self.finish_review(cx);
            return;
        }
        let Some(t) = self.current() else { return };
        if t.view == View::Pulls && t.new_pr.is_open() {
            self.create_pull(cx);
            return;
        }
        if t.view == View::Changes {
            self.commit_staged(cx);
            return;
        }
        if let (View::Pulls, Some(n), PrPage::Files, true) = (t.view, t.pr, t.pr_page, t.line_target.is_some()) {
            self.submit_line(n, cx);
            return;
        }
        if let (View::Issues, Some(n)) = (t.view, t.issue) {
            let body = t.issue_composer.read(cx).value().trim().to_string();
            if !body.is_empty() {
                self.issue_action(n, gh::IssueAction::Comment(body), cx);
            }
            return;
        }
        if let (View::Pulls, Some(n)) = (t.view, t.pr) {
            let body = t.composer.read(cx).value().trim().to_string();
            if !body.is_empty() {
                self.pr_action(n, gh::PrAction::Comment(body), cx);
            }
        }
    }
}

impl Workspace {
    /// Post the line comment box: a reply when the line already has a thread.
    fn submit_line(&mut self, n: u64, cx: &mut Context<Self>) {
        let Some(t) = self.current() else { return };
        let Some(target) = t.line_target.clone() else { return };
        let body = t.line_composer.read(cx).value().trim().to_string();
        if body.is_empty() {
            return;
        }
        let Some(pr) = self.data(&t.repo).and_then(|d| d.pull.get(&n)).and_then(|l| l.data.as_ref()) else {
            return;
        };
        let existing = pr.threads.iter().rev().find(|th| diff::thread_target(th).as_ref() == Some(&target));
        // With a review under way, a new comment joins it instead of posting alone.
        if existing.is_none() && self.drafts.count(&t.repo, n) > 0 {
            self.add_draft_line(n, cx);
            return;
        }
        let action = match existing {
            Some(th) => gh::PrAction::Reply { to: th.root.id, body },
            None => gh::PrAction::LineComment {
                commit: pr.head_ref_oid.clone(),
                start: t.line_start.as_ref().map(|s| (s.line, s.right)),
                path: target.path,
                line: target.line,
                right: target.right,
                body,
            },
        };
        self.pr_action(n, action, cx);
    }
}

fn load_blob(slug: &str, path: &str, branch: &str) -> anyhow::Result<Arc<BlobView>> {
    Ok(Arc::new(match gh::blob(slug, path, branch)? {
        gh::Blob::Text(t) => {
            let lines = t
                .lines()
                .map(|l| SharedString::from(l.replace('\t', "    ")))
                .collect::<Vec<_>>();
            let highlights = syntax::language(path)
                .and_then(|lang| syntax::highlight(lang, &lines))
                .map(Arc::new);
            BlobView::Text(lines, highlights)
        }
        gh::Blob::Binary(n) => BlobView::Binary(n),
        gh::Blob::TooLarge(n) => BlobView::TooLarge(n),
    }))
}
fn load_graph(
    slug: &str,
    path: Option<&std::path::Path>,
    reference: Option<&str>,
    limit: u32,
) -> anyhow::Result<Arc<CommitGraph>> {
    let mut commits = match path {
        // A branch from GitHub may exist only as a remote-tracking ref locally.
        Some(p) => gh::local_log(p, reference, limit).or_else(|e| match reference {
            Some(r) => gh::local_log(p, Some(&format!("origin/{r}")), limit).map_err(|_| e),
            None => Err(e),
        })?,
        None => gh::remote_log(slug, reference.unwrap_or("HEAD"), limit)?,
    };
    if path.is_none() {
        // The REST history has no decorations; mark branch tips we know about.
        if let Ok(branches) = gh::branches(slug) {
            let tips = graph::tips(
                &branches
                    .into_iter()
                    .map(|b| (b.name, b.commit.sha))
                    .collect::<Vec<_>>(),
            );
            for c in &mut commits {
                if let Some(names) = tips.get(&c.sha) {
                    c.refs = names
                        .iter()
                        .map(|n| gh::Ref {
                            name: n.clone(),
                            kind: if Some(n.as_str()) == reference {
                                gh::RefKind::Head
                            } else {
                                gh::RefKind::Branch
                            },
                        })
                        .collect();
                }
            }
        }
    }
    let graph = graph::layout(commits.iter().map(|c| (c.sha.as_str(), c.parents.as_slice())));
    Ok(Arc::new(CommitGraph { commits, graph }))
}
fn load_log(lines: Vec<gh::LogLine>) -> LogView {
    let truncated = lines.len() >= gh::LOG_LIMIT;
    let mut rows = Vec::with_capacity(lines.len() + 32);
    let mut step: Option<String> = None;
    for l in lines {
        if step.as_deref() != Some(&l.step) {
            rows.push((true, SharedString::from(l.step.clone())));
            step = Some(l.step);
        }
        rows.push((false, SharedString::from(l.text)));
    }
    LogView { rows, truncated }
}
