//! The command palette (⌘K) and file finder (⌘P). Both are searchable popover
//! menus built from live workspace state, so they need no state of their own.
use super::*;
use crate::{
    icons::Glyph,
    menu::{Entry, Item},
};
use gpui_component::{Icon, IconName};

const WIDTH: f32 = 560.;

type Run = fn(&mut Workspace, &mut Window, &mut Context<Workspace>);

/// Commands that map straight onto an action, with the shortcut shown beside them.
fn commands() -> Vec<(&'static str, &'static str, Run)> {
    vec![
        ("Go to Home", "⌘0", |s, w, cx| s.show_home(&ShowHome, w, cx)),
        ("Find file…", "⌘P", |s, w, cx| s.find_file(&FindFile, w, cx)),
        ("Mark all notifications as read", "", |s, _, cx| s.mark_all_read(cx)),
        ("Refresh", "⌘R", |s, w, cx| s.refresh(&Refresh, w, cx)),
        ("Draft a release…", "⇧⌘N", |s, w, cx| {
            if s.current().is_some() {
                s.set_view(View::Releases, cx);
                s.open_release_draft(w, cx);
            }
        }),
        ("Toggle sidebar", "⌘B", |s, w, cx| s.toggle_sidebar(&ToggleSidebar, w, cx)),
        ("Toggle brief panel", "⌥⌘B", |s, w, cx| s.toggle_brief(&ToggleBrief, w, cx)),
        ("Duplicate tab", "⇧⌘D", |s, w, cx| s.duplicate_tab(&DuplicateTab, w, cx)),
        ("Close tab", "⌘W", |s, w, cx| s.close_tab(&CloseTab, w, cx)),
        ("Open local repository…", "⌘O", |s, w, cx| s.add_local(&AddLocal, w, cx)),
        ("Add repository from GitHub…", "⇧⌘O", |s, w, cx| s.open_add_url(&AddUrl, w, cx)),
        ("Preferences…", "⌘,", |s, w, cx| s.open_prefs(&OpenPreferences, w, cx)),
    ]
}

impl Workspace {
    /// A popover centred near the top of the window, where palettes live.
    fn palette_origin(window: &Window) -> Point<Pixels> {
        let w = window.viewport_size().width;
        point((w - px(WIDTH)) / 2., px(72.))
    }

    pub(super) fn open_palette(&mut self, _: &CommandPalette, window: &mut Window, cx: &mut Context<Self>) {
        let weak = cx.weak_entity();
        self.open_popover(
            "palette",
            Self::palette_origin(window),
            Some("Type a command, repository, tab or pull request…"),
            WIDTH,
            Rc::new(move |cx| {
                let Some(ws) = weak.upgrade() else { return vec![] };
                ws.read(cx).palette_entries(&weak)
            }),
            window,
            cx,
        );
    }

    fn palette_entries(&self, weak: &WeakEntity<Self>) -> Vec<Entry> {
        let mut out = vec![];
        let current = self.current();

        if let Some(t) = current {
            out.push(Entry::Label("This repository".into()));
            for (i, view) in View::ALL.into_iter().enumerate() {
                let w = weak.clone();
                out.push(Entry::Item(
                    Item::new(format!("Go to {}", view.title()))
                        .icon(super::chrome::view_glyph(view))
                        .detail(format!("⌘{}", i + 1))
                        .checked(t.view == view)
                        .on_click(move |_, cx| {
                            let _ = w.update(cx, |s, cx| s.set_view(view, cx));
                        }),
                ));
            }
            // Pull requests already loaded for this repository; nothing is fetched here.
            if let Some(pulls) = self
                .data(&t.repo)
                .and_then(|d| d.pulls.get(&PrFilter::Open))
                .and_then(|l| l.data.as_ref())
            {
                for p in pulls {
                    let (w, n) = (weak.clone(), p.number);
                    out.push(Entry::Item(
                        Item::new(format!("#{n} {}", p.title))
                            .icon(Glyph::PullRequest)
                            .detail(p.author.login.clone())
                            .on_click(move |_, cx| {
                                let _ = w.update(cx, |s, cx| s.show_pull(n, cx));
                            }),
                    ));
                }
            }
        }

        if self.tabs.len() > 1 {
            out.push(Entry::Label("Open tabs".into()));
            for (i, t) in self.tabs.iter().enumerate() {
                let w = weak.clone();
                out.push(Entry::Item(
                    Item::new(t.repo.clone())
                        .icon(super::chrome::view_glyph(t.view))
                        .detail(t.view.title())
                        .checked(self.active == Some(i))
                        .on_click(move |_, cx| {
                            let _ = w.update(cx, |s, cx| {
                                if i < s.tabs.len() {
                                    s.active = Some(i);
                                    cx.notify();
                                }
                            });
                        }),
                ));
            }
        }

        out.push(Entry::Label("Repositories".into()));
        for repo in &self.repos {
            let (w, key) = (weak.clone(), repo.key());
            out.push(Entry::Item(
                Item::new(repo.slug())
                    .icon(if repo.path.is_some() { Glyph::FolderGit.into() } else { Icon::from(IconName::GitHub) })
                    .detail(if repo.path.is_some() { "local" } else { "" })
                    .on_click(move |win, cx| {
                        let _ = w.update(cx, |s, cx| s.open_repo(&key, win, cx));
                    }),
            ));
        }

        out.push(Entry::Label("Commands".into()));
        for (label, keys, run) in commands() {
            let w = weak.clone();
            out.push(Entry::Item(Item::new(label).detail(keys).on_click(move |win, cx| {
                // After the menu closes, so commands that open popovers or move focus win.
                let w = w.clone();
                win.defer(cx, move |win, cx| {
                    let _ = w.update(cx, |s, cx| run(s, win, cx));
                });
            })));
        }
        out
    }

    /// Open a pull request in the current tab, on its conversation.
    pub(super) fn show_pull(&mut self, n: u64, cx: &mut Context<Self>) {
        if let Some(t) = self.current_mut() {
            t.view = View::Pulls;
            t.pr = Some(n);
            t.line_target = None;
            t.pr_page = PrPage::Conversation;
        }
        cx.notify();
    }

    pub(super) fn find_file(&mut self, _: &FindFile, window: &mut Window, cx: &mut Context<Self>) {
        let Some(t) = self.current() else { return };
        let Some(repo) = self.repo(&t.repo).cloned() else { return };
        let Some(branch) = self.branch_of(t) else { return };
        let key = repo.key();
        // A local clone lists its working tree, unless the tab browses another branch.
        let local = repo.path.clone().filter(|_| t.branch.is_none());
        let (slug, b) = (repo.slug(), branch.clone());
        self.fetch(
            &key,
            {
                let branch = branch.clone();
                move |d| d.files.entry(branch.clone()).or_default()
            },
            false,
            move || Ok(Arc::new(gh::files(&slug, local.as_deref(), &b)?)),
            cx,
        );
        let weak = cx.weak_entity();
        self.open_popover(
            "file-finder",
            Self::palette_origin(window),
            Some("Find a file by name or path…"),
            WIDTH,
            Rc::new(move |cx| {
                let Some(ws) = weak.upgrade() else { return vec![] };
                let s = ws.read(cx);
                let Some(load) = s.data(&key).and_then(|d| d.files.get(&branch)) else {
                    return vec![];
                };
                let Some(files) = &load.data else {
                    let text = match &load.error {
                        Some(e) => format!("Couldn't list files: {e}"),
                        None => "Loading files…".into(),
                    };
                    return vec![Entry::Label(text.into())];
                };
                files
                    .iter()
                    .map(|path| {
                        let (w, path) = (weak.clone(), path.clone());
                        let name = path.rsplit('/').next().unwrap_or(&path).to_string();
                        Entry::Item(Item::new(name).detail(path.clone()).on_click(move |_, cx| {
                            let _ = w.update(cx, |s, cx| s.show_file(&path, cx));
                        }))
                    })
                    .collect()
            }),
            window,
            cx,
        );
    }

    /// Open a file in the current tab's Code view.
    pub(super) fn show_file(&mut self, path: &str, cx: &mut Context<Self>) {
        if let Some(t) = self.current_mut() {
            t.view = View::Code;
            t.path = path.rsplit_once('/').map(|(dir, _)| dir.to_string()).unwrap_or_default();
            t.file = Some(path.to_string());
        }
        cx.notify();
    }
}
