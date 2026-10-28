//! The integrated terminal: a native PTY driven by alacritty_terminal, painted
//! directly on the GPU one styled run at a time.
use std::{
    borrow::Cow,
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use alacritty_terminal::{
    event::{Event as TermEvent, EventListener, Notify, OnResize, WindowSize},
    event_loop::{EventLoop, Msg, Notifier},
    grid::{Dimensions, Scroll},
    index::{Column, Line, Point as GridPoint, Side},
    selection::{Selection, SelectionType},
    sync::FairMutex,
    term::{Config, Term, TermMode, cell::Flags, point_to_viewport, viewport_to_point},
    tty,
    vte::ansi::{Color as AnsiColor, CursorShape, NamedColor, Rgb},
};
use futures::{StreamExt, channel::mpsc};
use gpui_kit::component::{ActiveTheme, IconName, StyledExt as _, Theme, h_flex, v_flex};
use gpui_kit::{
    App, AppContext, Bounds, ClipboardItem, Context, DragMoveEvent, Element, ElementId, Empty,
    Entity, FocusHandle, Focusable, Font, FontStyle, FontWeight, GlobalElementId, Hsla,
    InspectorElementId, InteractiveElement, IntoElement, KeyBinding, KeyDownEvent, LayoutId,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, NoAction, ParentElement, Pixels,
    Point, Render, ScrollDelta, ScrollEvent, SharedString, StatefulInteractiveElement, Style,
    Styled, Task, TextAlign, TextRun, Window, actions, div, fill, outline, point,
    prelude::FluentBuilder, px, relative, size,
};

use super::{icon_button, is_mac, r};
use crate::settings::Settings;

actions!(terminal, [Copy, Paste, Clear, SelectAll]);

const CONTEXT: &str = "Terminal";
const SCROLLBACK: usize = 10_000;

pub fn init(cx: &mut App) {
    let mut bindings = vec![
        // Keys the shell needs more than the workspace does.
        KeyBinding::new("ctrl-g", NoAction, Some(CONTEXT)),
        KeyBinding::new("escape", NoAction, Some(CONTEXT)),
    ];
    if is_mac() {
        bindings.extend([
            KeyBinding::new("cmd-c", Copy, Some(CONTEXT)),
            KeyBinding::new("cmd-v", Paste, Some(CONTEXT)),
            KeyBinding::new("cmd-k", Clear, Some(CONTEXT)),
            KeyBinding::new("cmd-a", SelectAll, Some(CONTEXT)),
        ]);
    } else {
        bindings.extend([
            KeyBinding::new("ctrl-shift-c", Copy, Some(CONTEXT)),
            KeyBinding::new("ctrl-shift-v", Paste, Some(CONTEXT)),
            KeyBinding::new("ctrl-shift-k", Clear, Some(CONTEXT)),
        ]);
        for key in [
            "ctrl-p",
            "ctrl-k",
            "ctrl-b",
            "ctrl-w",
            "ctrl-s",
            "ctrl-o",
            "ctrl-q",
            "ctrl-shift-f",
        ] {
            bindings.push(KeyBinding::new(key, NoAction, Some(CONTEXT)));
        }
    }
    cx.bind_keys(bindings);
}

// ---- Panel ------------------------------------------------------------------

#[derive(Clone)]
struct PanelResize;

impl Render for PanelResize {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

pub struct TerminalPanel {
    pub open: bool,
    height: f32,
    sessions: Vec<Entity<TerminalView>>,
    active: usize,
    cwd: Option<PathBuf>,
    next_id: u64,
}

impl TerminalPanel {
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            open: false,
            height: 260.,
            sessions: Vec::new(),
            active: 0,
            cwd: None,
            next_id: 1,
        }
    }

    pub fn set_cwd(&mut self, cwd: PathBuf) {
        self.cwd = Some(cwd);
    }

    pub fn is_focused(&self, window: &Window, cx: &App) -> bool {
        self.open
            && self
                .sessions
                .get(self.active)
                .is_some_and(|s| s.read(cx).focus.contains_focused(window, cx))
    }

    pub fn show(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open = true;
        if self.sessions.is_empty() {
            self.new_session(window, cx);
        } else if let Some(session) = self.sessions.get(self.active) {
            {
                let handle = session.read(cx).focus.clone();
                handle.focus(window, cx)
            };
        }
        cx.notify();
    }

    pub fn toggle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.open && self.is_focused(window, cx) {
            self.open = false;
            cx.notify();
        } else {
            self.show(window, cx);
        }
    }

    pub fn new_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let id = self.next_id;
        self.next_id += 1;
        let cwd = self.cwd.clone();
        let view = cx.new(|cx| TerminalView::new(id, cwd, window, cx));
        cx.observe(&view, |this, view, cx| {
            if view.read(cx).exited {
                this.remove(&view, cx);
            }
            cx.notify();
        })
        .detach();
        {
            let handle = view.read(cx).focus.clone();
            handle.focus(window, cx)
        };
        self.sessions.push(view);
        self.active = self.sessions.len() - 1;
        self.open = true;
        cx.notify();
    }

    fn remove(&mut self, view: &Entity<TerminalView>, cx: &mut Context<Self>) {
        if let Some(ix) = self.sessions.iter().position(|s| s == view) {
            self.sessions.remove(ix);
            if self.active >= self.sessions.len() {
                self.active = self.sessions.len().saturating_sub(1);
            }
            if self.sessions.is_empty() {
                self.open = false;
            }
            cx.notify();
        }
    }

    /// Type into the active session (used by the scripted capture tour).
    #[cfg(feature = "capture")]
    pub fn send_text(&mut self, text: &str, cx: &mut Context<Self>) {
        if let Some(view) = self.sessions.get(self.active).cloned() {
            let bytes = text.as_bytes().to_vec();
            view.update(cx, |view, cx| {
                view.input(bytes);
                cx.notify();
            });
        }
    }

    pub fn close_active(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(view) = self.sessions.get(self.active).cloned() {
            view.update(cx, |view, _| view.shutdown());
            self.remove(&view, cx);
        }
    }
}

impl Render for TerminalPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let hover = theme.list_hover;
        let tabs = self.sessions.iter().enumerate().map(|(ix, view)| {
            let active = ix == self.active;
            let title = view.read(cx).title.clone();
            h_flex()
                .id(("term-tab", ix))
                .group("term-tab")
                .h(r(24.))
                .pl(r(8.))
                .pr(r(2.))
                .gap(r(4.))
                .rounded(r(5.))
                .text_size(r(12.5))
                .cursor_pointer()
                .when(active, |t| {
                    t.bg(theme.list_active).text_color(theme.foreground)
                })
                .when(!active, |t| {
                    t.text_color(theme.muted_foreground)
                        .hover(move |s| s.bg(hover))
                })
                .child(div().max_w(r(160.)).truncate().child(title))
                .child(
                    div()
                        .id(("term-close", ix))
                        .size(r(16.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(r(4.))
                        .invisible()
                        .group_hover("term-tab", |s| s.visible())
                        .hover(move |s| s.bg(hover))
                        .child(gpui_kit::component::Icon::new(IconName::Close).size(r(11.)))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.active = ix;
                            this.close_active(window, cx);
                        })),
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.active = ix;
                    if let Some(view) = this.sessions.get(ix) {
                        {
                            let handle = view.read(cx).focus.clone();
                            handle.focus(window, cx)
                        };
                    }
                    cx.notify();
                }))
        });
        let height = self.height;
        v_flex()
            .id("terminal-panel")
            .relative()
            .flex_none()
            .h(px(height))
            .border_t_1()
            .border_color(theme.border)
            .bg(theme.background)
            .child(
                div()
                    .id("terminal-resize")
                    .absolute()
                    .top(px(-3.))
                    .left_0()
                    .right_0()
                    .h(px(6.))
                    .cursor_row_resize()
                    .on_drag(PanelResize, |_, _, _, cx| cx.new(|_| PanelResize)),
            )
            .on_drag_move(
                cx.listener(|this, event: &DragMoveEvent<PanelResize>, _, cx| {
                    let bottom: f32 = event.bounds.bottom().into();
                    let y: f32 = event.event.position.y.into();
                    this.height = (bottom - y).clamp(120., 900.);
                    cx.notify();
                }),
            )
            .child(
                h_flex()
                    .flex_none()
                    .h(r(34.))
                    .px(r(10.))
                    .gap(r(2.))
                    .child(
                        div()
                            .pr(r(8.))
                            .text_size(r(12.))
                            .font_medium()
                            .text_color(theme.muted_foreground)
                            .child("Terminal"),
                    )
                    .children(tabs)
                    .child(
                        icon_button("term-new", IconName::Plus, cx)
                            .size(r(22.))
                            .on_click(
                                cx.listener(|this, _, window, cx| this.new_session(window, cx)),
                            ),
                    )
                    .child(div().flex_1())
                    .child(
                        icon_button("term-hide", IconName::ChevronDown, cx)
                            .size(r(22.))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.open = false;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .when_some(self.sessions.get(self.active).cloned(), |d, view| {
                        d.child(view)
                    }),
            )
    }
}

// ---- Session ----------------------------------------------------------------

#[derive(Clone)]
struct Listener(mpsc::UnboundedSender<TermEvent>);

impl EventListener for Listener {
    fn send_event(&self, event: TermEvent) {
        let _ = self.0.unbounded_send(event);
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct GridSize {
    columns: usize,
    lines: usize,
    cell_width: Pixels,
    line_height: Pixels,
}

impl Dimensions for GridSize {
    fn total_lines(&self) -> usize {
        self.lines
    }
    fn screen_lines(&self) -> usize {
        self.lines
    }
    fn columns(&self) -> usize {
        self.columns
    }
}

impl GridSize {
    fn window_size(&self) -> WindowSize {
        WindowSize {
            num_lines: self.lines as u16,
            num_cols: self.columns as u16,
            cell_width: f32::from(self.cell_width) as u16,
            cell_height: f32::from(self.line_height) as u16,
        }
    }
}

pub struct TerminalView {
    term: Option<Arc<FairMutex<Term<Listener>>>>,
    notifier: Option<Notifier>,
    /// Everything needed to start the shell, held until the first layout
    /// knows the real grid size, so the prompt never draws at a wrong width.
    pending: Option<Spawn>,
    pub focus: FocusHandle,
    pub title: SharedString,
    /// The program name, shown when the shell's own title is just `user@host:path`.
    program: SharedString,
    pub exited: bool,
    error: Option<SharedString>,
    size: GridSize,
    origin: Point<Pixels>,
    scroll_remainder: f32,
    selecting: bool,
    _events: Task<()>,
}

struct Spawn {
    id: u64,
    options: tty::Options,
    listener: Listener,
}

/// The user's shell as a login shell, started directly rather than through
/// `/usr/bin/login`, which would print a "Last login" banner on macOS.
fn shell_command(configured: &str) -> Option<(String, Vec<String>)> {
    if !configured.is_empty() {
        let mut parts = configured.split_whitespace().map(str::to_owned);
        let program = parts.next()?;
        return Some((program, parts.collect()));
    }
    if cfg!(windows) {
        return None;
    }
    let shell = std::env::var("SHELL")
        .ok()
        .filter(|s| !s.trim().is_empty() && Path::new(s).exists())
        .unwrap_or_else(|| {
            if cfg!(target_os = "macos") {
                "/bin/zsh".into()
            } else {
                "/bin/bash".into()
            }
        });
    Some((shell, vec!["-l".to_owned()]))
}

/// Shell prompts often title the terminal `user@host:~/path`; that is noise in a tab.
fn clean_title(title: &str, program: &str) -> SharedString {
    let title = title.trim();
    let user_host_path = title
        .split_once(':')
        .is_some_and(|(left, _)| left.contains('@') && !left.contains(' '));
    if title.is_empty() || user_host_path {
        program.to_owned().into()
    } else {
        title.to_owned().into()
    }
}

impl TerminalView {
    fn new(id: u64, cwd: Option<PathBuf>, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let size = GridSize {
            columns: 100,
            lines: 24,
            cell_width: px(8.),
            line_height: px(17.),
        };
        let (tx, mut rx) = mpsc::unbounded();
        let configured = Settings::get(cx).terminal_shell.trim().to_owned();
        let shell = shell_command(&configured);
        let program = shell
            .as_ref()
            .map(|(program, _)| {
                program
                    .rsplit(['/', '\\'])
                    .next()
                    .unwrap_or("shell")
                    .to_owned()
            })
            .unwrap_or_else(|| {
                if cfg!(windows) {
                    "PowerShell".into()
                } else {
                    "shell".into()
                }
            });
        let shell = shell.map(|(program, args)| tty::Shell::new(program, args));
        let mut env = HashMap::new();
        env.insert("TERM".to_owned(), "xterm-256color".to_owned());
        env.insert("COLORTERM".to_owned(), "truecolor".to_owned());
        env.insert("TERM_PROGRAM".to_owned(), crate::brand::NAME.to_owned());
        env.insert(
            "TERM_PROGRAM_VERSION".to_owned(),
            crate::brand::VERSION.to_owned(),
        );
        let options = tty::Options {
            shell,
            working_directory: cwd.filter(|p| p.is_dir()),
            drain_on_exit: true,
            env,
            #[cfg(target_os = "windows")]
            escape_args: true,
        };
        let events = cx.spawn(async move |this, cx| {
            while let Some(event) = rx.next().await {
                // Coalesce bursts of output into one repaint.
                let mut events = vec![event];
                while let Ok(more) = rx.try_recv() {
                    events.push(more);
                }
                let alive = this
                    .update(cx, |this, cx| {
                        for event in events {
                            this.handle_event(event, cx);
                        }
                        cx.notify();
                    })
                    .is_ok();
                if !alive {
                    break;
                }
            }
        });
        Self {
            term: None,
            notifier: None,
            pending: Some(Spawn {
                id,
                options,
                listener: Listener(tx),
            }),
            focus: cx.focus_handle(),
            title: program.clone().into(),
            program: program.into(),
            exited: false,
            error: None,
            size,
            origin: Point::default(),
            scroll_remainder: 0.,
            selecting: false,
            _events: events,
        }
    }

    /// Start the shell at `size`. Called once, from the first layout.
    fn start(&mut self, size: GridSize) {
        let Some(spawn) = self.pending.take() else {
            return;
        };
        self.size = size;
        let config = Config {
            scrolling_history: SCROLLBACK,
            ..Config::default()
        };
        match tty::new(&spawn.options, size.window_size(), spawn.id) {
            Ok(pty) => {
                let term = Arc::new(FairMutex::new(Term::new(
                    config,
                    &size,
                    spawn.listener.clone(),
                )));
                match EventLoop::new(term.clone(), spawn.listener, pty, true, false) {
                    Ok(event_loop) => {
                        self.notifier = Some(Notifier(event_loop.channel()));
                        let _ = event_loop.spawn();
                        self.term = Some(term);
                    }
                    Err(e) => self.error = Some(format!("Cannot start the terminal: {e}").into()),
                }
            }
            Err(e) => self.error = Some(format!("Cannot start a shell: {e}").into()),
        }
    }

    fn handle_event(&mut self, event: TermEvent, cx: &mut Context<Self>) {
        match event {
            TermEvent::Title(title) => self.title = clean_title(&title, &self.program),
            TermEvent::ResetTitle => self.title = self.program.clone(),
            TermEvent::PtyWrite(text) => self.write(text.into_bytes()),
            TermEvent::ClipboardStore(_, text) => {
                cx.write_to_clipboard(ClipboardItem::new_string(text))
            }
            TermEvent::ClipboardLoad(_, format) => {
                let text = cx
                    .read_from_clipboard()
                    .and_then(|c| c.text())
                    .unwrap_or_default();
                self.write(format(&text).into_bytes());
            }
            TermEvent::TextAreaSizeRequest(format) => {
                self.write(format(self.size.window_size()).into_bytes())
            }
            TermEvent::ColorRequest(index, format) => {
                let theme = cx.theme().clone();
                let color = default_color(index, &theme);
                let rgba = color.to_rgb();
                let rgb = Rgb {
                    r: (rgba.r * 255.) as u8,
                    g: (rgba.g * 255.) as u8,
                    b: (rgba.b * 255.) as u8,
                };
                self.write(format(rgb).into_bytes());
            }
            TermEvent::Exit | TermEvent::ChildExit(_) => {
                self.exited = true;
            }
            _ => {}
        }
    }

    pub fn shutdown(&mut self) {
        if let Some(notifier) = &self.notifier {
            let _ = notifier.0.send(Msg::Shutdown);
        }
    }

    fn write(&self, bytes: impl Into<Cow<'static, [u8]>>) {
        if let Some(notifier) = &self.notifier {
            notifier.notify(bytes);
        }
    }

    fn input(&mut self, bytes: impl Into<Cow<'static, [u8]>>) {
        if let Some(term) = &self.term {
            let mut term = term.lock();
            term.scroll_display(Scroll::Bottom);
            term.selection = None;
        }
        self.write(bytes);
    }

    fn resize(&mut self, size: GridSize) {
        if self.pending.is_some() {
            self.start(size);
            return;
        }
        if size == self.size {
            return;
        }
        self.size = size;
        if let Some(term) = &self.term {
            term.lock().resize(size);
        }
        if let Some(notifier) = &mut self.notifier {
            notifier.on_resize(size.window_size());
        }
    }

    fn mode(&self) -> TermMode {
        self.term
            .as_ref()
            .map(|t| *t.lock().mode())
            .unwrap_or(TermMode::NONE)
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let mode = self.mode();
        if let Some(bytes) = encode_key(&event.keystroke, mode) {
            self.input(bytes);
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn grid_point(&self, position: Point<Pixels>) -> (GridPoint, Side) {
        let x = f32::from(position.x - self.origin.x).max(0.);
        let y = f32::from(position.y - self.origin.y).max(0.);
        let cell = f32::from(self.size.cell_width).max(1.);
        let line_height = f32::from(self.size.line_height).max(1.);
        let column = ((x / cell) as usize).min(self.size.columns.saturating_sub(1));
        let line = ((y / line_height) as usize).min(self.size.lines.saturating_sub(1));
        let side = if x % cell > cell / 2. {
            Side::Right
        } else {
            Side::Left
        };
        let offset = self
            .term
            .as_ref()
            .map(|t| t.lock().grid().display_offset())
            .unwrap_or(0);
        (
            viewport_to_point(offset, GridPoint::new(line, Column(column))),
            side,
        )
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus.focus(window, cx);
        let (point, side) = self.grid_point(event.position);
        let kind = match event.click_count {
            2 => SelectionType::Semantic,
            3 => SelectionType::Lines,
            _ => SelectionType::Simple,
        };
        if let Some(term) = &self.term {
            term.lock().selection = Some(Selection::new(kind, point, side));
        }
        self.selecting = true;
        cx.notify();
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selecting || event.pressed_button != Some(MouseButton::Left) {
            return;
        }
        let (point, side) = self.grid_point(event.position);
        if let Some(term) = &self.term
            && let Some(selection) = term.lock().selection.as_mut()
        {
            selection.update(point, side);
        }
        cx.notify();
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.selecting = false;
        if let Some(term) = &self.term {
            let mut term = term.lock();
            if term.selection.as_ref().is_some_and(|s| s.is_empty()) {
                term.selection = None;
            }
        }
        cx.notify();
    }

    fn on_scroll(&mut self, event: &ScrollEvent, window: &mut Window, cx: &mut Context<Self>) {
        let line_height = f32::from(self.size.line_height).max(1.);
        let lines = match event.delta {
            ScrollDelta::Lines(delta) => delta.y,
            ScrollDelta::Pixels(delta) => f32::from(delta.y) / line_height,
        } + self.scroll_remainder;
        let whole = lines.trunc() as i32;
        self.scroll_remainder = lines.fract();
        if whole == 0 {
            return;
        }
        let mode = self.mode();
        if mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL)
            && !event.modifiers.shift
        {
            // Full-screen apps (less, vim) expect arrow keys for the wheel.
            let key: &[u8] = if whole > 0 {
                if mode.contains(TermMode::APP_CURSOR) {
                    b"\x1bOA"
                } else {
                    b"\x1b[A"
                }
            } else if mode.contains(TermMode::APP_CURSOR) {
                b"\x1bOB"
            } else {
                b"\x1b[B"
            };
            let bytes: Vec<u8> = key.repeat(whole.unsigned_abs() as usize);
            self.write(bytes);
        } else if let Some(term) = &self.term {
            term.lock().scroll_display(Scroll::Delta(whole));
        }
        let _ = window;
        cx.notify();
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = self
            .term
            .as_ref()
            .and_then(|t| t.lock().selection_to_string())
        {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) else {
            return;
        };
        let bracketed = self.mode().contains(TermMode::BRACKETED_PASTE);
        let text = if bracketed {
            // Strip any end marker so pasted text cannot break out of the paste.
            format!("\x1b[200~{}\x1b[201~", text.replace("\x1b[201~", ""))
        } else {
            text.replace("\r\n", "\r").replace('\n', "\r")
        };
        self.input(text.into_bytes());
        cx.notify();
    }

    fn clear(&mut self, _: &Clear, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(term) = &self.term {
            let mut term = term.lock();
            term.grid_mut().clear_history();
        }
        // Form feed asks the shell to redraw its prompt at the top.
        self.input(b"\x0c".to_vec());
        cx.notify();
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(term) = &self.term {
            let mut term = term.lock();
            let start = GridPoint::new(Line(-(term.grid().history_size() as i32)), Column(0));
            let end = GridPoint::new(
                Line(term.screen_lines() as i32 - 1),
                Column(term.columns() - 1),
            );
            let mut selection = Selection::new(SelectionType::Simple, start, Side::Left);
            selection.update(end, Side::Right);
            term.selection = Some(selection);
        }
        cx.notify();
    }
}

impl Drop for TerminalView {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl Focusable for TerminalView {
    fn focus_handle(&self, _: &App) -> gpui_kit::FocusHandle {
        self.focus.clone()
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.focus.is_focused(window);
        div()
            .id("terminal")
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .size_full()
            .pl(r(12.))
            .pr(r(4.))
            .pb(r(6.))
            .cursor_text()
            .on_key_down(cx.listener(Self::on_key_down))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_scroll(cx.listener(Self::on_scroll))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::clear))
            .on_action(cx.listener(Self::select_all))
            .map(|d| match &self.error {
                Some(error) => d
                    .text_size(r(13.))
                    .text_color(cx.theme().red)
                    .child(error.clone())
                    .into_any_element(),
                None => d
                    .child(TerminalElement {
                        view: cx.entity(),
                        focused,
                    })
                    .into_any_element(),
            })
    }
}

// ---- Painting ---------------------------------------------------------------

struct TerminalElement {
    view: Entity<TerminalView>,
    focused: bool,
}

struct Run {
    line: usize,
    column: usize,
    text: String,
    color: Hsla,
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
}

struct Prepared {
    font: Font,
    font_size: Pixels,
    cell_width: Pixels,
    line_height: Pixels,
    backgrounds: Vec<(usize, usize, usize, Hsla)>,
    selections: Vec<(usize, usize, usize)>,
    runs: Vec<Run>,
    cursor: Option<(usize, usize, CursorShape, Hsla)>,
    selection_color: Hsla,
}

impl IntoElement for TerminalElement {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for TerminalElement {
    type RequestLayoutState = ();
    type PrepaintState = Option<Prepared>;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Prepared> {
        let theme = Theme::global(cx).clone();
        let settings = Settings::get(cx);
        let font_size = px(settings.terminal_font_size * settings.zoom);
        let font = gpui_kit::font(theme.mono_font_family.clone());
        let text_system = window.text_system().clone();
        let font_id = text_system.resolve_font(&font);
        let cell_width = text_system
            .advance(font_id, font_size, 'm')
            .map(|s| s.width)
            .unwrap_or(font_size * 0.6);
        let line_height = (font_size * 1.35).round();
        let columns =
            ((f32::from(bounds.size.width) / f32::from(cell_width)).floor() as usize).max(2);
        let lines =
            ((f32::from(bounds.size.height) / f32::from(line_height)).floor() as usize).max(1);
        let size = GridSize {
            columns,
            lines,
            cell_width,
            line_height,
        };
        let focused = self.focused;
        self.view.update(cx, |view, _| {
            view.origin = bounds.origin;
            view.resize(size);
            let term = view.term.clone()?;
            let term = term.lock();
            let content = term.renderable_content();
            let colors = content.colors;
            let offset = content.display_offset;
            let default_bg = theme.background;
            let mut backgrounds = Vec::new();
            let mut runs: Vec<Run> = Vec::new();
            let mut row_bg: Option<(usize, usize, usize, Hsla)> = None;
            for indexed in content.display_iter {
                let Some(at) = point_to_viewport(offset, indexed.point) else {
                    continue;
                };
                let (line, column) = (at.line, at.column.0);
                let cell = &indexed.cell;
                let flags = cell.flags;
                let mut fg = resolve(cell.fg, colors, &theme);
                let mut bg = resolve(cell.bg, colors, &theme);
                if flags.contains(Flags::INVERSE) {
                    std::mem::swap(&mut fg, &mut bg);
                }
                if flags.contains(Flags::DIM) {
                    fg = fg.opacity(0.66);
                }
                // Backgrounds: merge horizontal spans of the same color.
                if bg != default_bg && !matches!(cell.bg, AnsiColor::Named(NamedColor::Background))
                    || flags.contains(Flags::INVERSE)
                {
                    match &mut row_bg {
                        Some((l, _, end, color))
                            if *l == line && *end == column && *color == bg =>
                        {
                            *end = column + 1
                        }
                        _ => {
                            if let Some(span) = row_bg.take() {
                                backgrounds.push(span);
                            }
                            row_bg = Some((line, column, column + 1, bg));
                        }
                    }
                }
                if flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                    continue;
                }
                if cell.c == ' ' && !flags.intersects(Flags::UNDERLINE | Flags::STRIKEOUT) {
                    continue;
                }
                if flags.contains(Flags::HIDDEN) {
                    continue;
                }
                let bold = flags.contains(Flags::BOLD);
                let italic = flags.contains(Flags::ITALIC);
                let underline = flags.intersects(Flags::ALL_UNDERLINES);
                let strike = flags.contains(Flags::STRIKEOUT);
                let wide = flags.contains(Flags::WIDE_CHAR);
                let extends = runs.last().is_some_and(|last: &Run| {
                    last.line == line
                        && last.column + last.text.chars().count() == column
                        && last.color == fg
                        && last.bold == bold
                        && last.italic == italic
                        && last.underline == underline
                        && last.strike == strike
                        && !last.text.ends_with('\u{0}')
                });
                if extends && !wide {
                    runs.last_mut().unwrap().text.push(cell.c);
                } else {
                    runs.push(Run {
                        line,
                        column,
                        text: cell.c.to_string(),
                        color: fg,
                        bold,
                        italic,
                        underline,
                        strike,
                    });
                    if wide {
                        // A wide glyph ends its run so the next cell restarts at its own column.
                        runs.push(Run {
                            line,
                            column: column + 2,
                            text: String::new(),
                            color: fg,
                            bold,
                            italic,
                            underline,
                            strike,
                        });
                    }
                }
                for zero_width in cell.zerowidth().into_iter().flatten() {
                    if let Some(last) = runs.last_mut() {
                        last.text.push(*zero_width);
                    }
                }
            }
            if let Some(span) = row_bg.take() {
                backgrounds.push(span);
            }
            runs.retain(|run| !run.text.is_empty());
            let mut selections = Vec::new();
            if let Some(range) = content.selection {
                let start = point_to_viewport(offset, range.start);
                let end = point_to_viewport(offset, range.end);
                let first = start.map_or(0, |p| p.line);
                let last = end.map_or(0, |p| p.line).min(lines.saturating_sub(1));
                if let Some(end) = end {
                    for line in first..=last {
                        let from = match start {
                            Some(start) if start.line == line => start.column.0,
                            _ => 0,
                        };
                        let to = if end.line == line {
                            end.column.0 + 1
                        } else {
                            columns
                        };
                        if range.is_block {
                            selections.push((line, range.start.column.0, range.end.column.0 + 1));
                        } else {
                            selections.push((line, from, to));
                        }
                    }
                }
            }
            let cursor = (content.mode.contains(TermMode::SHOW_CURSOR) && offset == 0)
                .then(|| {
                    let at = point_to_viewport(offset, content.cursor.point)?;
                    let shape = if focused {
                        content.cursor.shape
                    } else {
                        CursorShape::HollowBlock
                    };
                    Some((at.line, at.column.0, shape, theme.foreground))
                })
                .flatten();
            Some(Prepared {
                font: font.clone(),
                font_size,
                cell_width,
                line_height,
                backgrounds,
                selections,
                runs,
                cursor,
                selection_color: theme.selection,
            })
        })
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        prepared: &mut Option<Prepared>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(p) = prepared.take() else { return };
        let origin = bounds.origin;
        let at = |line: usize, column: usize| {
            point(
                origin.x + p.cell_width * column as f32,
                origin.y + p.line_height * line as f32,
            )
        };
        let theme_bg = cx.theme().background;
        window.paint_layer(bounds, |window| {
            for (line, start, end, color) in &p.backgrounds {
                window.paint_quad(fill(
                    Bounds::new(
                        at(*line, *start),
                        size(p.cell_width * (end - start) as f32, p.line_height),
                    ),
                    *color,
                ));
            }
            for (line, start, end) in &p.selections {
                if end > start {
                    window.paint_quad(fill(
                        Bounds::new(
                            at(*line, *start),
                            size(p.cell_width * (end - start) as f32, p.line_height),
                        ),
                        p.selection_color,
                    ));
                }
            }
            let mut cursor_text = None;
            if let Some((line, column, shape, color)) = p.cursor {
                let origin = at(line, column);
                match shape {
                    CursorShape::Beam => window.paint_quad(fill(
                        Bounds::new(origin, size(px(2.), p.line_height)),
                        color,
                    )),
                    CursorShape::Underline => window.paint_quad(fill(
                        Bounds::new(
                            point(origin.x, origin.y + p.line_height - px(2.)),
                            size(p.cell_width, px(2.)),
                        ),
                        color,
                    )),
                    CursorShape::HollowBlock => window.paint_quad(outline(
                        Bounds::new(origin, size(p.cell_width, p.line_height)),
                        color,
                        gpui_kit::BorderStyle::Solid,
                    )),
                    CursorShape::Hidden => {}
                    CursorShape::Block => {
                        window.paint_quad(fill(
                            Bounds::new(origin, size(p.cell_width, p.line_height)),
                            color,
                        ));
                        cursor_text = Some((line, column));
                    }
                }
            }
            for run in &p.runs {
                let mut font = p.font.clone();
                if run.bold {
                    font.weight = FontWeight::BOLD;
                }
                if run.italic {
                    font.style = FontStyle::Italic;
                }
                let mut color = run.color;
                if let Some((line, column)) = cursor_text
                    && run.line == line
                    && run.column == column
                    && run.text.chars().count() == 1
                {
                    color = theme_bg;
                }
                let text_run = TextRun {
                    len: run.text.len(),
                    font,
                    color,
                    background_color: None,
                    underline: run.underline.then(|| gpui_kit::UnderlineStyle {
                        thickness: px(1.),
                        color: Some(run.color),
                        wavy: false,
                    }),
                    strikethrough: run.strike.then(|| gpui_kit::StrikethroughStyle {
                        thickness: px(1.),
                        color: Some(run.color),
                    }),
                };
                let shaped = window.text_system().shape_line(
                    SharedString::from(run.text.clone()),
                    p.font_size,
                    &[text_run],
                    Some(p.cell_width),
                );
                let _ = shaped.paint(
                    at(run.line, run.column),
                    p.line_height,
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                );
            }
        });
    }
}

fn resolve(
    color: AnsiColor,
    colors: &alacritty_terminal::term::color::Colors,
    theme: &Theme,
) -> Hsla {
    match color {
        AnsiColor::Spec(rgb) => rgb_to_hsla(rgb),
        AnsiColor::Named(named) => colors[named]
            .map(rgb_to_hsla)
            .unwrap_or_else(|| default_color(named as usize, theme)),
        AnsiColor::Indexed(index) => colors[index as usize]
            .map(rgb_to_hsla)
            .unwrap_or_else(|| default_color(index as usize, theme)),
    }
}

fn rgb_to_hsla(rgb: Rgb) -> Hsla {
    gpui_kit::Rgba {
        r: rgb.r as f32 / 255.,
        g: rgb.g as f32 / 255.,
        b: rgb.b as f32 / 255.,
        a: 1.,
    }
    .into()
}

fn hex(value: u32) -> Hsla {
    gpui_kit::rgb(value).into()
}

/// The 256-color palette, with the first 16 tuned to the active theme.
fn default_color(index: usize, theme: &Theme) -> Hsla {
    let dark = theme.is_dark();
    let ansi: [u32; 16] = if dark {
        [
            0x3a3a3a, 0xe06c6a, 0x6db38a, 0xd6a85a, 0x6f9be0, 0xb48ee0, 0x5fb3b3, 0xd4d4d4,
            0x6e6e6e, 0xf08a87, 0x86c99d, 0xe6bd73, 0x8db2ef, 0xc8a6ec, 0x7ecaca, 0xffffff,
        ]
    } else {
        [
            0x37352f, 0xd44c47, 0x448361, 0xb88a1e, 0x337ea9, 0x9065b0, 0x2b8a8a, 0xb4b3af,
            0x787774, 0xe03e3e, 0x2f9e62, 0xcb912f, 0x2383e2, 0xa66ac8, 0x3aa3a3, 0x37352f,
        ]
    };
    match index {
        0..16 => hex(ansi[index]),
        16..232 => {
            let i = index - 16;
            let level = |v: usize| if v == 0 { 0 } else { 55 + v as u32 * 40 };
            hex((level(i / 36) << 16) | (level((i / 6) % 6) << 8) | level(i % 6))
        }
        232..256 => {
            let v = 8 + (index as u32 - 232) * 10;
            hex((v << 16) | (v << 8) | v)
        }
        256 => theme.foreground, // Foreground
        257 => theme.background, // Background
        258 => theme.foreground, // Cursor
        259..=266 => default_color(index - 259, theme).opacity(0.66), // Dim colors
        267 => theme.foreground, // BrightForeground
        268 => theme.foreground.opacity(0.66), // DimForeground
        _ => theme.foreground,
    }
}

/// Translate a keystroke into the bytes a terminal application expects.
fn encode_key(keystroke: &gpui_kit::Keystroke, mode: TermMode) -> Option<Vec<u8>> {
    let m = &keystroke.modifiers;
    if m.platform {
        return None;
    }
    let modifier_code = 1 + (m.shift as u8) + 2 * (m.alt as u8) + 4 * (m.control as u8);
    let app_cursor = mode.contains(TermMode::APP_CURSOR);
    let csi = |final_byte: char| -> Vec<u8> {
        if modifier_code > 1 {
            format!("\x1b[1;{modifier_code}{final_byte}").into_bytes()
        } else if app_cursor {
            format!("\x1bO{final_byte}").into_bytes()
        } else {
            format!("\x1b[{final_byte}").into_bytes()
        }
    };
    let tilde = |code: u8| -> Vec<u8> {
        if modifier_code > 1 {
            format!("\x1b[{code};{modifier_code}~").into_bytes()
        } else {
            format!("\x1b[{code}~").into_bytes()
        }
    };
    let key = keystroke.key.as_str();
    let bytes = match key {
        "enter" => {
            if m.alt {
                b"\x1b\r".to_vec()
            } else {
                b"\r".to_vec()
            }
        }
        "backspace" => {
            if m.control {
                b"\x08".to_vec()
            } else if m.alt {
                b"\x1b\x7f".to_vec()
            } else {
                b"\x7f".to_vec()
            }
        }
        "tab" => {
            if m.shift {
                b"\x1b[Z".to_vec()
            } else {
                b"\t".to_vec()
            }
        }
        "escape" => b"\x1b".to_vec(),
        "up" => csi('A'),
        "down" => csi('B'),
        "right" => {
            if m.alt && !m.control && is_mac() {
                b"\x1bf".to_vec()
            } else {
                csi('C')
            }
        }
        "left" => {
            if m.alt && !m.control && is_mac() {
                b"\x1bb".to_vec()
            } else {
                csi('D')
            }
        }
        "home" => csi('H'),
        "end" => csi('F'),
        "pageup" => tilde(5),
        "pagedown" => tilde(6),
        "delete" => tilde(3),
        "insert" => tilde(2),
        "f1" => b"\x1bOP".to_vec(),
        "f2" => b"\x1bOQ".to_vec(),
        "f3" => b"\x1bOR".to_vec(),
        "f4" => b"\x1bOS".to_vec(),
        "f5" => tilde(15),
        "f6" => tilde(17),
        "f7" => tilde(18),
        "f8" => tilde(19),
        "f9" => tilde(20),
        "f10" => tilde(21),
        "f11" => tilde(23),
        "f12" => tilde(24),
        "space" if m.control => vec![0],
        _ if m.control && !m.alt => {
            let mut chars = key.chars();
            let c = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            match c {
                'a'..='z' => vec![(c as u8) & 0x1f],
                '@' | '2' => vec![0],
                '[' | '3' => vec![0x1b],
                '\\' | '4' => vec![0x1c],
                ']' | '5' => vec![0x1d],
                '^' | '6' => vec![0x1e],
                '_' | '-' | '7' => vec![0x1f],
                '/' => vec![0x1f],
                '8' | '?' => vec![0x7f],
                _ => return None,
            }
        }
        _ if m.alt && key.len() == 1 && key.is_ascii() => {
            // Option as Meta for word motions in shells.
            let c = if m.shift {
                key.to_ascii_uppercase()
            } else {
                key.to_owned()
            };
            let mut bytes = vec![0x1b];
            bytes.extend_from_slice(c.as_bytes());
            bytes
        }
        "space" => b" ".to_vec(),
        _ => keystroke.key_char.as_ref()?.as_bytes().to_vec(),
    };
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(text: &'static str) -> gpui_kit::Keystroke {
        gpui_kit::Keystroke::parse(text).unwrap()
    }

    #[test]
    fn keys_encode_like_xterm() {
        let normal = TermMode::NONE;
        assert_eq!(encode_key(&key("enter"), normal), Some(b"\r".to_vec()));
        assert_eq!(encode_key(&key("ctrl-c"), normal), Some(vec![3]));
        assert_eq!(encode_key(&key("up"), normal), Some(b"\x1b[A".to_vec()));
        assert_eq!(
            encode_key(&key("up"), TermMode::APP_CURSOR),
            Some(b"\x1bOA".to_vec())
        );
        assert_eq!(
            encode_key(&key("shift-up"), normal),
            Some(b"\x1b[1;2A".to_vec())
        );
        assert_eq!(
            encode_key(&key("shift-tab"), normal),
            Some(b"\x1b[Z".to_vec())
        );
        assert_eq!(
            encode_key(&key("pagedown"), normal),
            Some(b"\x1b[6~".to_vec())
        );
        assert_eq!(encode_key(&key("cmd-c"), normal), None);
    }

    #[test]
    fn prompt_titles_collapse_to_the_program_name() {
        assert_eq!(clean_title("dev@laptop: ~/code/demo", "zsh"), "zsh");
        assert_eq!(clean_title("cargo test", "zsh"), "cargo test");
        assert_eq!(clean_title("", "zsh"), "zsh");
        assert_eq!(clean_title("vim notes: draft", "zsh"), "vim notes: draft");
    }

    #[test]
    fn configured_shells_keep_their_arguments_and_defaults_are_login_shells() {
        let (program, args) = shell_command("/bin/bash --noprofile").unwrap();
        assert_eq!(program, "/bin/bash");
        assert_eq!(args, ["--noprofile"]);
        if !cfg!(windows) {
            assert!(shell_command("").is_some());
        }
    }

    #[test]
    fn palette_has_a_color_for_every_index() {
        let theme = Theme::default();
        for index in 0..=268 {
            let _ = default_color(index, &theme);
        }
    }
}
