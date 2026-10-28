//! The window's root view: project lifecycle, tabs, panels, and commands.
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use gpui_kit::component::{
    ActiveTheme, IconName, TitleBar, WindowExt, h_flex, notification::Notification, v_flex,
};
use gpui_kit::{
    AnyElement, App, AppContext, Context, Entity, FocusHandle, Focusable, InteractiveElement,
    IntoElement, KeyBinding, MouseButton, ParentElement, PathPromptOptions, PromptLevel, Render,
    ScrollStrategy, SharedString, StatefulInteractiveElement, Styled, Subscription, Task,
    UniformListScrollHandle, WeakEntity, Window, actions, deferred, div, prelude::FluentBuilder,
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
        Save,
        SaveAll,
        CloseTab,
        NextTab,
        PreviousTab,
        OpenSettings,
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

pub fn init(cx: &mut App) {
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
        KeyBinding::new(k("cmd-s"), Save, Some(CONTEXT)),
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
fn quit(cx: &mut App) {
    let windows = cx.windows();
    if windows.is_empty() {
        cx.quit();
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
                return;
            }
        }
        cx.update(|cx| cx.quit());
    })
    .detach();
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Panel {
    Files,
    Search,
    Changes,
}

// A window holds a handful of tabs, so the size difference does not matter.
#[allow(clippy::large_enum_variant)]
pub enum Tab {
    File(FileTab),
    Review,
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
    /// Files the watcher saw change; reloaded on the next frame with a window.
    pending_reload: Option<HashSet<PathBuf>>,
    watcher: Option<Watcher>,
    _subscriptions: Vec<Subscription>,
    weak: WeakEntity<Self>,
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
            cx.observe_global::<Settings>(|this, cx| this.apply_settings(cx)),
            cx.observe(&terminal, |_, _, cx| cx.notify()),
            cx.observe_window_activation(window, |this, window, cx| {
                if window.is_window_active() {
                    this.refresh_git(cx);
                    this.reload_clean_files(None, window, cx);
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
            sidebar_open: true,
            sidebar_width: 252.,
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
            pending_reload: None,
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
        let name: SharedString = root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| root.display().to_string())
            .into();
        window.set_window_title(&name);
        let restored = Session::get_project(cx, &root);
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
        self.git = GitState::default();
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
            for file in session.open_files.iter().filter(|p| p.is_file()) {
                self.open_file_quietly(file.clone(), window, cx);
            }
            if let Some(active) = session.active_file
                && let Some(ix) = self.tab_for_path(&active)
            {
                self.activate(ix, window, cx);
            }
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
        let session = ProjectSession {
            root: project.root.clone(),
            open_files: self
                .tabs
                .iter()
                .filter_map(|t| match t {
                    Tab::File(f) => Some(f.path.clone()),
                    Tab::Review => None,
                })
                .collect(),
            active_file: self.active_file().map(|f| f.path.clone()),
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
        self.rebuild_rows();
        cx.notify();
    }

    // ---- Tree --------------------------------------------------------------

    pub fn load_dir(&mut self, dir: PathBuf, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let read = {
                let dir = dir.clone();
                cx.background_executor()
                    .spawn(async move { files::read_directory(&dir) })
                    .await
            };
            this.update(cx, |this, cx| {
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
                this.rebuild_rows();
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn rebuild_rows(&mut self) {
        let Some(root) = self.root().map(Path::to_path_buf) else {
            self.tree.rows.clear();
            return;
        };
        let show_hidden = true;
        let mut rows = Vec::with_capacity(self.tree.rows.len().max(64));
        fn walk(tree: &Tree, dir: &Path, depth: usize, show_hidden: bool, rows: &mut Vec<TreeRow>) {
            let Some(children) = tree.children.get(dir) else {
                return;
            };
            for entry in children {
                if !show_hidden && entry.is_hidden() {
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
                });
                if expanded {
                    walk(tree, &entry.path, depth + 1, show_hidden, rows);
                }
            }
        }
        walk(&self.tree, &root, 0, show_hidden, &mut rows);
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
        self.rebuild_rows();
        cx.notify();
    }

    pub fn collapse_all(&mut self, cx: &mut Context<Self>) {
        if let Some(root) = self.root().map(Path::to_path_buf) {
            self.tree.expanded.retain(|p| p == &root);
            self.rebuild_rows();
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
            self.rebuild_rows();
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
        let Some(root) = self.root().map(Path::to_path_buf) else {
            return;
        };
        if !Settings::get(cx).watch_files {
            return;
        }
        let (tx, mut rx) = futures::channel::mpsc::unbounded::<PathBuf>();
        let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            if let Ok(event) = event {
                for path in event.paths {
                    let _ = tx.unbounded_send(path);
                }
            }
        });
        let Ok(mut watcher) = watcher else { return };
        use notify::Watcher as _;
        if watcher
            .watch(&root, notify::RecursiveMode::Recursive)
            .is_err()
        {
            return;
        }
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

    fn files_changed(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let Some(root) = self.root().map(Path::to_path_buf) else {
            return;
        };
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
        for dir in dirs {
            self.load_dir(dir, cx);
        }
        if git_changed || worktree_changed {
            self.refresh_git(cx);
        }
        if worktree_changed {
            self.review.update(cx, |review, cx| review.mark_stale(cx));
            let changed: HashSet<PathBuf> = paths.into_iter().collect();
            self.pending_reload = Some(changed);
            cx.notify();
        }
        if created_or_removed {
            self.start_index(cx);
        }
    }

    // ---- Tabs --------------------------------------------------------------

    pub fn tab_for_path(&self, path: &Path) -> Option<usize> {
        self.tabs
            .iter()
            .position(|t| matches!(t, Tab::File(f) if f.path == path))
    }

    pub fn active_file(&self) -> Option<&FileTab> {
        match self.tabs.get(self.active?)? {
            Tab::File(file) => Some(file),
            Tab::Review => None,
        }
    }

    pub fn activate(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix >= self.tabs.len() {
            return;
        }
        self.active = Some(ix);
        match &self.tabs[ix] {
            Tab::File(file) => {
                let path = file.path.clone();
                file.focus(window, cx);
                self.tree.selected = Some(path);
            }
            Tab::Review => {
                self.review
                    .update(cx, |review, cx| review.focus(window, cx));
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
            self.activate(ix, window, cx);
            if let (Some(line), Some(Tab::File(file))) = (line, self.tabs.get(ix)) {
                file.go_to_line(line, window, cx);
            }
            return;
        }
        let weak = self.weak.clone();
        cx.spawn_in(window, async move |_, cx| {
            let loaded = {
                let path = path.clone();
                cx.background_executor()
                    .spawn(async move { crate::document::load(&path) })
                    .await
            };
            let _ = cx.update(|window, cx| {
                let Some(this) = weak.upgrade() else { return };
                this.update(cx, |this, cx| match loaded {
                    Ok(file) => {
                        if let Some(ix) = this.tab_for_path(&file.path) {
                            this.activate(ix, window, cx);
                            return;
                        }
                        let tab = FileTab::new(file, window, cx);
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
                });
            });
        })
        .detach();
    }

    /// Restore a tab without stealing focus or reporting failures.
    fn open_file_quietly(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if let Ok(file) = crate::document::load(&path) {
            let tab = FileTab::new(file, window, cx);
            self.tabs.push(Tab::File(tab));
            self.active = Some(self.tabs.len() - 1);
        }
    }

    pub fn close_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(ix) else { return };
        if let Tab::File(file) = tab
            && file.dirty
        {
            let name = file.name();
            let answer = window.prompt(
                PromptLevel::Warning,
                &format!("Save changes to {name}?"),
                Some("Your changes will be lost if you don't save them."),
                &["Save", "Don't Save", "Cancel"],
                cx,
            );
            let path = file.path.clone();
            cx.spawn_in(window, async move |this, cx| {
                let Ok(answer) = answer.await else { return };
                let _ = this.update_in(cx, |this, window, cx| {
                    let Some(ix) = this.tab_for_path(&path) else {
                        return;
                    };
                    match answer {
                        0 => this.save_tab(ix, true, window, cx),
                        1 => this.remove_tab(ix, window, cx),
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

    pub fn save_tab(
        &mut self,
        ix: usize,
        close_after: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(Tab::File(file)) = self.tabs.get_mut(ix) else {
            return;
        };
        let Some(job) = file.begin_save(cx) else {
            if close_after {
                self.remove_tab(ix, window, cx);
            }
            return;
        };
        let path = file.path.clone();
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
                match file.finish_save(result, cx) {
                    Ok(()) => {
                        this.status = Some(format!("Saved {}", file.name()).into());
                        if close_after {
                            this.remove_tab(ix, window, cx);
                        }
                        this.refresh_git(cx);
                    }
                    Err(error) => this.notify_error(error, window, cx),
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
        let answer = window.prompt(
            PromptLevel::Warning,
            &message,
            Some("Your changes will be lost if you don't save them."),
            &["Save All", "Don't Save", "Cancel"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| match answer.await {
            Ok(0) => {
                let saved =
                    this.update_in(cx, |this, window, cx| this.save_all_blocking(window, cx));
                saved.unwrap_or(false)
            }
            Ok(1) => {
                let _ = this.update(cx, |this, cx| this.discard_all(cx));
                true
            }
            _ => false,
        })
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
                continue;
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

    pub fn reload_clean_files(
        &mut self,
        only: Option<&HashSet<PathBuf>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for tab in &mut self.tabs {
            if let Tab::File(file) = tab
                && only.is_none_or(|set| set.contains(&file.path))
            {
                file.reload_if_changed(window, cx);
            }
        }
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
                if let Some(file) = self.active_file() {
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
            None => self.focus.focus(window, cx),
        }
    }

    pub fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let view = cx.new(|cx| SettingsView::new(window, cx));
        self._subscriptions.push(cx.subscribe_in(
            &view,
            window,
            |this, _, _: &super::settings_view::Close, window, cx| {
                this.settings = None;
                this.restore_focus(window, cx);
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
            super::review::ReviewEvent::BaseChanged => self.remember_session(cx),
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
        let left_pad = if !self.sidebar_open && is_mac() && !window.is_fullscreen() {
            r(78.)
        } else {
            r(8.)
        };
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
            h_flex()
                .id(("tab", ix))
                .group("tab")
                .flex_none()
                .h(r(28.))
                .max_w(r(220.))
                .pl(r(10.))
                .pr(r(4.))
                .gap(r(6.))
                .rounded(r(6.))
                .text_size(r(13.))
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
                .child(icon)
                .child(div().truncate().child(label))
                .child(
                    div()
                        .id(("close", ix))
                        .flex()
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
        });
        let terminal_open = self.terminal.read(cx).open;
        TitleBar::new()
            .h(r(42.))
            .pl(left_pad)
            .pr(r(8.))
            .bg(theme.background)
            .border_color(gpui_kit::transparent_black())
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .gap(r(2.))
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
                            .gap(r(2.))
                            .overflow_x_scroll()
                            .children(tabs),
                    ),
            )
            .child(
                h_flex().flex_none().gap(r(2.)).child(
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

    fn render_content(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if self.project.is_none() && self.tabs.is_empty() {
            return super::welcome::render(self, window, cx).into_any_element();
        }
        let root = self.root().map(Path::to_path_buf);
        match self.active.and_then(|ix| self.tabs.get_mut(ix)) {
            Some(Tab::File(file)) => file.render(root.as_deref(), window, cx).into_any_element(),
            Some(Tab::Review) => self.review.clone().into_any_element(),
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
            .h(r(26.))
            .px(r(8.))
            .gap(r(2.))
            .text_size(r(12.))
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
        static LOGGED: std::sync::Once = std::sync::Once::new();
        LOGGED.call_once(|| {
            if std::env::var_os("MODO_LOG_STARTUP").is_some()
                && let Some(started) = crate::STARTED.get()
            {
                eprintln!("modo: first frame after {:?}", started.elapsed());
            }
        });
        if let Some(paths) = self.pending_reload.take() {
            self.reload_clean_files(Some(&paths), window, cx);
        }
        let theme = cx.theme().clone();
        let sidebar = self
            .sidebar_open
            .then(|| super::sidebar::render(self, window, cx));
        let tab_bar = self.render_tab_bar(window, cx);
        let content = self.render_content(window, cx);
        let status = self.render_status_bar(cx);
        let terminal = self.terminal.clone();
        let terminal_open = terminal.read(cx).open;
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
            .on_action(cx.listener(|this, _: &NewTerminal, window, cx| {
                this.terminal.update(cx, |t, cx| t.new_session(window, cx))
            }))
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
                if let Some(ix) = this.active {
                    this.close_tab(ix, window, cx);
                } else if this.terminal.read(cx).is_focused(window, cx) {
                    this.terminal.update(cx, |t, cx| t.close_active(window, cx));
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
            .on_action(cx.listener(|this, _: &About, window, cx| {
                let _ = this;
                let detail = format!(
                    "Version {}\n{}",
                    crate::brand::VERSION,
                    crate::brand::TAGLINE
                );
                // The only button closes the dialog; no answer to wait for.
                drop(window.prompt(
                    PromptLevel::Info,
                    crate::brand::NAME,
                    Some(&detail),
                    &["OK"],
                    cx,
                ));
            }))
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
                            .child(div().flex_1().min_h_0().child(content))
                            .when(terminal_open, |col| col.child(terminal)),
                    )
                    .when(self.project.is_some() || !self.tabs.is_empty(), |col| {
                        col.child(status)
                    }),
            )
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
