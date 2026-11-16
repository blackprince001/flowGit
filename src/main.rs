mod avatars;
mod cache;
mod gh;
mod graph;
mod icons;
mod menu;
mod settings;
mod theme;
mod util;
mod workspace;
use gpui::*;
use gpui_component::{Root, TitleBar};
use workspace::*;

fn main() {
    // Trim the disk cache off the main thread.
    std::thread::spawn(cache::prune);
    Application::new().with_assets(icons::Assets).run(move |cx| {
        gpui_component::init(cx);
        theme::init(cx);
        cx.bind_keys([
            KeyBinding::new("cmd-r", Refresh, None),
            KeyBinding::new("cmd-b", ToggleSidebar, None),
            KeyBinding::new("cmd-alt-b", ToggleBrief, None),
            KeyBinding::new("cmd-w", CloseTab, None),
            KeyBinding::new("cmd-shift-d", DuplicateTab, None),
            KeyBinding::new("cmd-shift-]", NextTab, None),
            KeyBinding::new("cmd-shift-[", PreviousTab, None),
            KeyBinding::new("ctrl-tab", NextTab, None),
            KeyBinding::new("ctrl-shift-tab", PreviousTab, None),
            KeyBinding::new("cmd-,", OpenPreferences, None),
            KeyBinding::new("cmd-o", AddLocal, None),
            KeyBinding::new("cmd-shift-o", AddUrl, None),
            KeyBinding::new("cmd-1", ShowCode, None),
            KeyBinding::new("cmd-2", ShowCommits, None),
            KeyBinding::new("cmd-3", ShowPulls, None),
            KeyBinding::new("cmd-4", ShowActions, None),
            KeyBinding::new("cmd-5", ShowReleases, None),
            KeyBinding::new("cmd-enter", Submit, None),
            KeyBinding::new("cmd-[", Back, None),
            KeyBinding::new("cmd-shift-n", NewPullRequest, None),
            KeyBinding::new("cmd-shift-enter", FinishReview, None),
            KeyBinding::new("cmd-q", Quit, None),
        ]);
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.set_menus(vec![
            Menu {
                name: "Gitflow".into(),
                items: vec![
                    MenuItem::action("Preferences…", OpenPreferences),
                    MenuItem::separator(),
                    MenuItem::action("Quit Gitflow", Quit),
                ],
            },
            Menu {
                name: "File".into(),
                items: vec![
                    MenuItem::action("Add Local Repository…", AddLocal),
                    MenuItem::action("Add from GitHub URL…", AddUrl),
                    MenuItem::separator(),
                    MenuItem::action("Duplicate Tab", DuplicateTab),
                    MenuItem::action("Close Tab", CloseTab),
                ],
            },
            Menu {
                name: "View".into(),
                items: vec![
                    MenuItem::action("Code", ShowCode),
                    MenuItem::action("Commits", ShowCommits),
                    MenuItem::action("Pull Requests", ShowPulls),
                    MenuItem::action("Actions", ShowActions),
                    MenuItem::action("Releases", ShowReleases),
                    MenuItem::separator(),
                    MenuItem::action("Refresh", Refresh),
                    MenuItem::action("Toggle Sidebar", ToggleSidebar),
                    MenuItem::action("Toggle Brief", ToggleBrief),
                ],
            },
        ]);
        let bounds = Bounds::centered(None, size(px(1440.), px(900.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitleBar::title_bar_options()),
                window_min_size: Some(size(px(980.), px(600.))),
                ..Default::default()
            },
            move |window, cx| {
                window.set_window_title("Gitflow");
                theme::sync(window, cx);
                window.observe_window_appearance(theme::sync).detach();
                let view = cx.new(|cx| Workspace::new(window, cx));
                cx.new(|cx| Root::new(view, window, cx))
            },
        )
        .expect("Could not open Gitflow window");
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        cx.activate(true);
    });
}
