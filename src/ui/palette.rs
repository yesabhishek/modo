//! One search box for everything, Notion style: files by default, `>` for
//! commands, `:` for a line number, `@` for recent projects.
use std::{
    ops::Range,
    path::{Path, PathBuf},
    sync::Arc,
};

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::{
    ActiveTheme, Icon, IconName, Sizable, StyledExt as _, h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};
use gpui_kit::{
    Action, AnyElement, App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable,
    FontWeight, Hsla, InteractiveElement, IntoElement, KeyBinding, MouseButton, ParentElement,
    Render, ScrollStrategy, SharedString, StatefulInteractiveElement, Styled, StyledText,
    Subscription, TextRun, UniformListScrollHandle, WeakEntity, Window, actions, div,
    prelude::FluentBuilder, uniform_list,
};

use super::{
    r, shortcut,
    workspace::{self as ws, Workspace},
};
use crate::{files::FileIndex, search::Fuzzy};

actions!(palette, [SelectPrevious, SelectNext, Close]);

const CONTEXT: &str = "Palette";

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("up", SelectPrevious, Some(CONTEXT)),
        KeyBinding::new("down", SelectNext, Some(CONTEXT)),
        KeyBinding::new("ctrl-p", SelectPrevious, Some(CONTEXT)),
        KeyBinding::new("ctrl-n", SelectNext, Some(CONTEXT)),
        KeyBinding::new("escape", Close, Some(CONTEXT)),
    ]);
}

pub enum PaletteEvent {
    Dismiss,
    OpenFile(PathBuf),
    OpenProject(PathBuf),
    GoToLine(u32),
    Run(Box<dyn Action>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Files,
    Commands,
    Line,
    Projects,
}

struct Command {
    label: &'static str,
    action: Box<dyn Action>,
    keys: &'static str,
    icon: Lucide,
}

fn commands() -> Vec<Command> {
    let c = |label, action: Box<dyn Action>, mac, other, icon| Command {
        label,
        action,
        keys: shortcut(mac, other),
        icon,
    };
    vec![
        c(
            "Review changes",
            Box::new(ws::ReviewChanges),
            "⌘⇧G",
            "Ctrl+Shift+G",
            Lucide::GitCompare,
        ),
        c(
            "Go to file",
            Box::new(ws::QuickOpen),
            "⌘P",
            "Ctrl+P",
            Lucide::File,
        ),
        c(
            "Search in files",
            Box::new(ws::FindInFiles),
            "⌘⇧F",
            "Ctrl+Shift+F",
            Lucide::TextSearch,
        ),
        c(
            "Go to line",
            Box::new(ws::GoToLine),
            "⌃G",
            "Ctrl+G",
            Lucide::CornerDownLeft,
        ),
        c(
            "Open folder…",
            Box::new(ws::OpenFolder),
            "⌘O",
            "Ctrl+O",
            Lucide::FolderOpen,
        ),
        c(
            "Switch project…",
            Box::new(ws::SwitchProject),
            "",
            "",
            Lucide::FolderGit2,
        ),
        c(
            "New window",
            Box::new(ws::NewWindow),
            "⌘⇧N",
            "Ctrl+Shift+N",
            Lucide::Layers,
        ),
        c(
            "Toggle terminal",
            Box::new(ws::ToggleTerminal),
            "⌘J",
            "Ctrl+J",
            Lucide::Terminal,
        ),
        c(
            "New terminal",
            Box::new(ws::NewTerminal),
            "⌃⇧`",
            "Ctrl+Shift+`",
            Lucide::Terminal,
        ),
        c(
            "Toggle sidebar",
            Box::new(ws::ToggleSidebar),
            "⌘B",
            "Ctrl+B",
            Lucide::PanelLeft,
        ),
        c(
            "Show files",
            Box::new(ws::ShowFiles),
            "⌘⇧E",
            "Ctrl+Shift+E",
            Lucide::ListTree,
        ),
        c(
            "Reveal active file in sidebar",
            Box::new(ws::RevealActiveFile),
            "",
            "",
            Lucide::ListTree,
        ),
        c("Save", Box::new(ws::Save), "⌘S", "Ctrl+S", Lucide::Save),
        c(
            "Save all",
            Box::new(ws::SaveAll),
            "⌥⌘S",
            "Ctrl+Alt+S",
            Lucide::Save,
        ),
        c(
            "Close tab",
            Box::new(ws::CloseTab),
            "⌘W",
            "Ctrl+W",
            Lucide::X,
        ),
        c(
            "Toggle word wrap",
            Box::new(ws::ToggleSoftWrap),
            "⌥Z",
            "Alt+Z",
            Lucide::TextWrap,
        ),
        c(
            "Toggle split / unified review",
            Box::new(ws::ToggleReviewLayout),
            "",
            "",
            Lucide::Columns2,
        ),
        c(
            "Refresh project",
            Box::new(ws::RefreshProject),
            "",
            "",
            Lucide::FolderGit2,
        ),
        c(
            if cfg!(target_os = "macos") {
                "Reveal in Finder"
            } else {
                "Reveal in file manager"
            },
            Box::new(ws::RevealInFinder),
            "",
            "",
            Lucide::FolderOpen,
        ),
        c(
            "Copy file path",
            Box::new(ws::CopyPath),
            "",
            "",
            Lucide::File,
        ),
        c(
            "Copy relative path",
            Box::new(ws::CopyRelativePath),
            "",
            "",
            Lucide::File,
        ),
        c(
            "Settings",
            Box::new(ws::OpenSettings),
            "⌘,",
            "Ctrl+,",
            Lucide::SlidersHorizontal,
        ),
        c(
            "Zoom in",
            Box::new(ws::ZoomIn),
            "⌘=",
            "Ctrl+=",
            Lucide::Type,
        ),
        c(
            "Zoom out",
            Box::new(ws::ZoomOut),
            "⌘-",
            "Ctrl+-",
            Lucide::Type,
        ),
        c(
            "Reset zoom",
            Box::new(ws::ZoomReset),
            "⌘0",
            "Ctrl+0",
            Lucide::Type,
        ),
        c("About Modo", Box::new(ws::About), "", "", Lucide::CircleDot),
    ]
}

struct Item {
    icon: AnyIcon,
    title: SharedString,
    detail: Option<SharedString>,
    keys: &'static str,
    positions: Vec<u32>,
    target: Target,
}

enum AnyIcon {
    Path(PathBuf),
    Lucide(Lucide),
}

enum Target {
    File(PathBuf),
    Project(PathBuf),
    Command(usize),
    Line(u32),
    OpenFolder,
}

pub struct Palette {
    input: Entity<InputState>,
    focus: FocusHandle,
    root: Option<PathBuf>,
    index: Option<Arc<FileIndex>>,
    recent: Vec<PathBuf>,
    commands: Vec<Command>,
    fuzzy: Fuzzy,
    mode: Mode,
    items: Vec<Item>,
    selected: usize,
    scroll: UniformListScrollHandle,
    _workspace: WeakEntity<Workspace>,
    _subscription: Subscription,
}

impl EventEmitter<PaletteEvent> for Palette {}

impl Palette {
    pub fn new(
        initial: &str,
        root: Option<PathBuf>,
        index: Option<Arc<FileIndex>>,
        recent: Vec<PathBuf>,
        workspace: WeakEntity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let placeholder = match initial {
            ">" => "Type a command",
            ":" => "Go to line",
            "@" => "Switch project",
            _ => "Search files by name",
        };
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(placeholder)
                .default_value(initial.to_owned())
        });
        let subscription =
            cx.subscribe_in(
                &input,
                window,
                |this, _, event: &InputEvent, _, cx| match event {
                    InputEvent::Change => this.update_items(cx),
                    InputEvent::PressEnter { .. } => this.confirm(cx),
                    _ => {}
                },
            );
        input.update(cx, |input, cx| {
            input.focus(window, cx);
            let len = input.value().len();
            input.set_selected_range(len..len, cx);
        });
        let mut this = Self {
            input,
            focus: cx.focus_handle(),
            root,
            index,
            recent,
            commands: commands(),
            fuzzy: Fuzzy::default(),
            mode: Mode::Files,
            items: Vec::new(),
            selected: 0,
            scroll: UniformListScrollHandle::new(),
            _workspace: workspace,
            _subscription: subscription,
        };
        this.update_items(cx);
        this
    }

    pub fn set_index(&mut self, index: Option<Arc<FileIndex>>, cx: &mut Context<Self>) {
        self.index = index;
        self.update_items(cx);
    }

    fn update_items(&mut self, cx: &mut Context<Self>) {
        let text = self.input.read(cx).value().to_string();
        let (mode, query) = match text.chars().next() {
            Some('>') => (Mode::Commands, text[1..].trim().to_owned()),
            Some(':') => (Mode::Line, text[1..].trim().to_owned()),
            Some('@') => (Mode::Projects, text[1..].trim().to_owned()),
            _ => (Mode::Files, text.trim().to_owned()),
        };
        self.mode = mode;
        self.items = match mode {
            Mode::Files => self.file_items(&query),
            Mode::Commands => self.command_items(&query),
            Mode::Line => match query.parse::<u32>() {
                Ok(line) if line > 0 => vec![Item {
                    icon: AnyIcon::Lucide(Lucide::CornerDownLeft),
                    title: format!("Go to line {line}").into(),
                    detail: None,
                    keys: "",
                    positions: Vec::new(),
                    target: Target::Line(line),
                }],
                _ => Vec::new(),
            },
            Mode::Projects => self.project_items(&query),
        };
        self.selected = 0;
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }

    fn file_items(&mut self, query: &str) -> Vec<Item> {
        let (Some(root), Some(index)) = (self.root.clone(), self.index.clone()) else {
            return Vec::new();
        };
        let make = |relative: &Arc<str>, positions: Vec<u32>| {
            let path = Path::new(relative.as_ref());
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let dir = path
                .parent()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
            let name_start = relative.chars().count() - name.chars().count();
            Item {
                icon: AnyIcon::Path(path.to_path_buf()),
                title: name.into(),
                detail: (!dir.is_empty()).then(|| dir.into()),
                keys: "",
                positions: positions
                    .into_iter()
                    .filter(|p| *p as usize >= name_start)
                    .map(|p| p - name_start as u32)
                    .collect(),
                target: Target::File(root.join(relative.as_ref())),
            }
        };
        if query.is_empty() {
            return index
                .files
                .iter()
                .take(60)
                .map(|f| make(f, Vec::new()))
                .collect();
        }
        self.fuzzy
            .rank(query, &index.files, 80)
            .into_iter()
            .map(|m| make(&index.files[m.index], m.positions))
            .collect()
    }

    fn command_items(&mut self, query: &str) -> Vec<Item> {
        let labels: Vec<Arc<str>> = self.commands.iter().map(|c| Arc::from(c.label)).collect();
        let order: Vec<(usize, Vec<u32>)> = if query.is_empty() {
            (0..labels.len()).map(|i| (i, Vec::new())).collect()
        } else {
            self.fuzzy
                .rank(query, &labels, 40)
                .into_iter()
                .map(|m| (m.index, m.positions))
                .collect()
        };
        order
            .into_iter()
            .map(|(ix, positions)| {
                let command = &self.commands[ix];
                Item {
                    icon: AnyIcon::Lucide(command.icon),
                    title: command.label.into(),
                    detail: None,
                    keys: command.keys,
                    positions,
                    target: Target::Command(ix),
                }
            })
            .collect()
    }

    fn project_items(&mut self, query: &str) -> Vec<Item> {
        let names: Vec<Arc<str>> = self
            .recent
            .iter()
            .map(|p| Arc::from(p.to_string_lossy().as_ref()))
            .collect();
        let mut items: Vec<Item> = if query.is_empty() {
            (0..names.len())
                .map(|i| (i, Vec::<u32>::new()))
                .collect::<Vec<_>>()
        } else {
            self.fuzzy
                .rank(query, &names, 20)
                .into_iter()
                .map(|m| (m.index, Vec::<u32>::new()))
                .collect::<Vec<_>>()
        }
        .into_iter()
        .map(|(ix, _)| {
            let path = self.recent[ix].clone();
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let detail = path.parent().map(tilde).unwrap_or_default();
            Item {
                icon: AnyIcon::Lucide(Lucide::FolderGit2),
                title: name.into(),
                detail: Some(detail.into()),
                keys: "",
                positions: Vec::new(),
                target: Target::Project(path),
            }
        })
        .collect();
        items.push(Item {
            icon: AnyIcon::Lucide(Lucide::FolderOpen),
            title: "Open folder…".into(),
            detail: None,
            keys: shortcut("⌘O", "Ctrl+O"),
            positions: Vec::new(),
            target: Target::OpenFolder,
        });
        items
    }

    /// Replace the query as if typed (used by the scripted capture tour).
    #[cfg(feature = "capture")]
    pub fn set_query(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.input
            .update(cx, |input, cx| input.set_value(text.to_owned(), window, cx));
        self.update_items(cx);
    }

    pub fn confirm(&mut self, cx: &mut Context<Self>) {
        let Some(item) = self.items.get(self.selected) else {
            return;
        };
        let event = match &item.target {
            Target::File(path) => PaletteEvent::OpenFile(path.clone()),
            Target::Project(path) => PaletteEvent::OpenProject(path.clone()),
            Target::Line(line) => PaletteEvent::GoToLine(*line),
            Target::Command(ix) => PaletteEvent::Run(self.commands[*ix].action.boxed_clone()),
            Target::OpenFolder => PaletteEvent::Run(Box::new(ws::OpenFolder)),
        };
        cx.emit(event);
    }

    fn select(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.items.is_empty() {
            return;
        }
        let n = self.items.len() as isize;
        self.selected = ((self.selected as isize + delta).rem_euclid(n)) as usize;
        self.scroll
            .scroll_to_item(self.selected, ScrollStrategy::Nearest);
        cx.notify();
    }

    fn render_item(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let item = &self.items[ix];
        let selected = ix == self.selected;
        let icon = match &item.icon {
            AnyIcon::Path(path) => super::sidebar::file_icon(path, cx).into_any_element(),
            AnyIcon::Lucide(icon) => Icon::new(*icon)
                .size(r(16.))
                .text_color(theme.muted_foreground)
                .into_any_element(),
        };
        let title = highlighted(
            &item.title,
            &item.positions,
            theme.foreground,
            theme.primary,
            &theme,
        );
        let row = h_flex()
            .id(("palette-item", ix))
            .w_full()
            .h(r(36.))
            .px(r(10.))
            .gap(r(10.))
            .rounded(r(6.))
            .cursor_pointer()
            .when(selected, |row| row.bg(theme.list_active))
            .on_mouse_move(cx.listener(move |this, _, _, cx| {
                if this.selected != ix {
                    this.selected = ix;
                    cx.notify();
                }
            }))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.selected = ix;
                this.confirm(cx);
            }))
            .child(icon)
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(r(8.))
                    .child(div().flex_none().text_size(r(14.)).child(title))
                    .when_some(item.detail.clone(), |row, detail| {
                        row.child(
                            div()
                                .truncate()
                                .text_size(r(12.5))
                                .text_color(theme.muted_foreground)
                                .child(detail),
                        )
                    }),
            )
            .when(!item.keys.is_empty(), |row| {
                row.child(
                    div()
                        .text_size(r(12.))
                        .text_color(theme.muted_foreground)
                        .child(item.keys),
                )
            })
            .when(selected && item.keys.is_empty(), |row| {
                row.child(
                    Icon::new(Lucide::CornerDownLeft)
                        .size(r(13.))
                        .text_color(theme.muted_foreground),
                )
            });
        div().w_full().px(r(6.)).child(row).into_any_element()
    }
}

fn tilde(path: &Path) -> String {
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from)
        && let Ok(rest) = path.strip_prefix(&home)
    {
        return format!("~/{}", rest.display());
    }
    path.display().to_string()
}

/// Bold the matched characters in a fuzzy result.
fn highlighted(
    text: &str,
    positions: &[u32],
    color: Hsla,
    _accent: Hsla,
    theme: &gpui_kit::component::Theme,
) -> StyledText {
    let font = gpui_kit::font(theme.font_family.clone());
    let mut runs = Vec::new();
    for (i, ch) in text.chars().enumerate() {
        let matched = positions.binary_search(&(i as u32)).is_ok();
        let mut run_font = font.clone();
        if matched {
            run_font.weight = FontWeight::BOLD;
        }
        let len = ch.len_utf8();
        match runs.last_mut() {
            Some(TextRun {
                font: last, len: l, ..
            }) if last.weight == run_font.weight => *l += len,
            _ => runs.push(TextRun {
                len,
                font: run_font,
                color,
                background_color: None,
                underline: None,
                strikethrough: None,
            }),
        }
    }
    StyledText::new(SharedString::from(text.to_owned())).with_runs(runs)
}

impl Focusable for Palette {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Palette {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let count = self.items.len();
        let empty: Option<SharedString> = (count == 0).then(|| match self.mode {
            Mode::Files if self.index.is_none() => "Indexing files…".into(),
            Mode::Files if self.root.is_none() => "Open a folder to search its files".into(),
            Mode::Line => "Type a line number".into(),
            _ => "No matches".into(),
        });
        let section: &'static str = match self.mode {
            Mode::Files if self.index.as_ref().is_some_and(|i| i.limited) => {
                "Files (first 250,000)"
            }
            Mode::Files => "Files",
            Mode::Commands => "Commands",
            Mode::Line => "Line",
            Mode::Projects => "Recent projects",
        };
        let list_height = (count.clamp(1, 10) as f32) * 36. + 8.;
        div()
            .size_full()
            .key_context(CONTEXT)
            .on_action(cx.listener(|this, _: &SelectPrevious, _, cx| this.select(-1, cx)))
            .on_action(cx.listener(|this, _: &SelectNext, _, cx| this.select(1, cx)))
            .on_action(cx.listener(|_, _: &Close, _, cx| cx.emit(PaletteEvent::Dismiss)))
            .child(
                div()
                    .id("palette-backdrop")
                    .absolute()
                    .inset_0()
                    .bg(theme.overlay)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|_, _, _, cx| cx.emit(PaletteEvent::Dismiss)),
                    ),
            )
            .child(
                v_flex()
                    .absolute()
                    .top(gpui_kit::relative(0.12))
                    .left_0()
                    .right_0()
                    .items_center()
                    .child(
                        v_flex()
                            .id("palette")
                            .w(r(640.))
                            .max_w(gpui_kit::relative(0.9))
                            .rounded(r(12.))
                            .bg(theme.popover)
                            .border_1()
                            .border_color(theme.border)
                            .shadow_2xl()
                            .overflow_hidden()
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .child(
                                h_flex()
                                    .h(r(52.))
                                    .px(r(16.))
                                    .gap(r(10.))
                                    .child(
                                        Icon::new(IconName::Search)
                                            .size(r(18.))
                                            .text_color(theme.muted_foreground),
                                    )
                                    .child(
                                        div().flex_1().text_size(r(16.)).child(
                                            Input::new(&self.input)
                                                .appearance(false)
                                                .bordered(false)
                                                .large(),
                                        ),
                                    ),
                            )
                            .child(div().h_px().bg(theme.border))
                            .child(
                                div()
                                    .px(r(18.))
                                    .pt(r(10.))
                                    .pb(r(4.))
                                    .text_size(r(12.))
                                    .font_medium()
                                    .text_color(theme.muted_foreground)
                                    .child(section),
                            )
                            .map(|panel| match empty {
                                Some(message) => panel.child(
                                    div()
                                        .px(r(18.))
                                        .py(r(14.))
                                        .text_size(r(13.5))
                                        .text_color(theme.muted_foreground)
                                        .child(message),
                                ),
                                None => panel.child(
                                    uniform_list(
                                        "palette-items",
                                        count,
                                        cx.processor(
                                            |this: &mut Self, range: Range<usize>, _, cx| {
                                                range
                                                    .map(|ix| this.render_item(ix, cx))
                                                    .collect::<Vec<_>>()
                                            },
                                        ),
                                    )
                                    .track_scroll(&self.scroll)
                                    .h(r(list_height))
                                    .pb(r(6.)),
                                ),
                            })
                            .child(div().h_px().bg(theme.border))
                            .child(
                                h_flex()
                                    .h(r(34.))
                                    .px(r(16.))
                                    .gap(r(16.))
                                    .text_size(r(12.))
                                    .text_color(theme.muted_foreground)
                                    .child(hint("↑↓", "Select"))
                                    .child(hint("↵", "Open"))
                                    .child(hint("esc", "Close"))
                                    .child(div().flex_1())
                                    .child(hint(">", "Commands"))
                                    .child(hint(":", "Line"))
                                    .child(hint("@", "Projects")),
                            ),
                    ),
            )
    }
}

fn hint(keys: &'static str, label: &'static str) -> impl IntoElement {
    h_flex()
        .gap(r(5.))
        .child(div().font_weight(FontWeight::SEMIBOLD).child(keys))
        .child(label)
}
