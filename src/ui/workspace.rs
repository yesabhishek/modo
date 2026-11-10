//! The window's root view: project lifecycle, tabs, panels, and commands.
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use gpui_kit::component::{
    ActiveTheme, IconName, TitleBar, WindowExt, h_flex,
    input::{InputEvent, InputState},
    menu::{ContextMenuExt, PopupMenuItem},
    notification::Notification,
    v_flex,
};
use gpui_kit::{
    AnyElement, App, AppContext, Context, Entity, FocusHandle, Focusable, InteractiveElement,
    IntoElement, KeyBinding, MouseButton, ParentElement, PathPromptOptions, PromptLevel, Render,
    ScrollHandle, ScrollStrategy, SharedString, StatefulInteractiveElement, Styled, Subscription,
    Task, UniformListScrollHandle, WeakEntity, Window, actions, deferred, div,
    prelude::FluentBuilder,
};

use super::{
    editor::FileTab, icon_button, is_mac, palette::Palette, r, review::Review,
    settings_view::SettingsView, terminal::TerminalPanel,
};
use crate::{
    files::{self, Entry, FileIndex},
    git,
    settings::{ProjectSession, Session, Settings},
};

actions!(
    modo,
    [
        OpenFolder,
        NewWindow,
        CloseWindow,
        QuickOpen,
        CommandPalette,
        SwitchProject,
        GoToLine,
        FindInFiles,
        ToggleSidebar,
        ToggleTerminal,
        NewTerminal,
        MaximizeTerminal,
        SplitTerminalHorizontal,
        SplitTerminalVertical,
        ShowTodos,
        ToggleWindowColor,
        CloseAllTabs,
        CloseOtherTabs,
        CloseTabsLeft,
        CloseTabsRight,
        Save,
        SaveAll,
        FormatDocument,
        CloseTab,
        NextTab,
        PreviousTab,
        OpenSettings,
        CheckForUpdates,
        ReviewChanges,
        ShowFiles,
        RefreshProject,
        RevealActiveFile,
        RevealInFinder,
        CopyPath,
        CopyRelativePath,
        ToggleSoftWrap,
        ToggleMarkdownPreview,
        ToggleReviewLayout,
        ZoomIn,
        ZoomOut,
        ZoomReset,
        Quit,
        Minimize,
        Zoom,
        Hide,
        HideOthers,
        ShowAll,
        About,
        Dismiss,
    ]
);

const CONTEXT: &str = "Workspace";
/// The pause in typing that triggers an automatic save.
const AUTO_SAVE_DELAY: Duration = Duration::from_millis(250);
/// Editors kept mounted. Each holds its text, undo history, and syntax tree;
/// past this many, the tabs unused longest go back to being a path and are
/// read again when next shown.
pub const LOADED_TABS: usize = 12;
/// How long a save's own file events are recognised as its echo.
const SAVE_ECHO: Duration = Duration::from_secs(3);

pub fn init(cx: &mut App) {
    super::data_view::init(cx);
    let cmd = if is_mac() { "cmd" } else { "ctrl" };
    // Bindings live for the whole process, so leaking the few strings is fine.
    let k = |keys: &str| -> &'static str { Box::leak(keys.replace("cmd", cmd).into_boxed_str()) };
    cx.bind_keys([
        KeyBinding::new(k("cmd-o"), OpenFolder, None),
        KeyBinding::new(k("cmd-shift-n"), NewWindow, None),
        KeyBinding::new(k("cmd-p"), QuickOpen, Some(CONTEXT)),
        KeyBinding::new(k("cmd-shift-p"), CommandPalette, Some(CONTEXT)),
        KeyBinding::new(k("cmd-k"), CommandPalette, Some(CONTEXT)),
        KeyBinding::new("ctrl-g", GoToLine, Some(CONTEXT)),
        KeyBinding::new(k("cmd-shift-f"), FindInFiles, Some(CONTEXT)),
        KeyBinding::new(k("cmd-b"), ToggleSidebar, Some(CONTEXT)),
        KeyBinding::new(k("cmd-j"), ToggleTerminal, Some(CONTEXT)),
        KeyBinding::new("ctrl-`", ToggleTerminal, Some(CONTEXT)),
        KeyBinding::new("ctrl-shift-`", NewTerminal, Some(CONTEXT)),
        KeyBinding::new(k("cmd-shift-j"), MaximizeTerminal, Some(CONTEXT)),
        KeyBinding::new(k("cmd-s"), Save, Some(CONTEXT)),
        KeyBinding::new("alt-shift-f", FormatDocument, Some(CONTEXT)),
        KeyBinding::new(k("cmd-alt-s"), SaveAll, Some(CONTEXT)),
        KeyBinding::new(k("cmd-w"), CloseTab, Some(CONTEXT)),
        KeyBinding::new("ctrl-tab", NextTab, Some(CONTEXT)),
        KeyBinding::new("ctrl-shift-tab", PreviousTab, Some(CONTEXT)),
        KeyBinding::new(k("cmd-shift-]"), NextTab, Some(CONTEXT)),
        KeyBinding::new(k("cmd-shift-["), PreviousTab, Some(CONTEXT)),
        KeyBinding::new(k("cmd-,"), OpenSettings, Some(CONTEXT)),
        KeyBinding::new(k("cmd-shift-g"), ReviewChanges, Some(CONTEXT)),
        KeyBinding::new(k("cmd-shift-e"), ShowFiles, Some(CONTEXT)),
        KeyBinding::new("alt-z", ToggleSoftWrap, Some(CONTEXT)),
        KeyBinding::new(k("cmd-shift-v"), ToggleMarkdownPreview, Some(CONTEXT)),
        KeyBinding::new(k("cmd-="), ZoomIn, None),
        KeyBinding::new(k("cmd-+"), ZoomIn, None),
        KeyBinding::new(k("cmd--"), ZoomOut, None),
        KeyBinding::new(k("cmd-0"), ZoomReset, None),
        KeyBinding::new("escape", Dismiss, Some("Overlay")),
        KeyBinding::new("escape", Dismiss, Some("NewFile")),
    ]);
    if is_mac() {
        // The editor binds ⌘⇧F to in-file replace; project search wins, like
        // VS Code and Cursor, and replace moves to ⌥⌘F.
        cx.bind_keys([
            KeyBinding::new("cmd-shift-f", FindInFiles, Some("Input")),
            KeyBinding::new(
                "cmd-alt-f",
                gpui_kit::component::input::Replace,
                Some("Input"),
            ),
        ]);
        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("cmd-m", Minimize, None),
            KeyBinding::new("cmd-h", Hide, None),
            KeyBinding::new("cmd-alt-h", HideOthers, None),
            KeyBinding::new("cmd-shift-w", CloseWindow, None),
        ]);
    } else {
        cx.bind_keys([KeyBinding::new("ctrl-q", Quit, None)]);
    }
    cx.on_action(|_: &NewWindow, cx| crate::open_window(None, cx));
    cx.on_action(|_: &ZoomIn, cx| Settings::update(cx, |s| s.zoom = (s.zoom + 0.1).min(1.6)));
    cx.on_action(|_: &ZoomOut, cx| Settings::update(cx, |s| s.zoom = (s.zoom - 0.1).max(0.7)));
    cx.on_action(|_: &ZoomReset, cx| Settings::update(cx, |s| s.zoom = 1.0));
    cx.on_action(|_: &Hide, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    cx.on_action(|_: &Quit, cx| quit(cx));
    cx.on_action(|_: &OpenFolder, cx| {
        // Reached only when no window is focused (all windows closed on macOS).
        prompt_open_folder_new_window(cx);
    });
    // The Dock's Quit, a logout, or an installer closing the app does not go
    // through `quit` and its prompts. With auto save on, edits still reach
    // the disk; either way the session is remembered.
    cx.on_app_quit(|cx| {
        for window in cx.windows() {
            let _ = window.update(cx, |root, _, cx| {
                if let Some(workspace) = root
                    .downcast::<gpui_kit::base::Root>()
                    .ok()
                    .and_then(|root| root.read(cx).view().clone().downcast::<Workspace>().ok())
                {
                    workspace.update(cx, |workspace, cx| workspace.save_before_exit(cx));
                }
            });
        }
        async {}
    })
    .detach();
}

fn prompt_open_folder_new_window(cx: &mut App) {
    let paths = cx.prompt_for_paths(PathPromptOptions {
        files: false,
        directories: true,
        multiple: false,
        prompt: Some("Open".into()),
    });
    cx.spawn(async move |cx| {
        if let Ok(Ok(Some(paths))) = paths.await
            && let Some(path) = paths.into_iter().next()
        {
            cx.update(|cx| crate::open_window(Some(path), cx));
        }
    })
    .detach();
}

/// Close every window; each one asks about its own unsaved files first.
pub fn quit(cx: &mut App) {
    let windows = cx.windows();
    if windows.is_empty() {
        crate::updater::finish_quit(cx);
        return;
    }
    cx.spawn(async move |cx| {
        for window in windows {
            let Ok(Some(workspace)) = window.update(cx, |root, _, cx| {
                root.downcast::<gpui_kit::base::Root>()
                    .ok()
                    .and_then(|root| root.read(cx).view().clone().downcast::<Workspace>().ok())
            }) else {
                continue;
            };
            let proceed = window
                .update(cx, |_, window, cx| {
                    workspace.update(cx, |workspace, cx| workspace.confirm_discard(window, cx))
                })
                .ok();
            let Some(proceed) = proceed else { continue };
            if !proceed.await {
                crate::updater::cancel_restart();
                return;
            }
        }
        cx.update(crate::updater::finish_quit);
    })
    .detach();
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Panel {
    Files,
    Search,
    Changes,
    Todos,
}

// A window holds a handful of tabs, so the size difference does not matter.
#[allow(clippy::large_enum_variant)]
pub enum Tab {
    File(FileTab),
    /// A file from the saved session that has not been shown yet. It is only
    /// a path until its tab is first selected, so a session with many tabs
    /// reads one file at launch instead of all of them.
    Pending(PendingTab),
    Review,
}

pub struct PendingTab {
    pub path: PathBuf,
    /// A line to land on once the file has loaded.
    line: Option<u32>,
    /// Where the cursor was and whether the preview was showing, for a tab
    /// that was unloaded to save memory.
    view: Option<(gpui_kit::component::input::Position, bool)>,
    loading: bool,
}

fn file_name(path: &Path) -> SharedString {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
        .into()
}

#[derive(Clone, Copy)]
pub enum TabClose {
    All,
    Others,
    Left,
    Right,
}

#[derive(Clone, PartialEq, Eq)]
enum TabTarget {
    File(PathBuf),
    Review,
}

impl Tab {
    fn target(&self) -> TabTarget {
        match self {
            Self::File(file) => TabTarget::File(file.path.clone()),
            Self::Pending(pending) => TabTarget::File(pending.path.clone()),
            Self::Review => TabTarget::Review,
        }
    }
}

impl TabClose {
    fn includes(self, ix: usize, anchor: Option<usize>) -> bool {
        match self {
            Self::All => true,
            Self::Others => anchor.is_some_and(|a| a != ix),
            Self::Left => anchor.is_some_and(|a| ix < a),
            Self::Right => anchor.is_some_and(|a| ix > a),
        }
    }
}

pub struct Project {
    pub root: PathBuf,
    pub name: SharedString,
}

#[derive(Default)]
pub struct Tree {
    pub children: HashMap<PathBuf, Vec<Entry>>,
    pub expanded: HashSet<PathBuf>,
    pub rows: Vec<TreeRow>,
    pub selected: Option<PathBuf>,
    pub scroll: UniformListScrollHandle,
    pub error: Option<SharedString>,
}

#[derive(Clone)]
pub struct TreeRow {
    pub path: PathBuf,
    pub name: SharedString,
    pub depth: usize,
    pub directory: bool,
    pub expanded: bool,
    pub hidden: bool,
    pub ignored: bool,
}

#[derive(Default)]
pub struct GitState {
    pub repo: Option<git::Repository>,
    /// Folders (relative) that contain at least one change.
    pub dirty_dirs: HashSet<PathBuf>,
    pub generation: u64,
}

pub struct Workspace {
    pub focus: FocusHandle,
    pub project: Option<Project>,
    pub tabs: Vec<Tab>,
    pub active: Option<usize>,
    /// Keeps the active tab in view when the bar holds more tabs than fit.
    tab_scroll: ScrollHandle,
    pub sidebar_open: bool,
    pub sidebar_width: f32,
    pub panel: Panel,
    pub tree: Tree,
    pub git: GitState,
    pub index: Option<Arc<FileIndex>>,
    pub indexing: Option<Task<()>>,
    pub review: Entity<Review>,
    pub changes_scroll: UniformListScrollHandle,
    pub search: Entity<super::sidebar::SearchPanel>,
    pub terminal: Entity<TerminalPanel>,
    pub palette: Option<Entity<Palette>>,
    pub settings: Option<Entity<SettingsView>>,
    pub status: Option<SharedString>,
    pub new_file: Option<NewFile>,
    /// Files the watcher saw change; reloaded on the next frame with a window.
    pending_reload: Option<HashSet<PathBuf>>,
    /// Files Modo just wrote, so the watcher can tell its own saves from an
    /// agent's edits and skip the reload, re-index, and tree refresh.
    recent_saves: HashMap<PathBuf, Instant>,
    pub todos: Vec<crate::todos::Todo>,
    pub completed_todos: HashSet<String>,
    pub todo_filter: Option<String>,
    pub show_completed_todos: bool,
    pub todo_loading: bool,
    pub todo_loaded: bool,
    pub todo_limited: bool,
    pub todo_error: Option<SharedString>,
    todo_task: Option<Task<()>>,
    pub window_color: Option<bool>,
    closing_tabs: bool,
    /// The first window, drawn empty until launch knows which project to open.
    launching: bool,
    save_prompts: usize,
    project_generation: u64,
    activation_generation: u64,
    restore_task: Option<Task<()>>,
    restoring_paths: Vec<PathBuf>,
    restoring_active: Option<PathBuf>,
    watcher_start: Option<Task<()>>,
    watcher: Option<Watcher>,
    _subscriptions: Vec<Subscription>,
    weak: WeakEntity<Self>,
}

pub struct NewFile {
    pub parent: PathBuf,
    pub input: Entity<InputState>,
    pub error: Option<SharedString>,
    pub creating: bool,
    _subscription: Subscription,
}

struct Watcher {
    _watcher: notify::RecommendedWatcher,
    _task: Task<()>,
}

impl Workspace {
    pub fn new(root: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let review = cx.new(|cx| Review::new(window, cx));
        let search = cx.new(|cx| super::sidebar::SearchPanel::new(window, cx));
        let terminal = cx.new(TerminalPanel::new);
        let mut subscriptions = vec![
            cx.observe_window_appearance(window, |_, window, cx| {
                crate::theme::appearance_changed(window.appearance(), window, cx);
            }),
            cx.subscribe_in(&review, window, Self::on_review_event),
            cx.subscribe_in(&search, window, Self::on_search_event),
            cx.observe_global_in::<Settings>(window, |this, window, cx| {
                this.apply_settings(cx);
                for ix in 0..this.tabs.len() {
                    this.schedule_auto_save(ix, window, cx);
                }
            }),
            cx.observe_global::<crate::updater::UpdateState>(|_, cx| cx.notify()),
            cx.observe(&terminal, |_, _, cx| cx.notify()),
            cx.observe_window_activation(window, |this, window, cx| {
                if window.is_window_active() {
                    this.refresh_git(cx);
                    this.reload_clean_files(None, window, cx);
                    crate::updater::check_replaced(cx);
                } else {
                    // Leaving for another app (a terminal, an agent): have
                    // everything on disk before it looks.
                    for ix in 0..this.tabs.len() {
                        this.flush_auto_save(ix, window, cx);
                    }
                }
            }),
        ];
        subscriptions.push(cx.observe(&review, |_, _, cx| cx.notify()));
        let focus = cx.focus_handle();
        let weak = cx.weak_entity();
        window.on_window_should_close(cx, {
            let weak = weak.clone();
            move |window, cx| {
                let Some(this) = weak.upgrade() else {
                    return true;
                };
                if !this.read(cx).has_unsaved(cx) {
                    this.update(cx, |this, cx| this.remember_session(cx));
                    return true;
                }
                let proceed = this.update(cx, |this, cx| this.confirm_discard(window, cx));
                let weak = weak.clone();
                window
                    .spawn(cx, async move |cx| {
                        if proceed.await {
                            let _ = cx.update(|window, cx| {
                                if let Some(this) = weak.upgrade() {
                                    this.update(cx, |this, cx| this.discard_all(cx));
                                }
                                window.remove_window();
                            });
                        }
                    })
                    .detach();
                false
            }
        });
        let settings = Settings::get(cx);
        let this = Self {
            focus,
            project: None,
            tabs: Vec::new(),
            active: None,
            tab_scroll: ScrollHandle::new(),
            sidebar_open: true,
            sidebar_width: 240.,
            panel: Panel::Files,
            tree: Tree::default(),
            git: GitState::default(),
            index: None,
            indexing: None,
            review,
            changes_scroll: UniformListScrollHandle::new(),
            search,
            terminal,
            palette: None,
            settings: None,
            status: None,
            new_file: None,
            pending_reload: None,
            recent_saves: HashMap::new(),
            todos: Vec::new(),
            completed_todos: HashSet::new(),
            todo_filter: None,
            show_completed_todos: false,
            todo_loading: false,
            todo_loaded: false,
            todo_limited: false,
            todo_error: None,
            todo_task: None,
            window_color: None,
            closing_tabs: false,
            launching: false,
            save_prompts: 0,
            project_generation: 0,
            activation_generation: 0,
            restore_task: None,
            restoring_paths: Vec::new(),
            restoring_active: None,
            watcher_start: None,
            watcher: None,
            _subscriptions: subscriptions,
            weak,
        };
        let _ = settings;
        if let Some(root) = root {
            // Open after the window has mounted, so errors can show as notifications.
            cx.defer_in(window, move |this, window, cx| {
                this.open_path(root, window, cx)
            });
        }
        this
    }

    /// The window opened at launch, before the project to show is decided.
    pub fn launching(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut this = Self::new(None, window, cx);
        this.launching = true;
        this
    }

    pub fn finish_launch(
        &mut self,
        path: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.launching = false;
        if let Some(path) = path {
            self.open_path(path, window, cx);
        }
        cx.notify();
    }

    // ---- Project lifecycle -------------------------------------------------

    /// Open a folder as the project, or a file inside its parent folder.
    pub fn open_path(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let path = dunce::canonicalize(&path).unwrap_or(path);
        if path.is_file() {
            let root = git_root_or_parent(&path);
            if self.project.as_ref().map(|p| &p.root) != Some(&root) {
                self.open_project(root, window, cx);
            }
            self.open_file(path, None, window, cx);
        } else if path.is_dir() {
            self.open_project(path, window, cx);
        } else {
            self.notify_error(format!("{} does not exist.", path.display()), window, cx);
        }
    }

    pub fn open_project(&mut self, root: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if self.project.is_some() {
            self.remember_session(cx);
        }
        self.project_generation += 1;
        self.new_file = None;
        self.restore_task = None;
        self.restoring_paths.clear();
        self.restoring_active = None;
        let name: SharedString = root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| root.display().to_string())
            .into();
        window.set_window_title(&name);
        let restored = Session::get_project(cx, &root);
        self.todo_task = None;
        self.todos.clear();
        self.todo_loaded = false;
        self.todo_loading = false;
        self.todo_error = None;
        self.completed_todos = restored
            .as_ref()
            .map(|s| s.completed_todos.clone())
            .unwrap_or_default();
        self.window_color = restored.as_ref().and_then(|s| s.window_color);
        self.tabs
            .retain(|tab| matches!(tab, Tab::File(file) if file.dirty));
        self.active = (!self.tabs.is_empty()).then_some(0);
        self.tree = Tree::default();
        self.tree.expanded.insert(root.clone());
        if let Some(session) = &restored {
            self.tree.expanded.extend(
                session
                    .expanded
                    .iter()
                    .filter(|p| p.starts_with(&root))
                    .cloned(),
            );
            if session.sidebar_width >= 180. {
                self.sidebar_width = session.sidebar_width;
            }
        }
        self.git = GitState {
            generation: self.git.generation + 1,
            ..GitState::default()
        };
        self.index = None;
        self.panel = Panel::Files;
        self.project = Some(Project {
            root: root.clone(),
            name,
        });
        let expanded: Vec<PathBuf> = self.tree.expanded.iter().cloned().collect();
        for dir in expanded {
            self.load_dir(dir, cx);
        }
        let review_base = restored.as_ref().and_then(|s| s.review_base.clone());
        self.review.update(cx, |review, cx| {
            review.set_root(Some(root.clone()), review_base, cx)
        });
        self.search
            .update(cx, |search, cx| search.set_root(Some(root.clone()), cx));
        self.terminal
            .update(cx, |terminal, _| terminal.set_cwd(root.clone()));
        self.refresh_git(cx);
        self.start_index(cx);
        self.start_watcher(cx);
        if let Some(session) = restored
            && Settings::get(cx).restore_session
        {
            self.restore_files(session.open_files, session.active_file, window, cx);
            if session.terminal_open {
                self.terminal.update(cx, |t, cx| t.show(window, cx));
            }
        }
        self.remember_session(cx);
        cx.notify();
    }

    pub fn root(&self) -> Option<&Path> {
        self.project.as_ref().map(|p| p.root.as_path())
    }

    pub fn remember_session(&self, cx: &mut App) {
        let Some(project) = &self.project else { return };
        let mut open_files: Vec<_> = self
            .tabs
            .iter()
            .filter_map(|tab| match tab {
                Tab::File(file) => Some(file.path.clone()),
                Tab::Pending(pending) => Some(pending.path.clone()),
                Tab::Review => None,
            })
            .collect();
        for path in &self.restoring_paths {
            if !open_files.contains(path) {
                open_files.push(path.clone());
            }
        }
        let session = ProjectSession {
            root: project.root.clone(),
            open_files,
            active_file: self.restoring_active.clone().or_else(|| {
                match self.tabs.get(self.active?)? {
                    Tab::File(file) => Some(file.path.clone()),
                    Tab::Pending(pending) => Some(pending.path.clone()),
                    Tab::Review => None,
                }
            }),
            expanded: self
                .tree
                .expanded
                .iter()
                .filter(|p| **p != project.root)
                .cloned()
                .collect(),
            sidebar_width: self.sidebar_width,
            terminal_open: self.terminal.read(cx).open,
            review_base: self.review.read(cx).chosen_base(),
            review_commit: self.review.read(cx).chosen_commit(),
            reviewed_files: self.review.read(cx).reviewed_files(),
            completed_todos: self.completed_todos.clone(),
            window_color: self.window_color,
        };
        Session::remember(cx, session);
    }

    fn apply_settings(&mut self, cx: &mut Context<Self>) {
        let settings = Settings::get(cx).clone();
        for tab in &self.tabs {
            if let Tab::File(file) = tab {
                file.apply_settings(&settings, cx);
            }
        }
        self.review
            .update(cx, |review, cx| review.apply_settings(&settings, cx));
        if settings.watch_files != (self.watcher.is_some() || self.watcher_start.is_some()) {
            self.start_watcher(cx);
        }
        self.rebuild_rows(cx);
        cx.notify();
    }

    // ---- Tree --------------------------------------------------------------

    pub fn load_dir(&mut self, dir: PathBuf, cx: &mut Context<Self>) {
        let root = self.root().map(Path::to_path_buf);
        cx.spawn(async move |this, cx| {
            let read = {
                let dir = dir.clone();
                cx.background_executor()
                    .spawn(async move { files::read_directory(&dir) })
                    .await
            };
            this.update(cx, |this, cx| {
                if this.root() != root.as_deref() {
                    return;
                }
                match read {
                    Ok(entries) => {
                        this.tree.children.insert(dir, entries);
                    }
                    Err(error) => {
                        if this.root() == Some(dir.as_path()) {
                            this.tree.error = Some(error.into());
                        }
                        this.tree.expanded.remove(&dir);
                    }
                }
                this.rebuild_rows(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn rebuild_rows(&mut self, cx: &App) {
        let Some(root) = self.root().map(Path::to_path_buf) else {
            self.tree.rows.clear();
            return;
        };
        let settings = Settings::get(cx);
        let show_hidden = settings.show_hidden_files;
        let show_ignored = settings.show_ignored_files;
        let mut rows = Vec::with_capacity(self.tree.rows.len().max(64));
        fn walk(
            tree: &Tree,
            dir: &Path,
            depth: usize,
            options: (bool, bool),
            parent_ignored: bool,
            rows: &mut Vec<TreeRow>,
        ) {
            let Some(children) = tree.children.get(dir) else {
                return;
            };
            for entry in children {
                let ignored = parent_ignored || entry.ignored;
                if (!options.0 && entry.is_hidden()) || (!options.1 && ignored) {
                    continue;
                }
                let expanded = entry.directory && tree.expanded.contains(&entry.path);
                rows.push(TreeRow {
                    path: entry.path.clone(),
                    name: entry.name.clone().into(),
                    depth,
                    directory: entry.directory,
                    expanded,
                    hidden: entry.is_hidden(),
                    ignored,
                });
                if expanded {
                    walk(tree, &entry.path, depth + 1, options, ignored, rows);
                }
            }
        }
        walk(
            &self.tree,
            &root,
            0,
            (show_hidden, show_ignored),
            false,
            &mut rows,
        );
        self.tree.rows = rows;
    }

    pub fn toggle_dir(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if !self.tree.expanded.remove(&path) {
            self.tree.expanded.insert(path.clone());
            if !self.tree.children.contains_key(&path) {
                self.load_dir(path.clone(), cx);
            }
        }
        self.tree.selected = Some(path);
        self.rebuild_rows(cx);
        cx.notify();
    }

    pub fn collapse_all(&mut self, cx: &mut Context<Self>) {
        if let Some(root) = self.root().map(Path::to_path_buf) {
            self.tree.expanded.retain(|p| p == &root);
            self.rebuild_rows(cx);
            cx.notify();
        }
    }

    /// Expand every ancestor of `path` and scroll it into view.
    pub fn reveal(&mut self, path: &Path, cx: &mut Context<Self>) {
        let Some(root) = self.root().map(Path::to_path_buf) else {
            return;
        };
        if !path.starts_with(&root) {
            return;
        }
        let mut missing = Vec::new();
        for ancestor in path.ancestors().skip(1) {
            if !ancestor.starts_with(&root) {
                break;
            }
            if self.tree.expanded.insert(ancestor.to_path_buf())
                && !self.tree.children.contains_key(ancestor)
            {
                missing.push(ancestor.to_path_buf());
            }
        }
        self.tree.selected = Some(path.to_path_buf());
        if missing.is_empty() {
            self.rebuild_rows(cx);
            if let Some(ix) = self.tree.rows.iter().position(|row| row.path == path) {
                self.tree.scroll.scroll_to_item(ix, ScrollStrategy::Center);
            }
        } else {
            for dir in missing {
                self.load_dir(dir, cx);
            }
            let target = path.to_path_buf();
            cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(80))
                    .await;
                this.update(cx, |this, _| {
                    if let Some(ix) = this.tree.rows.iter().position(|row| row.path == target) {
                        this.tree.scroll.scroll_to_item(ix, ScrollStrategy::Center);
                    }
                })
                .ok();
            })
            .detach();
        }
        cx.notify();
    }

    // ---- Git ---------------------------------------------------------------

    pub fn refresh_git(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.root().map(Path::to_path_buf) else {
            return;
        };
        self.git.generation += 1;
        let generation = self.git.generation;
        cx.spawn(async move |this, cx| {
            let repo = cx
                .background_executor()
                .spawn(async move { git::inspect(&root).ok().flatten() })
                .await;
            this.update(cx, |this, cx| {
                if this.git.generation != generation {
                    return;
                }
                let mut dirty_dirs = HashSet::new();
                if let Some(repo) = &repo {
                    for change in &repo.changes {
                        for ancestor in change.path.ancestors().skip(1) {
                            if !dirty_dirs.insert(repo.root.join(ancestor)) {
                                break;
                            }
                        }
                    }
                }
                let changed = this
                    .git
                    .repo
                    .as_ref()
                    .map(|r| (&r.branch, &r.changes, r.worktrees.len()))
                    != repo
                        .as_ref()
                        .map(|r| (&r.branch, &r.changes, r.worktrees.len()));
                this.git.repo = repo.clone();
                this.git.dirty_dirs = dirty_dirs;
                if changed {
                    this.review
                        .update(cx, |review, cx| review.repository_changed(repo, cx));
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn change_for(&self, path: &Path) -> Option<&git::Change> {
        self.git.repo.as_ref()?.change_for_path(path)
    }

    pub fn change_count(&self) -> usize {
        self.git.repo.as_ref().map_or(0, |r| r.changes.len())
    }

    /// Switch the project to another worktree of the same repository.
    pub fn switch_worktree(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if self.root() != Some(path.as_path()) {
            self.open_project(path, window, cx);
            self.panel = Panel::Changes;
            self.open_review(window, cx);
        }
    }

    // ---- File index and watching -------------------------------------------

    pub fn start_index(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.root().map(Path::to_path_buf) else {
            return;
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let background = cancel.clone();
        self.indexing = Some(cx.spawn(async move |this, cx| {
            let _cancel_on_drop = CancelOnDrop(cancel);
            let index = cx
                .background_executor()
                .spawn(async move { files::index_files(&root, &background) })
                .await;
            this.update(cx, |this, cx| {
                this.index = Some(Arc::new(index));
                this.indexing = None;
                if let Some(palette) = &this.palette {
                    let index = this.index.clone();
                    palette.update(cx, |palette, cx| palette.set_index(index, cx));
                }
            })
            .ok();
        }));
    }

    fn start_watcher(&mut self, cx: &mut Context<Self>) {
        self.watcher = None;
        self.watcher_start = None;
        let Some(root) = self.root().map(Path::to_path_buf) else {
            return;
        };
        if !Settings::get(cx).watch_files {
            return;
        }
        let generation = self.project_generation;
        let (tx, rx) = futures::channel::mpsc::unbounded::<PathBuf>();
        self.watcher_start = Some(cx.spawn(async move |this, cx| {
            let watcher = cx
                .background_executor()
                .spawn(async move {
                    use notify::Watcher as _;
                    let mut watcher = notify::recommended_watcher(
                        move |event: notify::Result<notify::Event>| {
                            if let Ok(event) = event {
                                for path in event.paths {
                                    let _ = tx.unbounded_send(path);
                                }
                            }
                        },
                    )?;
                    watcher.watch(&root, notify::RecursiveMode::Recursive)?;
                    crate::startup::log("file watcher ready".into());
                    Ok::<_, notify::Error>(watcher)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.project_generation != generation || !Settings::get(cx).watch_files {
                    return;
                }
                if let Ok(watcher) = watcher {
                    this.watch_events(watcher, rx, cx);
                }
            });
        }));
    }

    fn watch_events(
        &mut self,
        watcher: notify::RecommendedWatcher,
        mut rx: futures::channel::mpsc::UnboundedReceiver<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        let task = cx.spawn(async move |this, cx| {
            use futures::StreamExt as _;
            // Sleep until something changes, then gather the burst that follows.
            while let Some(first) = rx.next().await {
                cx.background_executor()
                    .timer(Duration::from_millis(250))
                    .await;
                let mut paths = vec![first];
                while let Ok(path) = rx.try_recv() {
                    paths.push(path);
                }
                if this
                    .update(cx, |this, cx| this.files_changed(paths, cx))
                    .is_err()
                {
                    return;
                }
            }
        });
        self.watcher = Some(Watcher {
            _watcher: watcher,
            _task: task,
        });
    }

    fn files_changed(&mut self, mut paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let Some(root) = self.root().map(Path::to_path_buf) else {
            return;
        };
        // A save writes a temporary file and renames it over the target. Those
        // events are ours: nothing was created or removed, and the open tab
        // already holds the new text.
        self.recent_saves
            .retain(|_, saved| saved.elapsed() < SAVE_ECHO);
        if !self.recent_saves.is_empty() {
            let saves = &self.recent_saves;
            let tabs = &self.tabs;
            paths.retain(|path| {
                // Exact for the saved file: still what the tab wrote. An agent
                // editing it a second later changes the stamp and is followed.
                let unchanged = saves.contains_key(path)
                    && tabs.iter().any(
                        |tab| matches!(tab, Tab::File(file) if &file.path == path && file.matches_disk()),
                    );
                !unchanged && !is_save_temporary(path, saves)
            });
            if paths.is_empty() {
                if self.todo_loaded || self.panel == Panel::Todos {
                    self.start_todos(cx);
                }
                return;
            }
        }
        let mut dirs = HashSet::new();
        let mut git_changed = false;
        let mut worktree_changed = false;
        let mut created_or_removed = false;
        for path in &paths {
            let Ok(relative) = path.strip_prefix(&root) else {
                continue;
            };
            let mut parts = relative
                .components()
                .map(|c| c.as_os_str().to_string_lossy());
            let first = parts.next().unwrap_or_default();
            if first == ".git" {
                let second = parts.next().unwrap_or_default();
                if second != "objects" && second != "logs" {
                    git_changed = true;
                }
                continue;
            }
            if files::HEAVY_DIRS
                .iter()
                .any(|heavy| relative.components().any(|c| c.as_os_str() == *heavy))
            {
                continue;
            }
            worktree_changed = true;
            if let Some(parent) = path.parent()
                && self.tree.children.contains_key(parent)
            {
                dirs.insert(parent.to_path_buf());
            }
            if !path.exists()
                || !self.index.as_ref().is_some_and(|i| {
                    i.files
                        .binary_search_by(|p| (**p).cmp(&*files::relative_str(relative)))
                        .is_ok()
                })
            {
                created_or_removed = true;
            }
        }
        if paths.iter().any(|path| {
            path.file_name()
                .is_some_and(|name| name == ".ignore" || name == ".gitignore")
        }) {
            dirs.extend(self.tree.children.keys().cloned());
            created_or_removed = true;
        }
        for dir in dirs {
            self.load_dir(dir, cx);
        }
        if git_changed || worktree_changed {
            self.refresh_git(cx);
        }
        if worktree_changed {
            if self.todo_loaded || self.panel == Panel::Todos {
                self.start_todos(cx);
            }
            self.review_changed(cx);
            let changed: HashSet<PathBuf> = paths.into_iter().collect();
            self.pending_reload = Some(changed);
            cx.notify();
        }
        if created_or_removed {
            self.start_index(cx);
        }
    }

    /// The diff is on screen: as the Review tab, the overview drawn when no
    /// tab is open, or the change list in the sidebar.
    fn review_visible(&self) -> bool {
        self.panel == Panel::Changes
            || matches!(
                self.active.and_then(|ix| self.tabs.get(ix)),
                Some(Tab::Review) | None
            )
    }

    /// The working tree changed. Re-read the diff now if it is showing;
    /// otherwise when it next is, so typing with auto save on does not run
    /// Git after every pause.
    fn review_changed(&mut self, cx: &mut Context<Self>) {
        let visible = self.review_visible();
        self.review.update(cx, |review, cx| {
            if visible {
                review.mark_stale(cx)
            } else {
                review.mark_stale_later()
            }
        });
    }

    pub fn start_todos(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.root().map(Path::to_path_buf) else {
            return;
        };
        self.todo_loading = true;
        self.todo_error = None;
        let cancel = Arc::new(AtomicBool::new(false));
        self.todo_task = Some(cx.spawn(async move |this, cx| {
            let _cancel = CancelOnDrop(cancel.clone());
            cx.background_executor()
                .timer(Duration::from_millis(400))
                .await;
            let scan_root = root.clone();
            let result = cx
                .background_executor()
                .spawn(async move { crate::todos::scan(&scan_root, &cancel) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.root() != Some(root.as_path()) {
                    return;
                }
                this.todo_loading = false;
                this.todo_loaded = true;
                match result {
                    Ok((todos, limited)) => {
                        this.todos = todos;
                        this.todo_limited = limited;
                    }
                    Err(error) => this.todo_error = Some(error.into()),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub fn toggle_todo(&mut self, key: &str, cx: &mut Context<Self>) {
        if !self.completed_todos.remove(key) {
            self.completed_todos.insert(key.to_owned());
        }
        self.remember_session(cx);
        cx.notify();
    }

    pub fn project_color(&self, cx: &App) -> Option<gpui_kit::Hsla> {
        if !self.window_color.unwrap_or(Settings::get(cx).window_colors) {
            return None;
        }
        let root = self.root()?;
        let hash = blake3::hash(root.to_string_lossy().as_bytes());
        Some(gpui_kit::hsla(
            f32::from(hash.as_bytes()[0]) / 256.,
            0.5,
            if cx.theme().is_dark() { 0.6 } else { 0.4 },
            1.,
        ))
    }

    // ---- Tabs --------------------------------------------------------------

    pub fn tab_for_path(&self, path: &Path) -> Option<usize> {
        self.tabs.iter().position(|tab| match tab {
            Tab::File(file) => file.path == path,
            Tab::Pending(pending) => pending.path == path,
            Tab::Review => false,
        })
    }

    pub fn active_file(&self) -> Option<&FileTab> {
        match self.tabs.get(self.active?)? {
            Tab::File(file) => Some(file),
            Tab::Pending(_) | Tab::Review => None,
        }
    }

    pub fn activate(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix >= self.tabs.len() {
            return;
        }
        if let Some(previous) = self.active
            && previous != ix
        {
            self.flush_auto_save(previous, window, cx);
        }
        self.activation_generation += 1;
        self.active = Some(ix);
        self.tab_scroll.scroll_to_item(ix);
        self.unload_idle_tabs(cx);
        // The scroll lands during layout; draw once more so what depends on
        // it (the tab list button) reflects the new position.
        cx.on_next_frame(window, |_, _, cx| cx.notify());
        let shown = self.activation_generation;
        match &mut self.tabs[ix] {
            Tab::File(file) => {
                let path = file.path.clone();
                file.last_shown = shown;
                file.focus(window, cx);
                self.tree.selected = Some(path);
            }
            Tab::Pending(pending) => {
                self.tree.selected = Some(pending.path.clone());
                self.load_pending(ix, window, cx);
            }
            Tab::Review => {
                self.review.update(cx, |review, cx| {
                    review.ensure_loaded(cx);
                    review.focus(window, cx)
                });
            }
        }
        cx.notify();
    }

    pub fn open_file(
        &mut self,
        path: PathBuf,
        line: Option<u32>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let path = dunce::canonicalize(&path).unwrap_or(path);
        if let Some(ix) = self.tab_for_path(&path) {
            if let (Some(line), Some(Tab::Pending(pending))) = (line, self.tabs.get_mut(ix)) {
                pending.line = Some(line);
            }
            self.activate(ix, window, cx);
            if let (Some(line), Some(Tab::File(file))) = (line, self.tabs.get_mut(ix)) {
                file.go_to_line(line, window, cx);
            }
            return;
        }
        let weak = self.weak.clone();
        let generation = self.project_generation;
        cx.spawn_in(window, async move |_, cx| {
            let loaded = {
                let path = path.clone();
                cx.background_executor()
                    .spawn(async move { crate::document::load(&path) })
                    .await
            };
            let _ = cx.update(|window, cx| {
                let Some(this) = weak.upgrade() else { return };
                this.update(cx, |this, cx| {
                    if this.project_generation != generation {
                        return;
                    }
                    match loaded {
                        Ok(file) => {
                            if let Some(ix) = this.tab_for_path(&file.path) {
                                this.activate(ix, window, cx);
                                return;
                            }
                            let mut tab = FileTab::new(file, window, cx);
                            if let Some(line) = line {
                                tab.go_to_line(line, window, cx);
                            }
                            let ix = this
                                .active
                                .map_or(this.tabs.len(), |a| (a + 1).min(this.tabs.len()));
                            this.tabs.insert(ix, Tab::File(tab));
                            this.activate(ix, window, cx);
                            this.remember_session(cx);
                        }
                        Err(error) => this.notify_error(error, window, cx),
                    }
                });
            });
        })
        .detach();
    }

    /// Follow a link from a rendered document to a file or folder.
    pub fn open_linked(
        &mut self,
        path: PathBuf,
        line: Option<u32>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let path = dunce::canonicalize(&path).unwrap_or(path);
        if path.is_file() {
            self.open_file(path, line, window, cx);
        } else if path.is_dir() {
            self.sidebar_open = true;
            self.panel = Panel::Files;
            if !self.tree.expanded.contains(&path) {
                self.toggle_dir(path.clone(), cx);
            }
            self.reveal(&path, cx);
        } else {
            self.notify_error(format!("{} does not exist.", path.display()), window, cx);
        }
    }

    /// Put back the session's tabs without reading their files: each one is a
    /// path until it is first shown, so launch reads only the tab left open.
    fn restore_files(
        &mut self,
        paths: Vec<PathBuf>,
        active: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Until the tabs exist, a session saved by another event keeps them.
        self.restoring_paths = paths.clone();
        self.restoring_active = active.clone();
        let generation = self.project_generation;
        self.restore_task = Some(cx.spawn_in(window, async move |this, cx| {
            // Files deleted since the session was saved do not come back.
            let existing = cx
                .background_executor()
                .spawn(async move {
                    paths
                        .into_iter()
                        .filter(|path| path.is_file())
                        .collect::<Vec<_>>()
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.project_generation != generation {
                    return;
                }
                this.restoring_paths.clear();
                this.restoring_active = None;
                for path in existing {
                    if this.tab_for_path(&path).is_none() {
                        this.tabs.push(Tab::Pending(PendingTab {
                            path,
                            line: None,
                            view: None,
                            loading: false,
                        }));
                    }
                }
                // Return to the tab the user left on, unless they have
                // already opened something in the meantime.
                if this.active.is_none() {
                    let ix = active
                        .and_then(|path| this.tab_for_path(&path))
                        .or((!this.tabs.is_empty()).then_some(0));
                    if let Some(ix) = ix {
                        this.activate(ix, window, cx);
                    }
                }
                this.remember_session(cx);
                cx.notify();
            });
        }));
    }

    /// Keep at most `LOADED_TABS` editors mounted. The tabs stay in the bar;
    /// the ones unused longest give up their editor and keep their place,
    /// cursor, and preview state. Tabs with unsaved work are never unloaded.
    fn unload_idle_tabs(&mut self, cx: &mut Context<Self>) {
        let loaded = self
            .tabs
            .iter()
            .filter(|tab| matches!(tab, Tab::File(_)))
            .count();
        if loaded <= LOADED_TABS {
            return;
        }
        let mut idle: Vec<(u64, usize)> = self
            .tabs
            .iter()
            .enumerate()
            .filter_map(|(ix, tab)| match tab {
                Tab::File(file) if Some(ix) != self.active && file.is_idle() => {
                    Some((file.last_shown, ix))
                }
                _ => None,
            })
            .collect();
        idle.sort_unstable();
        for (_, ix) in idle.into_iter().take(loaded - LOADED_TABS) {
            let Tab::File(file) = &self.tabs[ix] else {
                continue;
            };
            let pending = PendingTab {
                path: file.path.clone(),
                line: None,
                view: Some((file.cursor(cx), file.preview)),
                loading: false,
            };
            self.tabs[ix] = Tab::Pending(pending);
        }
    }

    /// Read a restored tab's file and mount its editor, the first time it is shown.
    fn load_pending(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Tab::Pending(pending)) = self.tabs.get_mut(ix) else {
            return;
        };
        if pending.loading {
            return;
        }
        pending.loading = true;
        let path = pending.path.clone();
        let generation = self.project_generation;
        cx.spawn_in(window, async move |this, cx| {
            let loaded = {
                let path = path.clone();
                cx.background_executor()
                    .spawn(async move { crate::document::load(&path) })
                    .await
            };
            let _ =
                this.update_in(cx, |this, window, cx| {
                    if this.project_generation != generation {
                        return;
                    }
                    let Some(ix) = this.tabs.iter().position(
                        |tab| matches!(tab, Tab::Pending(pending) if pending.path == path),
                    ) else {
                        return;
                    };
                    let (line, view) = match &this.tabs[ix] {
                        Tab::Pending(pending) => (pending.line, pending.view),
                        _ => (None, None),
                    };
                    match loaded {
                        // The same file is already open under its canonical path.
                        Ok(file)
                            if this.tabs.iter().any(
                                |tab| matches!(tab, Tab::File(open) if open.path == file.path),
                            ) =>
                        {
                            this.remove_tab(ix, window, cx)
                        }
                        Ok(file) => {
                            let mut tab = FileTab::new(file, window, cx);
                            tab.last_shown = this.activation_generation;
                            if let Some((cursor, preview)) = view {
                                tab.restore_view(cursor, preview, window, cx);
                            }
                            if let Some(line) = line {
                                tab.go_to_line(line, window, cx);
                            }
                            // Take the keyboard only if nothing else (the palette,
                            // the terminal) took it while the file was read.
                            if this.active == Some(ix) && this.focus.is_focused(window) {
                                tab.focus(window, cx);
                            }
                            this.tabs[ix] = Tab::File(tab);
                            this.unload_idle_tabs(cx);
                        }
                        // Deleted, or no longer readable as text.
                        Err(_) => this.remove_tab(ix, window, cx),
                    }
                    cx.notify();
                });
        })
        .detach();
    }

    pub fn close_tabs(
        &mut self,
        mode: TabClose,
        anchor: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.closing_tabs {
            return;
        }
        if matches!(mode, TabClose::All) {
            self.restore_task = None;
            self.restoring_paths.clear();
            self.restoring_active = None;
        }
        let targets: Vec<_> = self
            .tabs
            .iter()
            .enumerate()
            .filter(|(ix, _)| mode.includes(*ix, anchor))
            .map(|(_, tab)| tab.target())
            .collect();
        self.closing_tabs = true;
        cx.spawn_in(window, async move |this, cx| {
            for target in targets {
                let prompt = this.update_in(cx, |this, window, cx| {
                    let ix = this.tabs.iter().position(|tab| tab.target() == target)?;
                    if let Tab::File(file) = &this.tabs[ix]
                        && file.dirty
                    {
                        return Some(window.prompt(
                            PromptLevel::Warning,
                            &format!("Save changes to {}?", file.name()),
                            Some("Your changes will be lost if you don't save them."),
                            &["Save", "Don't Save", "Cancel"],
                            cx,
                        ));
                    }
                    this.remove_tab(ix, window, cx);
                    None
                });
                let Ok(prompt) = prompt else {
                    break;
                };
                let Some(prompt) = prompt else {
                    continue;
                };
                match prompt.await {
                    Ok(0) => {
                        let job = this
                            .update(cx, |this, cx| {
                                let ix = this.tabs.iter().position(|tab| tab.target() == target)?;
                                match &mut this.tabs[ix] {
                                    Tab::File(file) => file.begin_save(cx),
                                    _ => None,
                                }
                            })
                            .ok()
                            .flatten();
                        // A save already in flight must finish before this tab can close.
                        let Some(job) = job else {
                            break;
                        };
                        let result = cx
                            .background_executor()
                            .spawn(async move { job.run() })
                            .await;
                        let closed = this
                            .update_in(cx, |this, window, cx| {
                                let Some(ix) =
                                    this.tabs.iter().position(|tab| tab.target() == target)
                                else {
                                    return true;
                                };
                                if let Tab::File(file) = &mut this.tabs[ix] {
                                    if let Err(error) = file.finish_save(result, cx) {
                                        this.notify_error(error, window, cx);
                                        return false;
                                    }
                                    if file.dirty {
                                        return false;
                                    }
                                }
                                this.remove_tab(ix, window, cx);
                                this.refresh_git(cx);
                                true
                            })
                            .unwrap_or(false);
                        if !closed {
                            break;
                        }
                    }
                    Ok(1) => {
                        let _ = this.update_in(cx, |this, window, cx| {
                            if let Some(ix) =
                                this.tabs.iter().position(|tab| tab.target() == target)
                            {
                                this.remove_tab(ix, window, cx);
                            }
                        });
                    }
                    _ => break,
                }
            }
            let _ = this.update(cx, |this, cx| {
                this.closing_tabs = false;
                cx.notify();
            });
        })
        .detach();
    }

    pub fn close_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(ix) else { return };
        if let Tab::File(file) = tab
            && file.dirty
        {
            let name = file.name();
            self.save_prompts += 1;
            let answer = window.prompt(
                PromptLevel::Warning,
                &format!("Save changes to {name}?"),
                Some("Your changes will be lost if you don't save them."),
                &["Save", "Don't Save", "Cancel"],
                cx,
            );
            let path = file.path.clone();
            cx.spawn_in(window, async move |this, cx| {
                let answer = answer.await.ok();
                let _ = this.update_in(cx, |this, window, cx| {
                    this.save_prompts = this.save_prompts.saturating_sub(1);
                    let Some(ix) = this.tab_for_path(&path) else {
                        return;
                    };
                    match answer {
                        Some(0) => this.save_tab(ix, true, window, cx),
                        Some(1) => this.remove_tab(ix, window, cx),
                        _ => {}
                    }
                });
            })
            .detach();
            return;
        }
        self.remove_tab(ix, window, cx);
    }

    fn remove_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix >= self.tabs.len() {
            return;
        }
        self.tabs.remove(ix);
        self.active = match self.active {
            _ if self.tabs.is_empty() => None,
            Some(active) if active > ix => Some(active - 1),
            Some(active) if active == ix => Some(ix.min(self.tabs.len() - 1)),
            other => other,
        };
        if let Some(active) = self.active {
            self.activate(active, window, cx);
        } else {
            self.focus.focus(window, cx);
        }
        self.remember_session(cx);
        cx.notify();
    }

    pub fn start_new_file(&mut self, parent: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if self.new_file.as_ref().is_some_and(|draft| draft.creating) {
            return;
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("File name"));
        let subscription = cx.subscribe_in(&input, window, |this, _, event, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.create_file(window, cx);
            }
        });
        input.update(cx, |input, cx| input.focus(window, cx));
        self.new_file = Some(NewFile {
            parent,
            input,
            error: None,
            creating: false,
            _subscription: subscription,
        });
        self.panel = Panel::Files;
        self.sidebar_open = true;
        cx.notify();
    }

    pub fn create_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(draft) = self.new_file.as_mut() else {
            return;
        };
        if draft.creating {
            return;
        }
        draft.creating = true;
        let parent = draft.parent.clone();
        let input = draft.input.clone();
        let name = input.read(cx).value().to_string();
        let generation = self.project_generation;
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { crate::document::create(&parent, &name) })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.project_generation != generation {
                    return;
                }
                if this
                    .new_file
                    .as_ref()
                    .is_none_or(|draft| draft.input != input)
                {
                    return;
                }
                match result {
                    Ok(path) => {
                        this.new_file = None;
                        if let Some(parent) = path.parent() {
                            this.load_dir(parent.to_path_buf(), cx);
                        }
                        this.reveal(&path, cx);
                        this.start_index(cx);
                        this.refresh_git(cx);
                        this.open_file(path, None, window, cx);
                    }
                    Err(error) => {
                        if let Some(draft) = &mut this.new_file {
                            draft.error = Some(error.into());
                            draft.creating = false;
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Save a tab now instead of waiting out its timer, when focus moves away.
    fn flush_auto_save(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if !Settings::get(cx).auto_save || self.closing_tabs || self.save_prompts > 0 {
            return;
        }
        if let Some(Tab::File(file)) = self.tabs.get_mut(ix)
            && file.auto_save_ready()
        {
            file.auto_save_task = None;
            self.save_tab_impl(ix, false, true, window, cx);
        }
    }

    /// Each edit replaces its tab's timer; idle workspaces do no autosave polling.
    pub fn schedule_auto_save(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Tab::File(file)) = self.tabs.get_mut(ix) else {
            return;
        };
        file.auto_save_task = None;
        if !Settings::get(cx).auto_save || !file.auto_save_ready() {
            return;
        }
        let editor = file.editor.clone();
        file.auto_save_task = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(AUTO_SAVE_DELAY).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if !Settings::get(cx).auto_save { return; }
                if let Some(ix) = this.tabs.iter().position(|tab| matches!(tab, Tab::File(file) if file.editor == editor && file.auto_save_ready())) {
                    if this.closing_tabs || this.save_prompts > 0 {
                        this.schedule_auto_save(ix, window, cx);
                    } else {
                        this.save_tab_impl(ix, false, true, window, cx);
                    }
                }
            });
        }));
    }

    pub fn format_document(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.active else {
            return;
        };
        let Some(Tab::File(file)) = self.tabs.get_mut(ix) else {
            return;
        };
        let Some(kind) = crate::structured::Kind::for_path(&file.path) else {
            return;
        };
        if file.formatting {
            return;
        }
        file.formatting = true;
        let source = file.editor.read(cx).value();
        let editor = file.editor.clone();
        let version = file.version;
        let indent = Settings::get(cx).tab_size as usize;
        cx.spawn_in(window, async move |this, cx| {
            let text = source.clone();
            let formatted = cx
                .background_executor()
                .spawn(async move {
                    crate::structured::format(kind, &text, indent, &AtomicBool::new(false))
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                let Some(ix) = this
                    .tabs
                    .iter()
                    .position(|tab| matches!(tab, Tab::File(file) if file.editor == editor))
                else {
                    return;
                };
                let Tab::File(file) = &mut this.tabs[ix] else {
                    return;
                };
                file.formatting = false;
                if file.version != version || editor.read(cx).value() != source {
                    this.status = Some("File changed while formatting. Run Format again.".into());
                    cx.notify();
                    return;
                }
                match formatted {
                    Ok(text) if text != source.as_ref() => {
                        editor.update(cx, |state, cx| state.replace_all(text, window, cx))
                    }
                    Ok(_) => {}
                    Err(error) => this.notify_error(error, window, cx),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub fn save_tab(
        &mut self,
        ix: usize,
        close_after: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.save_tab_impl(ix, close_after, false, window, cx);
    }

    fn save_tab_impl(
        &mut self,
        ix: usize,
        close_after: bool,
        automatic: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(Tab::File(file)) = self.tabs.get_mut(ix) else {
            return;
        };
        let Some(job) = file.begin_save(cx) else {
            return;
        };
        let path = file.path.clone();
        let editor = file.editor.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { job.run() })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                let Some(ix) = this.tab_for_path(&path) else {
                    return;
                };
                let Some(Tab::File(file)) = this.tabs.get_mut(ix) else {
                    return;
                };
                if file.editor != editor {
                    return;
                }
                match file.finish_save(result, cx) {
                    Ok(()) => {
                        let (name, dirty) = (file.name(), file.dirty);
                        this.recent_saves.insert(path.clone(), Instant::now());
                        this.review_changed(cx);
                        if !automatic {
                            this.status = Some(format!("Saved {name}").into());
                        }
                        if close_after && !dirty {
                            this.remove_tab(ix, window, cx);
                        } else {
                            this.schedule_auto_save(ix, window, cx);
                        }
                        this.refresh_git(cx);
                    }
                    Err(error) => {
                        this.notify_error(error, window, cx);
                        this.schedule_auto_save(ix, window, cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub fn has_unsaved(&self, _cx: &App) -> bool {
        self.tabs
            .iter()
            .any(|t| matches!(t, Tab::File(f) if f.dirty))
    }

    fn discard_all(&mut self, cx: &mut Context<Self>) {
        for tab in &mut self.tabs {
            if let Tab::File(file) = tab {
                file.dirty = false;
            }
        }
        self.remember_session(cx);
    }

    /// Ask before losing edits. Resolves to true when it is safe to continue.
    pub fn confirm_discard(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Task<bool> {
        let dirty: Vec<String> = self
            .tabs
            .iter()
            .filter_map(|t| match t {
                Tab::File(f) if f.dirty => Some(f.name().to_string()),
                _ => None,
            })
            .collect();
        if dirty.is_empty() {
            return Task::ready(true);
        }
        let message = if dirty.len() == 1 {
            format!("Save changes to {}?", dirty[0])
        } else {
            format!("Save changes to {} files?", dirty.len())
        };
        self.save_prompts += 1;
        let answer = window.prompt(
            PromptLevel::Warning,
            &message,
            Some("Your changes will be lost if you don't save them."),
            &["Save All", "Don't Save", "Cancel"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            let answer = answer.await;
            this.update_in(cx, |this, window, cx| {
                this.save_prompts = this.save_prompts.saturating_sub(1);
                match answer {
                    Ok(0) => this.save_all_blocking(window, cx),
                    Ok(1) => {
                        this.discard_all(cx);
                        true
                    }
                    _ => false,
                }
            })
            .unwrap_or(false)
        })
    }

    /// The process is ending without having asked: write what auto save
    /// would have written, and remember the session.
    fn save_before_exit(&mut self, cx: &mut Context<Self>) {
        if Settings::get(cx).auto_save {
            for tab in &mut self.tabs {
                if let Tab::File(file) = tab
                    && file.auto_save_ready()
                    && let Some(job) = file.begin_save(cx)
                {
                    let _ = file.finish_save(job.run(), cx);
                }
            }
        }
        self.remember_session(cx);
    }

    /// Saves synchronously; used only when the window is about to go away.
    fn save_all_blocking(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let mut ok = true;
        for ix in 0..self.tabs.len() {
            let Some(Tab::File(file)) = self.tabs.get_mut(ix) else {
                continue;
            };
            if !file.dirty {
                continue;
            }
            let Some(job) = file.begin_save(cx) else {
                self.notify_error(
                    "A save is still finishing. Please try closing again.",
                    window,
                    cx,
                );
                ok = false;
                break;
            };
            let result = job.run();
            if let Err(error) = file.finish_save(result, cx) {
                ok = false;
                self.notify_error(error, window, cx);
                break;
            }
        }
        ok
    }

    /// Pick up files changed outside Modo. The stat, hash, and read happen off
    /// the UI thread; a tab that moved on in the meantime is left alone.
    pub fn reload_clean_files(
        &mut self,
        only: Option<&HashSet<PathBuf>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let probes: Vec<_> = self
            .tabs
            .iter()
            .filter_map(|tab| match tab {
                Tab::File(file) if only.is_none_or(|set| set.contains(&file.path)) => {
                    file.disk_probe()
                }
                _ => None,
            })
            .collect();
        if probes.is_empty() {
            return;
        }
        cx.spawn_in(window, async move |this, cx| {
            let found = cx
                .background_executor()
                .spawn(async move {
                    probes
                        .into_iter()
                        .filter_map(|probe| {
                            let state = probe.check()?;
                            Some((probe.path.clone(), probe.fingerprint(), state))
                        })
                        .collect::<Vec<_>>()
                })
                .await;
            if found.is_empty() {
                return;
            }
            let _ = this.update_in(cx, |this, window, cx| {
                for (path, probed, state) in found {
                    if let Some(ix) = this.tab_for_path(&path)
                        && let Some(Tab::File(file)) = this.tabs.get_mut(ix)
                    {
                        file.apply_disk_state(probed, state, window, cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Close the window, asking about unsaved files first.
    pub fn close_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let proceed = self.confirm_discard(window, cx);
        cx.spawn_in(window, async move |this, cx| {
            if proceed.await {
                let _ = this.update_in(cx, |this, window, cx| {
                    this.discard_all(cx);
                    window.remove_window();
                });
            }
        })
        .detach();
    }

    // ---- Panels and overlays -----------------------------------------------

    pub fn open_review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ix = match self.tabs.iter().position(|t| matches!(t, Tab::Review)) {
            Some(ix) => ix,
            None => {
                self.tabs.insert(0, Tab::Review);
                if let Some(active) = self.active.as_mut() {
                    *active += 1;
                }
                0
            }
        };
        self.review
            .update(cx, |review, cx| review.ensure_loaded(cx));
        self.activate(ix, window, cx);
    }

    pub fn open_palette(&mut self, initial: &str, window: &mut Window, cx: &mut Context<Self>) {
        let index = self.index.clone();
        let root = self.root().map(Path::to_path_buf);
        let recent = Session::get(cx).recent.clone();
        let workspace = self.weak.clone();
        let palette =
            cx.new(|cx| Palette::new(initial, root, index, recent, workspace, window, cx));
        self._subscriptions.push(cx.subscribe_in(
            &palette,
            window,
            |this, _, event, window, cx| this.on_palette_event(event, window, cx),
        ));
        if self.index.is_none() && self.indexing.is_none() {
            self.start_index(cx);
        }
        self.palette = Some(palette);
        cx.notify();
    }

    fn on_palette_event(
        &mut self,
        event: &super::palette::PaletteEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use super::palette::PaletteEvent;
        self.palette = None;
        match event {
            PaletteEvent::Dismiss => self.restore_focus(window, cx),
            PaletteEvent::OpenFile(path) => self.open_file(path.clone(), None, window, cx),
            PaletteEvent::OpenProject(path) => self.open_path(path.clone(), window, cx),
            PaletteEvent::GoToLine(line) => {
                if let Some(ix) = self.active
                    && let Some(Tab::File(file)) = self.tabs.get_mut(ix)
                {
                    file.go_to_line(*line, window, cx);
                }
            }
            PaletteEvent::Run(action) => {
                self.restore_focus(window, cx);
                window.dispatch_action(action.boxed_clone(), cx);
            }
        }
        cx.notify();
    }

    pub fn restore_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.active.and_then(|ix| self.tabs.get(ix)) {
            Some(Tab::File(file)) => file.focus(window, cx),
            Some(Tab::Review) => self.review.update(cx, |r, cx| r.focus(window, cx)),
            Some(Tab::Pending(_)) | None => self.focus.focus(window, cx),
        }
    }

    pub fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let gradient_enabled = if self.project.is_some() {
            self.project_color(cx).is_some()
        } else {
            Settings::get(cx).window_colors
        };
        let view = cx.new(|cx| SettingsView::new(gradient_enabled, window, cx));
        self._subscriptions.push(cx.subscribe_in(
            &view,
            window,
            |this, _, event: &super::settings_view::SettingsEvent, window, cx| {
                match event {
                    super::settings_view::SettingsEvent::Close => {
                        this.settings = None;
                        this.restore_focus(window, cx);
                    }
                    super::settings_view::SettingsEvent::ProjectGradient(enabled) => {
                        if this.project.is_some() {
                            this.window_color = Some(*enabled);
                            this.remember_session(cx);
                        }
                    }
                }
                cx.notify();
            },
        ));
        view.update(cx, |view, cx| view.focus(window, cx));
        self.settings = Some(view);
        cx.notify();
    }

    pub fn prompt_open_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Open".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = paths.await
                && let Some(path) = paths.into_iter().next()
            {
                let _ = this.update_in(cx, |this, window, cx| {
                    if this.project.is_some() && this.has_unsaved(cx) {
                        // Keep unsaved work: open the folder beside this one.
                        crate::open_window(Some(path), cx);
                    } else {
                        this.open_path(path, window, cx);
                    }
                });
            }
        })
        .detach();
    }

    fn on_review_event(
        &mut self,
        _: &Entity<Review>,
        event: &super::review::ReviewEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            super::review::ReviewEvent::OpenFile(path, line) => {
                self.open_file(path.clone(), *line, window, cx)
            }
            super::review::ReviewEvent::BaseChanged
            | super::review::ReviewEvent::ProgressChanged => self.remember_session(cx),
        }
    }

    fn on_search_event(
        &mut self,
        _: &Entity<super::sidebar::SearchPanel>,
        event: &super::sidebar::SearchEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            super::sidebar::SearchEvent::Open(path, line) => {
                self.open_file(path.clone(), Some(*line), window, cx)
            }
        }
    }

    pub fn notify_error(
        &mut self,
        message: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let note = Notification::error(message.into());
        window.push_notification(note, cx);
    }

    // ---- Rendering ---------------------------------------------------------

    fn render_tab_bar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let left_pad = if (!self.sidebar_open
            || (self.terminal.read(cx).open && self.terminal.read(cx).maximized))
            && is_mac()
            && !window.is_fullscreen()
        {
            r(78.)
        } else {
            r(8.)
        };
        // Measured on the previous frame; `activate` asks for one more frame so
        // the list button appears as soon as the tabs stop fitting.
        let overflowing = self.tab_scroll.max_offset().x > gpui_kit::px(0.);
        let tab_list = overflowing.then(|| self.render_tab_list(cx));
        let tabs = self.tabs.iter().enumerate().map(|(ix, tab)| {
            let active = self.active == Some(ix);
            let (label, icon, dirty, tooltip): (
                SharedString,
                AnyElement,
                bool,
                Option<SharedString>,
            ) = match tab {
                Tab::File(file) => (
                    file.name(),
                    super::sidebar::file_icon(&file.path, cx).into_any_element(),
                    file.dirty,
                    self.root()
                        .map(|root| files::display_relative(root, &file.path).into()),
                ),
                Tab::Pending(pending) => (
                    file_name(&pending.path),
                    super::sidebar::file_icon(&pending.path, cx).into_any_element(),
                    false,
                    self.root()
                        .map(|root| files::display_relative(root, &pending.path).into()),
                ),
                Tab::Review => (
                    "Review".into(),
                    gpui_kit::component::Icon::new(gpui_kit::assets::IconName::GitCompare)
                        .size(r(14.))
                        .text_color(theme.muted_foreground)
                        .into_any_element(),
                    false,
                    None,
                ),
            };
            let hover = theme.list_hover;
            let workspace = cx.entity().downgrade();
            let target = tab.target();
            h_flex()
                .id(("tab", ix))
                .group("tab")
                .flex_none()
                .h(r(26.))
                .min_w(r(72.))
                .max_w(r(190.))
                .pl(r(9.))
                .pr(r(4.))
                .gap(r(6.))
                .rounded(r(6.))
                .text_size(r(12.5))
                .cursor_pointer()
                .when(active, |t| {
                    t.bg(theme.list_active).text_color(theme.foreground)
                })
                .when(!active, |t| {
                    t.text_color(theme.muted_foreground)
                        .hover(move |s| s.bg(hover))
                })
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_mouse_down(
                    MouseButton::Middle,
                    cx.listener(move |this, _, window, cx| this.close_tab(ix, window, cx)),
                )
                .on_click(cx.listener(move |this, _, window, cx| this.activate(ix, window, cx)))
                .when_some(tooltip, |t, tip| {
                    t.tooltip(move |window, cx| {
                        gpui_kit::component::tooltip::Tooltip::new(tip.clone()).build(window, cx)
                    })
                })
                .child(div().flex_none().child(icon))
                .child(div().flex_1().min_w_0().truncate().child(label))
                .child(
                    div()
                        .id(("close", ix))
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .size(r(18.))
                        .rounded(r(4.))
                        .hover(move |s| s.bg(hover))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.close_tab(ix, window, cx)
                        }))
                        .map(|close| {
                            if dirty {
                                close.child(
                                    div().size(r(7.)).rounded_full().bg(theme.muted_foreground),
                                )
                            } else {
                                close
                                    .invisible()
                                    .group_hover("tab", |s| s.visible())
                                    .when(active, |s| s.visible())
                                    .child(
                                        gpui_kit::component::Icon::new(IconName::Close)
                                            .size(r(12.))
                                            .text_color(theme.muted_foreground),
                                    )
                            }
                        }),
                )
                .context_menu(move |menu, _, _| {
                    let mut menu = menu;
                    for (label, mode) in [
                        ("Close all tabs", TabClose::All),
                        ("Close all but this", TabClose::Others),
                        ("Close tabs to the left", TabClose::Left),
                        ("Close tabs to the right", TabClose::Right),
                    ] {
                        let workspace = workspace.clone();
                        let target = target.clone();
                        menu =
                            menu.item(PopupMenuItem::new(label).on_click(move |_, window, cx| {
                                let _ = workspace.update(cx, |this, cx| {
                                    let anchor =
                                        this.tabs.iter().position(|tab| tab.target() == target);
                                    if anchor.is_some() {
                                        this.close_tabs(mode, anchor, window, cx);
                                    }
                                });
                            }));
                    }
                    menu
                })
        });
        let terminal_open = self.terminal.read(cx).open;
        TitleBar::new()
            .h(r(38.))
            .pl(left_pad)
            .pr(r(8.))
            .bg(theme.background)
            .border_color(gpui_kit::transparent_black())
            .child(
                // An absolute zero basis keeps a long row of tabs from widening
                // the bar (a percentage basis falls back to the content width
                // when the bar is measured): the row takes the space left over
                // and scrolls inside it.
                h_flex()
                    .w_0()
                    .flex_grow(1.)
                    .flex_shrink(1.)
                    .flex_basis(gpui_kit::px(0.))
                    .min_w_0()
                    .h_full()
                    .gap(r(2.))
                    .overflow_hidden()
                    .when(!self.sidebar_open, |bar| {
                        bar.child(
                            icon_button("open-sidebar", gpui_kit::assets::IconName::PanelLeft, cx)
                                .mr(r(4.))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.sidebar_open = true;
                                    cx.notify();
                                })),
                        )
                    })
                    .child(
                        h_flex()
                            .id("tabs")
                            .min_w_0()
                            .h_full()
                            .gap(r(2.))
                            .overflow_x_scroll()
                            .track_scroll(&self.tab_scroll)
                            .children(tabs),
                    ),
            )
            .child(
                h_flex()
                    .flex_none()
                    .pl(r(6.))
                    .gap(r(2.))
                    .children(tab_list)
                    .child(
                        icon_button("toggle-terminal", gpui_kit::assets::IconName::Terminal, cx)
                            .when(terminal_open, |b| b.bg(cx.theme().list_active))
                            .tooltip(|window, cx| {
                                gpui_kit::component::tooltip::Tooltip::new("Terminal")
                                    .key_binding(gpui_kit::component::kbd::Kbd::binding_for_action(
                                        &ToggleTerminal,
                                        None,
                                        window,
                                    ))
                                    .build(window, cx)
                            })
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.terminal.update(cx, |t, cx| t.toggle(window, cx))
                            })),
                    ),
            )
            .into_any_element()
    }

    /// Every open tab in a menu, for when the bar holds more than it can show.
    fn render_tab_list(&self, cx: &mut Context<Self>) -> AnyElement {
        use gpui_kit::component::{
            Sizable as _, button::ButtonVariants as _, menu::DropdownMenu as _,
        };
        let workspace = cx.entity().downgrade();
        gpui_kit::component::button::Button::new("tab-list")
            .ghost()
            .small()
            .icon(
                gpui_kit::component::Icon::new(IconName::ChevronDown)
                    .size(r(14.))
                    .text_color(cx.theme().muted_foreground),
            )
            .tooltip("Open tabs")
            .dropdown_menu_with_anchor(gpui_kit::Anchor::TopRight, move |menu, window, cx| {
                let height =
                    gpui_kit::px(420.).min(window.viewport_size().height - gpui_kit::px(96.));
                let mut menu = menu
                    .min_w(gpui_kit::px(240.))
                    .max_w(gpui_kit::px(360.))
                    .max_h(height)
                    .scrollable(true);
                let Some(this) = workspace.upgrade() else {
                    return menu;
                };
                let this = this.read(cx);
                for (ix, tab) in this.tabs.iter().enumerate() {
                    let label: SharedString = match tab {
                        Tab::File(FileTab { path, .. }) | Tab::Pending(PendingTab { path, .. }) => {
                            match this.root() {
                                Some(root) => files::display_relative(root, path).into(),
                                None => file_name(path),
                            }
                        }
                        Tab::Review => "Review".into(),
                    };
                    let target = tab.target();
                    let workspace = workspace.clone();
                    menu = menu.item(
                        PopupMenuItem::new(label)
                            .checked(this.active == Some(ix))
                            .on_click(move |_, window, cx| {
                                let _ = workspace.update(cx, |this, cx| {
                                    if let Some(ix) =
                                        this.tabs.iter().position(|tab| tab.target() == target)
                                    {
                                        this.activate(ix, window, cx);
                                    }
                                });
                            }),
                    );
                }
                menu
            })
            .into_any_element()
    }

    fn render_content(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if self.project.is_none() && self.tabs.is_empty() {
            return super::welcome::render(self, window, cx).into_any_element();
        }
        let root = self.root().map(Path::to_path_buf);
        match self.active.and_then(|ix| self.tabs.get_mut(ix)) {
            Some(Tab::File(file)) => file.render(root.as_deref(), window, cx).into_any_element(),
            Some(Tab::Review) => self.review.clone().into_any_element(),
            // Its file is being read; the editor replaces this within a frame or two.
            Some(Tab::Pending(_)) => div().size_full().into_any_element(),
            None => super::welcome::render_empty_project(self, cx).into_any_element(),
        }
    }

    fn render_status_bar(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let muted = theme.muted_foreground;
        let hover = theme.list_hover;
        let item = move |id: &'static str| {
            h_flex()
                .id(id)
                .h_full()
                .px(r(6.))
                .gap(r(5.))
                .rounded(r(4.))
                .hover(move |s| s.bg(hover))
        };
        let branch = self.git.repo.as_ref().map(|r| r.branch.clone());
        let changes = self.change_count();
        let limited = self.git.repo.as_ref().is_some_and(|r| r.limited);
        h_flex()
            .flex_none()
            .h(r(24.))
            .px(r(8.))
            .gap(r(2.))
            .text_size(r(11.5))
            .text_color(muted)
            .border_t_1()
            .border_color(theme.border)
            .bg(theme.background)
            .when_some(branch, |bar, branch| {
                bar.child(
                    item("status-branch")
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.panel = Panel::Changes;
                            this.sidebar_open = true;
                            this.open_review(window, cx);
                        }))
                        .child(
                            gpui_kit::component::Icon::new(gpui_kit::assets::IconName::GitBranch)
                                .size(r(13.)),
                        )
                        .child(branch)
                        .when(changes > 0, |row| {
                            let plus = if limited { "+" } else { "" };
                            row.child(
                                div()
                                    .text_color(theme.yellow)
                                    .child(format!("{changes}{plus} changed")),
                            )
                        }),
                )
            })
            .child(div().flex_1())
            .when_some(
                cx.try_global::<crate::updater::UpdateState>()
                    .and_then(|state| state.notice().map(|notice| (notice, state.phase.clone()))),
                |bar, (notice, phase)| {
                    use crate::updater::Phase;
                    let actionable = !matches!(phase, Phase::Downloading { .. });
                    bar.child(
                        item("update-status")
                            .max_w(r(420.))
                            .when(actionable, |item| {
                                item.cursor_pointer().text_color(theme.primary).on_click(
                                    move |_, _, cx| match phase {
                                        Phase::Ready { .. } => crate::updater::restart(cx),
                                        _ => crate::updater::install_available(cx),
                                    },
                                )
                            })
                            .child(div().truncate().child(notice)),
                    )
                },
            )
            .when_some(self.status.clone(), |bar, status| {
                bar.child(div().px(r(6.)).child(status))
            })
            .when_some(self.active_file(), |bar, file| {
                let info = file.status_info(cx);
                bar.child(item("status-cursor").child(info.position))
                    .child(item("status-indent").child(info.indent))
                    .child(item("status-encoding").child(info.encoding))
                    .child(item("status-eol").child(info.line_ending))
                    .child(item("status-language").child(info.language))
            })
            .into_any_element()
    }
}

/// The temporary file a recent save wrote beside its target and has since
/// renamed into place.
fn is_save_temporary(path: &Path, saves: &HashMap<PathBuf, Instant>) -> bool {
    path.file_name()
        .is_some_and(|name| name.to_string_lossy().starts_with(".tmp"))
        && saves.keys().any(|saved| saved.parent() == path.parent())
        && !path.exists()
}

/// Cancels a background walk when its owning task is dropped.
struct CancelOnDrop(Arc<AtomicBool>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

fn git_root_or_parent(file: &Path) -> PathBuf {
    let parent = file.parent().unwrap_or(file).to_path_buf();
    for ancestor in parent.ancestors() {
        if ancestor.join(".git").exists() {
            return ancestor.to_path_buf();
        }
    }
    parent
}

impl Focusable for Workspace {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.launching {
            // A frame or two of plain background, rather than a welcome page
            // that the restored project would immediately replace.
            return div()
                .id("workspace")
                .size_full()
                .bg(cx.theme().background)
                .into_any_element();
        }
        // Timed from the first frame with content, not the placeholder above.
        static LOGGED: std::sync::Once = std::sync::Once::new();
        LOGGED.call_once(|| {
            if std::env::var_os("MODO_LOG_STARTUP").is_some()
                && let Some(started) = crate::STARTED.get()
            {
                eprintln!("modo: first frame after {:?}", started.elapsed());
            }
            cx.background_executor()
                .spawn(async {
                    crate::startup::log("first frame".into());
                })
                .detach();
        });
        if let Some(paths) = self.pending_reload.take() {
            self.reload_clean_files(Some(&paths), window, cx);
        }
        let theme = cx.theme().clone();

        let terminal_maximized = self.terminal.read(cx).open && self.terminal.read(cx).maximized;
        let sidebar = (self.sidebar_open && !terminal_maximized)
            .then(|| super::sidebar::render(self, window, cx));
        let tab_bar = self.render_tab_bar(window, cx);
        let content = (!terminal_maximized).then(|| self.render_content(window, cx));
        let status = self.render_status_bar(cx);
        let terminal = self.terminal.clone();
        let terminal_open = terminal.read(cx).open;
        let gradient = self.project_color(cx).map(|color| {
            super::project_gradient::render(color, theme.is_dark()).into_any_element()
        });
        let overlay = self
            .palette
            .clone()
            .map(|p| p.into_any_element())
            .or_else(|| self.settings.clone().map(|s| s.into_any_element()));
        h_flex()
            .id("workspace")
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .size_full()
            .relative()
            .bg(theme.background)
            .text_color(theme.foreground)
            .font_family(theme.font_family.clone())
            .on_action(
                cx.listener(|this, _: &OpenFolder, window, cx| this.prompt_open_folder(window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &QuickOpen, window, cx| this.open_palette("", window, cx)),
            )
            .on_action(cx.listener(|this, _: &CommandPalette, window, cx| {
                this.open_palette(">", window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &GoToLine, window, cx| this.open_palette(":", window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &SwitchProject, window, cx| {
                    this.open_palette("@", window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &FindInFiles, window, cx| {
                this.sidebar_open = true;
                this.panel = Panel::Search;
                let selection = this.active_file().and_then(|f| f.selected_text(cx));
                this.search
                    .update(cx, |s, cx| s.focus(selection, window, cx));
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ShowFiles, _, cx| {
                this.sidebar_open = true;
                this.panel = Panel::Files;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| {
                this.sidebar_open = !this.sidebar_open;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleTerminal, window, cx| {
                this.terminal.update(cx, |t, cx| t.toggle(window, cx));
                if !this.terminal.read(cx).open {
                    this.restore_focus(window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &MaximizeTerminal, window, cx| {
                this.terminal
                    .update(cx, |t, cx| t.toggle_maximized(window, cx));
            }))
            .on_action(
                cx.listener(|this, _: &SplitTerminalHorizontal, window, cx| {
                    this.terminal.update(cx, |t, cx| {
                        t.split(super::terminal::SplitAxis::Horizontal, window, cx)
                    });
                }),
            )
            .on_action(cx.listener(|this, _: &SplitTerminalVertical, window, cx| {
                this.terminal.update(cx, |t, cx| {
                    t.split(super::terminal::SplitAxis::Vertical, window, cx)
                });
            }))
            .on_action(cx.listener(|this, _: &ShowTodos, _, cx| {
                this.sidebar_open = true;
                this.panel = Panel::Todos;
                this.start_todos(cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleWindowColor, _, cx| {
                this.window_color =
                    Some(!this.window_color.unwrap_or(Settings::get(cx).window_colors));
                if let Some(settings) = &this.settings {
                    let enabled = this.window_color.unwrap_or(false);
                    settings.update(cx, |settings, cx| {
                        settings.set_gradient_enabled(enabled, cx)
                    });
                }
                this.remember_session(cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &CloseAllTabs, window, cx| {
                this.close_tabs(TabClose::All, this.active, window, cx)
            }))
            .on_action(cx.listener(|this, _: &CloseOtherTabs, window, cx| {
                this.close_tabs(TabClose::Others, this.active, window, cx)
            }))
            .on_action(cx.listener(|this, _: &CloseTabsLeft, window, cx| {
                this.close_tabs(TabClose::Left, this.active, window, cx)
            }))
            .on_action(cx.listener(|this, _: &CloseTabsRight, window, cx| {
                this.close_tabs(TabClose::Right, this.active, window, cx)
            }))
            .on_action(cx.listener(|this, _: &NewTerminal, window, cx| {
                this.terminal.update(cx, |t, cx| t.new_session(window, cx))
            }))
            .on_action(
                cx.listener(|this, _: &FormatDocument, window, cx| {
                    this.format_document(window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &Save, window, cx| {
                if let Some(ix) = this.active {
                    this.save_tab(ix, false, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &SaveAll, window, cx| {
                for ix in 0..this.tabs.len() {
                    if matches!(&this.tabs[ix], Tab::File(f) if f.dirty) {
                        this.save_tab(ix, false, window, cx);
                    }
                }
            }))
            .on_action(cx.listener(|this, _: &CloseTab, window, cx| {
                if this.terminal.read(cx).is_focused(window, cx) {
                    this.terminal.update(cx, |t, cx| t.close_active(window, cx));
                } else if let Some(ix) = this.active {
                    this.close_tab(ix, window, cx);
                } else {
                    window.dispatch_action(Box::new(CloseWindow), cx);
                }
            }))
            .on_action(
                cx.listener(|this, _: &CloseWindow, window, cx| this.close_window(window, cx)),
            )
            .on_action(cx.listener(|this, _: &NextTab, window, cx| {
                if let Some(active) = this.active {
                    let n = this.tabs.len();
                    this.activate((active + 1) % n, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &PreviousTab, window, cx| {
                if let Some(active) = this.active {
                    let n = this.tabs.len();
                    this.activate((active + n - 1) % n, window, cx);
                }
            }))
            .on_action(
                cx.listener(|this, _: &OpenSettings, window, cx| this.open_settings(window, cx)),
            )
            .on_action(cx.listener(|this, _: &CheckForUpdates, window, cx| {
                // Show the result where the user can act on it.
                if this.settings.is_none() {
                    this.open_settings(window, cx);
                }
                if let Some(view) = this.settings.clone() {
                    view.update(cx, |view, cx| view.show_about(cx));
                }
                crate::updater::check(cx);
            }))
            .on_action(cx.listener(|this, _: &ReviewChanges, window, cx| {
                this.sidebar_open = true;
                this.panel = Panel::Changes;
                this.open_review(window, cx);
            }))
            .on_action(cx.listener(|this, _: &RefreshProject, window, cx| {
                if let Some(root) = this.root().map(Path::to_path_buf) {
                    let expanded: Vec<_> = this.tree.expanded.iter().cloned().collect();
                    for dir in expanded {
                        this.load_dir(dir, cx);
                    }
                    let _ = root;
                    this.refresh_git(cx);
                    this.start_index(cx);
                    this.review.update(cx, |r, cx| r.reload(cx));
                    this.reload_clean_files(None, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &RevealActiveFile, _, cx| {
                if let Some(path) = this.active_file().map(|f| f.path.clone()) {
                    this.sidebar_open = true;
                    this.panel = Panel::Files;
                    this.reveal(&path, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &RevealInFinder, _, cx| {
                if let Some(path) = this
                    .active_file()
                    .map(|f| f.path.clone())
                    .or_else(|| this.root().map(Path::to_path_buf))
                {
                    cx.reveal_path(&path);
                }
            }))
            .on_action(cx.listener(|this, _: &CopyPath, _, cx| {
                if let Some(path) = this.active_file().map(|f| f.path.display().to_string()) {
                    cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(path));
                    this.status = Some("Copied path".into());
                }
            }))
            .on_action(cx.listener(|this, _: &CopyRelativePath, _, cx| {
                if let (Some(root), Some(file)) = (this.root(), this.active_file()) {
                    let path = files::display_relative(root, &file.path);
                    cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(path));
                    this.status = Some("Copied relative path".into());
                }
            }))
            .on_action(cx.listener(|this, _: &ToggleMarkdownPreview, window, cx| {
                if let Some(ix) = this.active
                    && let Some(Tab::File(file)) = this.tabs.get_mut(ix)
                {
                    file.toggle_preview(window, cx);
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|_, _: &ToggleSoftWrap, _, cx| {
                Settings::update(cx, |s| s.soft_wrap = !s.soft_wrap)
            }))
            .on_action(cx.listener(|_, _: &ToggleReviewLayout, _, cx| {
                Settings::update(cx, |s| {
                    s.review_layout = match s.review_layout {
                        crate::settings::ReviewLayout::Unified => {
                            crate::settings::ReviewLayout::Split
                        }
                        crate::settings::ReviewLayout::Split => {
                            crate::settings::ReviewLayout::Unified
                        }
                    }
                })
            }))
            .on_action(cx.listener(|_, _: &Minimize, window, _| window.minimize_window()))
            .on_action(cx.listener(|_, _: &Zoom, window, _| window.zoom_window()))
            .on_action(cx.listener(|_, _: &About, window, cx| super::about::open(window, cx)))
            .when_some(sidebar, |root, sidebar| root.child(sidebar))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(tab_bar)
                    .child(
                        v_flex()
                            .flex_1()
                            .min_h_0()
                            .when_some(content, |col, content| {
                                col.child(div().flex_1().min_h_0().child(content))
                            })
                            .when(terminal_open, |col| col.child(terminal)),
                    )
                    .when(self.project.is_some() || !self.tabs.is_empty(), |col| {
                        col.child(status)
                    }),
            )
            .when_some(gradient, |root, gradient| root.child(gradient))
            .when_some(overlay, |root, overlay| {
                root.child(
                    deferred(
                        div()
                            .key_context("Overlay")
                            .absolute()
                            .inset_0()
                            .child(overlay),
                    )
                    .with_priority(1),
                )
            })
            .into_any_element()
    }
}

impl Session {
    pub fn get(cx: &App) -> &Session {
        cx.global::<Session>()
    }
    pub fn get_project(cx: &App, root: &Path) -> Option<ProjectSession> {
        cx.global::<Session>().project(root).cloned()
    }
}
