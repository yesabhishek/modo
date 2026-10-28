//! The left sidebar: project switcher, navigation, and the Files / Search /
//! Changes panels, styled after Notion's quiet page tree.
use std::{
    ops::Range,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::{
    ActiveTheme, Icon, IconName, Sizable, StyledExt as _, h_flex,
    input::{Input, InputEvent, InputState},
    spinner::Spinner,
    v_flex,
};
use gpui_kit::{
    AnyElement, App, AppContext, Context, DragMoveEvent, Empty, Entity, EventEmitter, FontWeight,
    Hsla, InteractiveElement, IntoElement, MouseButton, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, StyledText, Subscription, Task, TextRun,
    UniformListScrollHandle, Window, div, prelude::FluentBuilder, uniform_list,
};

use super::{
    NavRow, drag_region, icon_button, is_mac, kbd, r, section_label, shortcut,
    workspace::{
        FindInFiles, OpenFolder, OpenSettings, Panel, QuickOpen, RefreshProject, ReviewChanges,
        ShowFiles, SwitchProject, ToggleSidebar, Workspace,
    },
};
use crate::{
    files,
    search::{self, Query, Results},
};

/// Marker carried while the sidebar edge is dragged.
#[derive(Clone)]
struct SidebarResize;

impl Render for SidebarResize {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

pub fn file_icon(path: &Path, cx: &App) -> Icon {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let icon = match ext.as_str() {
        "json" | "jsonc" | "toml" | "yaml" | "yml" | "lock" => Lucide::FileBraces,
        "md" | "markdown" | "mdx" | "txt" | "rst" => Lucide::FileText,
        "png" | "jpg" | "jpeg" | "gif" | "svg" | "webp" | "ico" | "icns" => Lucide::FileImage,
        "sh" | "bash" | "zsh" | "fish" | "ps1" => Lucide::FileTerminal,
        "ttf" | "otf" | "woff" | "woff2" => Lucide::FileType,
        "diff" | "patch" => Lucide::FileDiff,
        _ if files::language_for(path) != "text" => Lucide::FileCode,
        _ if name.starts_with('.') => Lucide::File,
        _ => Lucide::File,
    };
    Icon::new(icon)
        .size(r(15.))
        .text_color(cx.theme().muted_foreground)
}

fn status_color(badge: &str, cx: &App) -> Hsla {
    match badge {
        "A" => cx.theme().green,
        "D" => cx.theme().red,
        "!" => cx.theme().red,
        "R" => cx.theme().blue,
        _ => cx.theme().yellow,
    }
}

pub fn render(
    this: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> AnyElement {
    let theme = cx.theme().clone();
    let width = this.sidebar_width;
    let top_pad = if is_mac() && !window.is_fullscreen() {
        r(78.)
    } else {
        r(10.)
    };
    let review_count = {
        let review = this.review.read(cx);
        if review.is_loaded() {
            review.files().len()
        } else {
            this.change_count()
        }
    };
    let panel = this.panel;

    let header = drag_region("sidebar-drag")
        .flex()
        .flex_none()
        .items_center()
        .h(r(42.))
        .pl(top_pad)
        .pr(r(8.))
        .child(div().flex_1())
        .child(
            icon_button("collapse-sidebar", Lucide::PanelLeft, cx)
                .tooltip(|window, cx| tooltip("Hide sidebar", &ToggleSidebar, window, cx))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.sidebar_open = false;
                    cx.notify();
                })),
        );

    let project_row = {
        let (letter, name) = match &this.project {
            Some(project) => (
                project
                    .name
                    .chars()
                    .next()
                    .unwrap_or('M')
                    .to_uppercase()
                    .to_string(),
                project.name.clone(),
            ),
            None => ("M".to_owned(), SharedString::from(crate::brand::NAME)),
        };
        let hover = theme.list_hover;
        h_flex()
            .id("project-switcher")
            .h(r(32.))
            .mx(r(6.))
            .px(r(6.))
            .gap(r(8.))
            .rounded(r(6.))
            .cursor_pointer()
            .hover(move |s| s.bg(hover))
            .on_click(|_, window, cx| window.dispatch_action(Box::new(SwitchProject), cx))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(r(22.))
                    .rounded(r(5.))
                    .bg(theme.foreground)
                    .text_color(theme.background)
                    .text_size(r(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(letter),
            )
            .child(
                div()
                    .flex_1()
                    .truncate()
                    .text_size(r(14.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(name),
            )
            .child(
                Icon::new(Lucide::ChevronsUpDown)
                    .size(r(14.))
                    .text_color(theme.muted_foreground),
            )
    };

    let nav = v_flex()
        .flex_none()
        .py(r(4.))
        .gap(r(1.))
        .child(
            NavRow::new("nav-search", "Search")
                .icon(Lucide::TextSearch)
                .selected(panel == Panel::Search)
                .trailing(kbd(shortcut("⌘⇧F", "Ctrl+Shift+F"), cx))
                .on_click(cx.listener(|this, _, window, cx| {
                    if this.panel == Panel::Search {
                        this.panel = Panel::Files;
                        cx.notify();
                    } else {
                        window.dispatch_action(Box::new(FindInFiles), cx);
                    }
                })),
        )
        .child(
            NavRow::new("nav-goto", "Go to file")
                .icon(Lucide::Command)
                .trailing(kbd(shortcut("⌘P", "Ctrl+P"), cx))
                .on_click(|_, window, cx| window.dispatch_action(Box::new(QuickOpen), cx)),
        )
        .child(
            NavRow::new("nav-review", "Review changes")
                .icon(Lucide::GitCompare)
                .selected(panel == Panel::Changes)
                .when(review_count > 0, |row| {
                    row.trailing(
                        div()
                            .px(r(6.))
                            .rounded(r(4.))
                            .bg(theme.yellow_light)
                            .text_color(theme.yellow)
                            .text_size(r(11.5))
                            .font_medium()
                            .child(review_count.to_string()),
                    )
                })
                .on_click(cx.listener(|this, _, window, cx| {
                    if this.panel == Panel::Changes {
                        this.panel = Panel::Files;
                        cx.notify();
                    } else {
                        window.dispatch_action(Box::new(ReviewChanges), cx);
                    }
                })),
        )
        .child(
            NavRow::new("nav-settings", "Settings")
                .icon(IconName::Settings)
                .trailing(kbd(shortcut("⌘,", "Ctrl+,"), cx))
                .on_click(|_, window, cx| window.dispatch_action(Box::new(OpenSettings), cx)),
        );

    let body: AnyElement = if this.project.is_none() {
        v_flex()
            .px(r(14.))
            .pt(r(16.))
            .gap(r(8.))
            .text_size(r(13.))
            .text_color(theme.muted_foreground)
            .child("No folder open")
            .child(
                div()
                    .id("sidebar-open")
                    .text_color(theme.primary)
                    .cursor_pointer()
                    .child("Open a folder…")
                    .on_click(|_, window, cx| window.dispatch_action(Box::new(OpenFolder), cx)),
            )
            .into_any_element()
    } else {
        match panel {
            Panel::Files => render_files(this, cx),
            Panel::Search => v_flex()
                .size_full()
                .child(panel_header("Search", [close_panel(cx)], cx))
                .child(div().flex_1().min_h_0().child(this.search.clone()))
                .into_any_element(),
            Panel::Changes => render_changes(this, cx),
        }
    };

    v_flex()
        .id("sidebar")
        .relative()
        .flex_none()
        .w(gpui_kit::px(width))
        .h_full()
        .bg(theme.sidebar)
        .border_r_1()
        .border_color(theme.sidebar_border)
        .child(header)
        .child(project_row)
        .child(nav)
        .child(div().flex_1().min_h_0().pt(r(6.)).child(body))
        .child(
            div()
                .id("sidebar-resize")
                .absolute()
                .top_0()
                .bottom_0()
                .right(gpui_kit::px(-3.))
                .w(gpui_kit::px(6.))
                .cursor_col_resize()
                .on_drag(SidebarResize, |_, _, _, cx| cx.new(|_| SidebarResize))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()),
        )
        .on_drag_move(
            cx.listener(|this, event: &DragMoveEvent<SidebarResize>, _, cx| {
                let x: f32 = (event.event.position.x - event.bounds.left()).into();
                this.sidebar_width = x.clamp(190., 520.);
                cx.notify();
            }),
        )
        .into_any_element()
}

fn tooltip(
    label: &'static str,
    action: &dyn gpui_kit::Action,
    window: &mut Window,
    cx: &mut App,
) -> gpui_kit::AnyView {
    gpui_kit::component::tooltip::Tooltip::new(label)
        .key_binding(gpui_kit::component::kbd::Kbd::binding_for_action(
            action, None, window,
        ))
        .build(window, cx)
}

fn panel_header(
    title: &'static str,
    actions: impl IntoIterator<Item = AnyElement>,
    cx: &App,
) -> impl IntoElement {
    h_flex()
        .flex_none()
        .h(r(26.))
        .pl(r(14.))
        .pr(r(8.))
        .gap(r(2.))
        .group("panel-header")
        .child(section_label(title, cx).flex_1())
        .child(
            h_flex()
                .gap(r(2.))
                .invisible()
                .group_hover("panel-header", |s| s.visible())
                .children(actions),
        )
}

fn render_files(this: &mut Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let count = this.tree.rows.len();
    let scroll = this.tree.scroll.clone();
    let header = panel_header(
        "Files",
        [
            icon_button("files-collapse", Lucide::ChevronsDownUp, cx)
                .on_click(cx.listener(|this, _, _, cx| this.collapse_all(cx)))
                .into_any_element(),
            icon_button("files-refresh", IconName::RefreshCw, cx)
                .on_click(|_, window, cx| window.dispatch_action(Box::new(RefreshProject), cx))
                .into_any_element(),
        ],
        cx,
    );
    let error = this.tree.error.clone();
    v_flex()
        .size_full()
        .child(header)
        .when_some(error, |col, error| {
            col.child(
                div()
                    .px(r(14.))
                    .text_size(r(12.5))
                    .text_color(cx.theme().red)
                    .child(error),
            )
        })
        .child(
            uniform_list(
                "file-tree",
                count,
                cx.processor(|this: &mut Workspace, range: Range<usize>, _, cx| {
                    range
                        .filter_map(|ix| this.tree.rows.get(ix).cloned().map(|row| (ix, row)))
                        .map(|(ix, row)| render_tree_row(this, ix, row, cx))
                        .collect::<Vec<_>>()
                }),
            )
            .track_scroll(&scroll)
            .flex_1()
            .pb(r(12.)),
        )
        .into_any_element()
}

fn render_tree_row(
    this: &Workspace,
    ix: usize,
    row: super::workspace::TreeRow,
    cx: &mut Context<Workspace>,
) -> AnyElement {
    let theme = cx.theme().clone();
    let selected = this.tree.selected.as_ref() == Some(&row.path)
        || this.active_file().is_some_and(|f| f.path == row.path);
    let change = (!row.directory)
        .then(|| this.change_for(&row.path))
        .flatten();
    let badge = change.map(|c| c.badge());
    let dir_dirty = row.directory && this.git.dirty_dirs.contains(&row.path);
    let hover = theme.list_hover;
    let name_color = match badge {
        Some(b) => status_color(b, cx),
        None if row.hidden => theme.muted_foreground,
        None => theme.sidebar_foreground,
    };
    let icon = if row.directory {
        Icon::new(if row.expanded {
            IconName::ChevronDown
        } else {
            IconName::ChevronRight
        })
        .size(r(14.))
        .text_color(theme.muted_foreground)
        .into_any_element()
    } else {
        file_icon(&row.path, cx).into_any_element()
    };
    let path = row.path.clone();
    let directory = row.directory;
    let row_el = h_flex()
        .id(("tree-row", ix))
        .w_full()
        .h(r(27.))
        .pl(r(8. + row.depth as f32 * 14.))
        .pr(r(8.))
        .gap(r(6.))
        .rounded(r(5.))
        .text_size(r(13.5))
        .text_color(name_color)
        .cursor_pointer()
        .when(selected, |r| {
            r.bg(theme.list_active).text_color(theme.foreground)
        })
        .when(!selected, |r| r.hover(move |s| s.bg(hover)))
        .child(icon)
        .child(div().flex_1().truncate().child(row.name.clone()))
        .when(dir_dirty, |r| {
            r.child(
                div()
                    .size(super::r(6.))
                    .rounded_full()
                    .bg(theme.yellow.opacity(0.8)),
            )
        })
        .when_some(badge, |r, badge| {
            r.child(
                div()
                    .text_size(super::r(11.5))
                    .font_medium()
                    .text_color(status_color(badge, cx))
                    .child(badge),
            )
        })
        .on_click(cx.listener(move |this, _, window, cx| {
            if directory {
                this.toggle_dir(path.clone(), cx);
            } else {
                this.tree.selected = Some(path.clone());
                this.open_file(path.clone(), None, window, cx);
            }
        }));
    div().w_full().px(r(6.)).child(row_el).into_any_element()
}

fn render_changes(this: &mut Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let theme = cx.theme().clone();
    let review = this.review.read(cx);
    let summary = review.summary(cx);
    let count = review.files().len();
    let worktrees = this
        .git
        .repo
        .as_ref()
        .map(|r| r.worktrees.clone())
        .unwrap_or_default();
    let root = this.root().map(Path::to_path_buf);
    let branch = this.git.repo.as_ref().map(|r| r.branch.clone());
    let no_repo = this.git.repo.is_none();

    let mut col = v_flex().size_full().child(panel_header(
        "Changes",
        [
            icon_button("changes-refresh", IconName::RefreshCw, cx)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.refresh_git(cx);
                    this.review.update(cx, |r, cx| r.reload(cx));
                }))
                .into_any_element(),
            close_panel(cx),
        ],
        cx,
    ));
    if no_repo {
        return col
            .child(
                div()
                    .px(r(14.))
                    .text_size(r(13.))
                    .text_color(theme.muted_foreground)
                    .child("This folder is not a Git repository."),
            )
            .into_any_element();
    }
    // Worktrees agents created alongside this checkout.
    if worktrees.len() > 1 {
        col = col.child(
            div()
                .px(r(14.))
                .pt(r(4.))
                .pb(r(2.))
                .child(section_label("Worktrees", cx)),
        );
        for (ix, tree) in worktrees.iter().enumerate() {
            let current = root.as_deref() == Some(tree.path.as_path());
            let path = tree.path.clone();
            col = col.child(
                NavRow::new(("worktree", ix), tree.to_string())
                    .icon(Lucide::FolderGit2)
                    .selected(current)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.switch_worktree(path.clone(), window, cx)
                    })),
            );
        }
        col = col.child(div().h(r(8.)));
    } else if let Some(branch) = branch {
        col = col.child(
            h_flex()
                .px(r(14.))
                .h(r(26.))
                .gap(r(6.))
                .text_size(r(13.))
                .text_color(theme.sidebar_foreground)
                .child(
                    Icon::new(Lucide::GitBranch)
                        .size(r(14.))
                        .text_color(theme.muted_foreground),
                )
                .child(div().truncate().child(branch)),
        );
    }
    col = col.child(super::review::compare_picker(&this.review, cx));
    col = col.child(
        h_flex()
            .px(r(14.))
            .pt(r(8.))
            .pb(r(4.))
            .gap(r(6.))
            .text_size(r(12.))
            .text_color(theme.muted_foreground)
            .child(div().flex_1().child(summary.label))
            .when(summary.loading, |row| row.child(Spinner::new().xsmall()))
            .when(summary.additions + summary.deletions > 0, |row| {
                row.child(
                    div()
                        .text_color(theme.green)
                        .child(format!("+{}", summary.additions)),
                )
                .child(
                    div()
                        .text_color(theme.red)
                        .child(format!("−{}", summary.deletions)),
                )
            }),
    );
    // Branch reviews can touch thousands of files; draw only the visible rows.
    let list = uniform_list(
        "changes-list",
        count,
        cx.processor(|this: &mut Workspace, range: Range<usize>, _, cx| {
            let files = this.review.read(cx).files();
            range
                .filter_map(|ix| files.get(ix).cloned().map(|file| (ix, file)))
                .collect::<Vec<_>>()
                .into_iter()
                .map(|(ix, file)| render_change_row(ix, file, cx))
                .collect::<Vec<_>>()
        }),
    )
    .track_scroll(&this.changes_scroll)
    .flex_1()
    .pb(r(12.));
    col.child(list).into_any_element()
}

fn render_change_row(
    ix: usize,
    file: super::review::ReviewFile,
    cx: &mut Context<Workspace>,
) -> AnyElement {
    let theme = cx.theme().clone();
    let hover = theme.list_hover;
    let path = file.path.clone();
    let name = Path::new(path.as_ref())
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let dir = Path::new(path.as_ref())
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let color = status_color(file.badge, cx);
    let count = |text: String, color: Hsla| {
        div()
            .flex_none()
            .text_size(r(11.5))
            .text_color(color)
            .child(text)
    };
    let row = h_flex()
        .id(("change", ix))
        .w_full()
        .h(r(27.))
        .px(r(8.))
        .gap(r(6.))
        .rounded(r(5.))
        .text_size(r(13.))
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .child(file_icon(Path::new(path.as_ref()), cx).flex_none())
        .child(
            // The folder only takes space the name leaves; a long name truncates
            // before it can run into the counts.
            h_flex()
                .flex_1()
                .min_w_0()
                .gap(r(6.))
                .overflow_hidden()
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_color(theme.sidebar_foreground)
                        .child(name),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(r(12.))
                        .text_color(theme.muted_foreground)
                        .child(dir),
                ),
        )
        .when(file.additions > 0, |r| {
            r.child(count(format!("+{}", file.additions), theme.green))
        })
        .when(file.deletions > 0, |r| {
            r.child(count(format!("−{}", file.deletions), theme.red))
        })
        .child(
            div()
                .flex_none()
                .w(r(10.))
                .text_size(r(11.5))
                .font_medium()
                .text_color(color)
                .child(file.badge),
        )
        .on_click(cx.listener(move |this, _, window, cx| {
            this.open_review(window, cx);
            let path = path.clone();
            this.review
                .update(cx, |review, cx| review.scroll_to_file(&path, cx));
        }));
    div().w_full().px(r(6.)).child(row).into_any_element()
}

/// Return the sidebar to the file tree.
fn close_panel(cx: &mut Context<Workspace>) -> AnyElement {
    icon_button("panel-close", IconName::Close, cx)
        .tooltip(|window, cx| tooltip("Back to files", &ShowFiles, window, cx))
        .on_click(cx.listener(|this, _, _, cx| {
            this.panel = Panel::Files;
            cx.notify();
        }))
        .into_any_element()
}

/// Inset a list row so its hover fill keeps a margin from the sidebar edge.
trait PipeRow {
    fn pipe_row(self) -> AnyElement;
}

impl PipeRow for AnyElement {
    fn pipe_row(self) -> AnyElement {
        div().w_full().px(r(6.)).child(self).into_any_element()
    }
}

// ---- Project search ---------------------------------------------------------

pub enum SearchEvent {
    Open(PathBuf, u32),
}

#[derive(Clone)]
enum SearchRow {
    File { file: usize },
    Line { file: usize, line: usize },
}

pub struct SearchPanel {
    input: Entity<InputState>,
    query: Query,
    root: Option<PathBuf>,
    results: Arc<Results>,
    rows: Vec<SearchRow>,
    collapsed: std::collections::HashSet<usize>,
    error: Option<SharedString>,
    running: Option<(Task<()>, Arc<AtomicBool>)>,
    scroll: UniformListScrollHandle,
    _subscription: Subscription,
}

impl EventEmitter<SearchEvent> for SearchPanel {}

impl SearchPanel {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Search in files"));
        let subscription = cx.subscribe_in(
            &input,
            window,
            |this, input, event: &InputEvent, _, cx| match event {
                InputEvent::Change => {
                    this.query.text = input.read(cx).value().to_string();
                    this.schedule(cx);
                }
                InputEvent::PressEnter { .. } => this.schedule(cx),
                _ => {}
            },
        );
        Self {
            input,
            query: Query::default(),
            root: None,
            results: Arc::default(),
            rows: Vec::new(),
            collapsed: Default::default(),
            error: None,
            running: None,
            scroll: UniformListScrollHandle::new(),
            _subscription: subscription,
        }
    }

    pub fn set_root(&mut self, root: Option<PathBuf>, cx: &mut Context<Self>) {
        if self.root != root {
            self.root = root;
            self.results = Arc::default();
            self.rows.clear();
            self.schedule(cx);
        }
    }

    pub fn focus(&mut self, prefill: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |input, cx| {
            if let Some(text) = prefill {
                input.set_value(text, window, cx);
            }
            input.focus(window, cx);
        });
        if let Some(text) = Some(self.input.read(cx).value().to_string())
            && text != self.query.text
        {
            self.query.text = text;
            self.schedule(cx);
        }
    }

    fn schedule(&mut self, cx: &mut Context<Self>) {
        if let Some((_, cancel)) = self.running.take() {
            cancel.store(true, Ordering::Relaxed);
        }
        let Some(root) = self.root.clone() else {
            return;
        };
        let query = self.query.clone();
        if query.text.trim().is_empty() {
            self.results = Arc::default();
            self.rows.clear();
            self.error = None;
            cx.notify();
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let task = cx.spawn(async move |this, cx| {
            // Debounce typing.
            cx.background_executor()
                .timer(Duration::from_millis(140))
                .await;
            if flag.load(Ordering::Relaxed) {
                return;
            }
            let background = flag.clone();
            let result = cx
                .background_executor()
                .spawn(async move { search::search(&root, &query, &background) })
                .await;
            if flag.load(Ordering::Relaxed) {
                return;
            }
            this.update(cx, |this, cx| {
                this.running = None;
                match result {
                    Ok(results) => {
                        this.error = None;
                        this.results = Arc::new(results);
                        this.collapsed.clear();
                        this.rebuild_rows();
                    }
                    Err(error) => this.error = Some(error.into()),
                }
                cx.notify();
            })
            .ok();
        });
        self.running = Some((task, cancel));
        cx.notify();
    }

    fn rebuild_rows(&mut self) {
        self.rows.clear();
        for (file, matches) in self.results.files.iter().enumerate() {
            self.rows.push(SearchRow::File { file });
            if !self.collapsed.contains(&file) {
                for line in 0..matches.lines.len() {
                    self.rows.push(SearchRow::Line { file, line });
                }
            }
        }
    }

    fn toggle(&mut self, option: fn(&mut Query) -> &mut bool, cx: &mut Context<Self>) {
        let flag = option(&mut self.query);
        *flag = !*flag;
        self.schedule(cx);
    }

    fn render_row(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let hover = theme.list_hover;
        match self.rows[ix].clone() {
            SearchRow::File { file } => {
                let matches = &self.results.files[file];
                let path = Path::new(matches.relative.as_ref());
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let dir = path
                    .parent()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let collapsed = self.collapsed.contains(&file);
                h_flex()
                    .id(("search-file", ix))
                    .w_full()
                    .h(r(26.))
                    .px(r(6.))
                    .gap(r(6.))
                    .rounded(r(5.))
                    .text_size(r(13.))
                    .cursor_pointer()
                    .hover(move |s| s.bg(hover))
                    .child(
                        Icon::new(if collapsed {
                            IconName::ChevronRight
                        } else {
                            IconName::ChevronDown
                        })
                        .size(r(13.))
                        .text_color(theme.muted_foreground),
                    )
                    .child(file_icon(path, cx))
                    .child(div().flex_none().text_color(theme.foreground).child(name))
                    .child(
                        div()
                            .flex_1()
                            .truncate()
                            .text_size(r(12.))
                            .text_color(theme.muted_foreground)
                            .child(dir),
                    )
                    .child(
                        div()
                            .px(r(5.))
                            .rounded(r(4.))
                            .bg(theme.list_hover)
                            .text_size(r(11.))
                            .text_color(theme.muted_foreground)
                            .child(matches.lines.len().to_string()),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.collapsed.remove(&file) {
                            this.collapsed.insert(file);
                        }
                        this.rebuild_rows();
                        cx.notify();
                    }))
                    .into_any_element()
                    .pipe_row()
            }
            SearchRow::Line { file, line } => {
                let matches = &self.results.files[file];
                let found = &matches.lines[line];
                let path = matches.path.clone();
                let line_number = found.line;
                let mut runs = Vec::new();
                let base = gpui_kit::TextStyle::default();
                let _ = base;
                let font = gpui_kit::font(theme.font_family.clone());
                let mut cursor = 0;
                for range in &found.ranges {
                    if range.start > cursor {
                        runs.push(text_run(
                            range.start - cursor,
                            &font,
                            theme.sidebar_foreground,
                            None,
                        ));
                    }
                    runs.push(text_run(
                        range.end - range.start,
                        &font,
                        theme.foreground,
                        Some(theme.yellow_light),
                    ));
                    cursor = range.end;
                }
                if cursor < found.text.len() {
                    runs.push(text_run(
                        found.text.len() - cursor,
                        &font,
                        theme.sidebar_foreground,
                        None,
                    ));
                }
                h_flex()
                    .id(("search-line", ix))
                    .w_full()
                    .h(r(24.))
                    .pl(r(30.))
                    .pr(r(6.))
                    .gap(r(8.))
                    .rounded(r(5.))
                    .text_size(r(12.5))
                    .cursor_pointer()
                    .hover(move |s| s.bg(hover))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .child(StyledText::new(found.text.clone()).with_runs(runs)),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_size(r(11.))
                            .text_color(theme.muted_foreground)
                            .child(line_number.to_string()),
                    )
                    .on_click(cx.listener(move |_, _, _, cx| {
                        cx.emit(SearchEvent::Open(path.clone(), line_number));
                    }))
                    .into_any_element()
                    .pipe_row()
            }
        }
    }
}

fn text_run(len: usize, font: &gpui_kit::Font, color: Hsla, background: Option<Hsla>) -> TextRun {
    TextRun {
        len,
        font: font.clone(),
        color,
        background_color: background,
        underline: None,
        strikethrough: None,
    }
}

impl Render for SearchPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let toggle = |id: &'static str, label: &'static str, on: bool, tip: &'static str| {
            let hover = theme.list_hover;
            div()
                .id(id)
                .px(r(5.))
                .h(r(20.))
                .flex()
                .items_center()
                .rounded(r(4.))
                .text_size(r(11.5))
                .font_medium()
                .cursor_pointer()
                .when(on, |d| {
                    d.bg(theme.primary.opacity(0.14)).text_color(theme.primary)
                })
                .when(!on, |d| {
                    d.text_color(theme.muted_foreground)
                        .hover(move |s| s.bg(hover))
                })
                .child(label)
                .tooltip(move |window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new(tip).build(window, cx)
                })
        };
        let summary: SharedString = if let Some(error) = &self.error {
            error.clone()
        } else if self.running.is_some() {
            "Searching…".into()
        } else if self.query.text.is_empty() {
            "".into()
        } else if self.results.total == 0 {
            "No results".into()
        } else {
            format!(
                "{}{} results in {} files",
                self.results.total,
                if self.results.limited { "+" } else { "" },
                self.results.files.len()
            )
            .into()
        };
        let count = self.rows.len();
        v_flex()
            .size_full()
            .child(
                v_flex()
                    .flex_none()
                    .px(r(10.))
                    .pb(r(6.))
                    .gap(r(6.))
                    .child(
                        Input::new(&self.input)
                            .small()
                            .prefix(
                                Icon::new(IconName::Search)
                                    .size(r(14.))
                                    .text_color(theme.muted_foreground),
                            )
                            .suffix(
                                h_flex()
                                    .gap(r(1.))
                                    .child(
                                        toggle(
                                            "case",
                                            "Aa",
                                            self.query.case_sensitive,
                                            "Match case",
                                        )
                                        .on_click(
                                            cx.listener(|this, _, _, cx| {
                                                this.toggle(|q| &mut q.case_sensitive, cx)
                                            }),
                                        ),
                                    )
                                    .child(
                                        toggle("word", "ab", self.query.whole_word, "Whole word")
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.toggle(|q| &mut q.whole_word, cx)
                                            })),
                                    )
                                    .child(
                                        toggle(
                                            "regex",
                                            ".*",
                                            self.query.regex,
                                            "Regular expression",
                                        )
                                        .on_click(
                                            cx.listener(|this, _, _, cx| {
                                                this.toggle(|q| &mut q.regex, cx)
                                            }),
                                        ),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .px(r(4.))
                            .text_size(r(12.))
                            .text_color(if self.error.is_some() {
                                theme.red
                            } else {
                                theme.muted_foreground
                            })
                            .child(summary),
                    ),
            )
            .child(
                uniform_list(
                    "search-results",
                    count,
                    cx.processor(|this: &mut Self, range: Range<usize>, _, cx| {
                        range.map(|ix| this.render_row(ix, cx)).collect::<Vec<_>>()
                    }),
                )
                .track_scroll(&self.scroll)
                .flex_1()
                .pb(r(12.)),
            )
    }
}
