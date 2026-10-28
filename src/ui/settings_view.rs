//! Settings, laid out like Notion's: a quiet category list on the left and
//! labelled rows with a control on the right.
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::{
    ActiveTheme, Icon, IconName, Sizable, StyledExt as _, h_flex,
    input::{Input, InputEvent, InputState},
    switch::Switch,
    v_flex,
};
use gpui_kit::{
    AnyElement, App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight,
    Hsla, InteractiveElement, IntoElement, MouseButton, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Window, div, prelude::FluentBuilder,
};

use super::{r, shortcut};
use crate::{
    brand,
    settings::{ReviewBase, ReviewLayout, Settings, ThemeChoice},
};

pub struct Close;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Category {
    Appearance,
    Editor,
    Terminal,
    Review,
    Shortcuts,
    About,
}

impl Category {
    const ALL: [Self; 6] = [
        Self::Appearance,
        Self::Editor,
        Self::Terminal,
        Self::Review,
        Self::Shortcuts,
        Self::About,
    ];
    fn label(self) -> &'static str {
        match self {
            Self::Appearance => "Appearance",
            Self::Editor => "Editor",
            Self::Terminal => "Terminal",
            Self::Review => "Review",
            Self::Shortcuts => "Shortcuts",
            Self::About => "About",
        }
    }
    fn icon(self) -> Lucide {
        match self {
            Self::Appearance => Lucide::Paintbrush,
            Self::Editor => Lucide::Code,
            Self::Terminal => Lucide::Terminal,
            Self::Review => Lucide::GitCompare,
            Self::Shortcuts => Lucide::Keyboard,
            Self::About => Lucide::CircleDot,
        }
    }
}

pub struct SettingsView {
    focus: FocusHandle,
    category: Category,
    font_input: Entity<InputState>,
    shell_input: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<Close> for SettingsView {}

impl SettingsView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = Settings::get(cx).clone();
        let default_font = crate::theme::mono_font("", cx);
        let font_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(format!("{default_font} (default)"))
                .default_value(settings.editor_font_family.clone())
        });
        let shell_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Login shell (default)")
                .default_value(settings.terminal_shell.clone())
        });
        let subscriptions = vec![
            cx.subscribe_in(
                &font_input,
                window,
                |_, input, event: &InputEvent, _, cx| {
                    if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                        let value = input.read(cx).value().trim().to_owned();
                        Settings::update(cx, |s| s.editor_font_family = value);
                    }
                },
            ),
            cx.subscribe_in(
                &shell_input,
                window,
                |_, input, event: &InputEvent, _, cx| {
                    if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                        let value = input.read(cx).value().trim().to_owned();
                        Settings::update(cx, |s| s.terminal_shell = value);
                    }
                },
            ),
        ];
        Self {
            focus: cx.focus_handle(),
            category: Category::Appearance,
            font_input,
            shell_input,
            _subscriptions: subscriptions,
        }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.focus.focus(window, cx);
    }

    fn render_category(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let s = Settings::get(cx).clone();
        match self.category {
            Category::Appearance => vec![
                heading("Theme", cx),
                theme_picker(s.theme, cx),
                row(
                    "Interface size",
                    "Scales text and controls across the whole window.",
                    stepper(
                        "zoom",
                        format!("{}%", (s.zoom * 100.).round()),
                        |s| s.zoom -= 0.1,
                        |s| s.zoom += 0.1,
                        cx,
                    ),
                    cx,
                ),
            ],
            Category::Editor => vec![
                heading("Text", cx),
                row(
                    "Font",
                    "Any installed monospace family.",
                    div()
                        .w(r(220.))
                        .child(Input::new(&self.font_input).small())
                        .into_any_element(),
                    cx,
                ),
                row(
                    "Font size",
                    "Code size in points.",
                    stepper(
                        "font-size",
                        format!("{}", s.editor_font_size),
                        |s| s.editor_font_size -= 1.,
                        |s| s.editor_font_size += 1.,
                        cx,
                    ),
                    cx,
                ),
                row(
                    "Tab size",
                    "Spaces inserted for one indent.",
                    segmented(
                        "tab",
                        &[("2", 2u8), ("4", 4), ("8", 8)],
                        s.tab_size,
                        |s, v| s.tab_size = v,
                        cx,
                    ),
                    cx,
                ),
                heading("Behavior", cx),
                toggle(
                    "Word wrap",
                    "Wrap long lines to the window width.",
                    s.soft_wrap,
                    |s, v| s.soft_wrap = v,
                    cx,
                ),
                toggle(
                    "Line numbers",
                    "Show numbers in the gutter.",
                    s.line_numbers,
                    |s, v| s.line_numbers = v,
                    cx,
                ),
                toggle(
                    "Indent guides",
                    "Faint lines that follow indentation.",
                    s.indent_guides,
                    |s, v| s.indent_guides = v,
                    cx,
                ),
                toggle(
                    "Trim trailing whitespace",
                    "Remove spaces at line ends when saving.",
                    s.format_on_save_trim,
                    |s, v| s.format_on_save_trim = v,
                    cx,
                ),
                toggle(
                    "Follow changes on disk",
                    "Reload files and the review as agents edit them.",
                    s.watch_files,
                    |s, v| s.watch_files = v,
                    cx,
                ),
                toggle(
                    "Restore open files",
                    "Reopen your tabs when you come back to a project.",
                    s.restore_session,
                    |s, v| s.restore_session = v,
                    cx,
                ),
            ],
            Category::Terminal => vec![
                heading("Terminal", cx),
                row(
                    "Shell",
                    "Program to run. Leave empty for your login shell.",
                    div()
                        .w(r(220.))
                        .child(Input::new(&self.shell_input).small())
                        .into_any_element(),
                    cx,
                ),
                row(
                    "Font size",
                    "Terminal text size in points.",
                    stepper(
                        "term-size",
                        format!("{}", s.terminal_font_size),
                        |s| s.terminal_font_size -= 0.5,
                        |s| s.terminal_font_size += 0.5,
                        cx,
                    ),
                    cx,
                ),
            ],
            Category::Review => vec![
                heading("Review", cx),
                row(
                    "Compare against",
                    "Agents usually commit as they go, so the whole branch is the default.",
                    segmented(
                        "base",
                        &[
                            ("Base branch", ReviewBase::Branch),
                            ("Uncommitted", ReviewBase::Uncommitted),
                        ],
                        s.review_base,
                        |s, v| s.review_base = v,
                        cx,
                    ),
                    cx,
                ),
                row(
                    "Layout",
                    "Unified reads top to bottom; split shows before and after.",
                    segmented(
                        "layout",
                        &[
                            ("Unified", ReviewLayout::Unified),
                            ("Split", ReviewLayout::Split),
                        ],
                        s.review_layout,
                        |s, v| s.review_layout = v,
                        cx,
                    ),
                    cx,
                ),
            ],
            Category::Shortcuts => {
                let mut rows = vec![heading("Keyboard shortcuts", cx)];
                for (label, mac, other) in SHORTCUTS {
                    rows.push(shortcut_row(label, shortcut(mac, other), cx));
                }
                rows
            }
            Category::About => vec![about(cx)],
        }
    }
}

const SHORTCUTS: &[(&str, &str, &str)] = &[
    ("Go to file", "⌘P", "Ctrl+P"),
    ("Command palette", "⌘K  or  ⌘⇧P", "Ctrl+K  or  Ctrl+Shift+P"),
    ("Search in files", "⌘⇧F", "Ctrl+Shift+F"),
    ("Review changes", "⌘⇧G", "Ctrl+Shift+G"),
    ("Show files", "⌘⇧E", "Ctrl+Shift+E"),
    ("Find in file", "⌘F", "Ctrl+F"),
    ("Replace in file", "⌥⌘F", "Ctrl+H"),
    ("Go to line", "⌃G", "Ctrl+G"),
    ("Toggle terminal", "⌘J  or  ⌃`", "Ctrl+J  or  Ctrl+`"),
    ("New terminal", "⌃⇧`", "Ctrl+Shift+`"),
    ("Toggle sidebar", "⌘B", "Ctrl+B"),
    ("Save", "⌘S", "Ctrl+S"),
    ("Close tab", "⌘W", "Ctrl+W"),
    (
        "Next / previous tab",
        "⌃Tab  /  ⌃⇧Tab",
        "Ctrl+Tab  /  Ctrl+Shift+Tab",
    ),
    ("Word wrap", "⌥Z", "Alt+Z"),
    ("Open folder", "⌘O", "Ctrl+O"),
    ("New window", "⌘⇧N", "Ctrl+Shift+N"),
    ("Settings", "⌘,", "Ctrl+,"),
    (
        "Zoom in / out / reset",
        "⌘=  ⌘-  ⌘0",
        "Ctrl+=  Ctrl+-  Ctrl+0",
    ),
];

fn heading(text: &'static str, cx: &App) -> AnyElement {
    v_flex()
        .pt(r(20.))
        .gap(r(10.))
        .child(
            div()
                .text_size(r(15.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(text),
        )
        .child(div().h_px().bg(cx.theme().border))
        .into_any_element()
}

fn row(
    label: &'static str,
    description: &'static str,
    control: AnyElement,
    cx: &App,
) -> AnyElement {
    h_flex()
        .py(r(10.))
        .gap(r(24.))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap(r(2.))
                .child(div().text_size(r(14.)).child(label))
                .child(
                    div()
                        .text_size(r(12.5))
                        .text_color(cx.theme().muted_foreground)
                        .child(description),
                ),
        )
        .child(control)
        .into_any_element()
}

fn toggle(
    label: &'static str,
    description: &'static str,
    value: bool,
    set: fn(&mut Settings, bool),
    cx: &App,
) -> AnyElement {
    row(
        label,
        description,
        Switch::new(SharedString::from(label))
            .checked(value)
            .on_click(move |checked, _, cx| {
                let checked = *checked;
                Settings::update(cx, |s| set(s, checked));
            })
            .into_any_element(),
        cx,
    )
}

fn stepper(
    id: &'static str,
    value: String,
    down: fn(&mut Settings),
    up: fn(&mut Settings),
    cx: &App,
) -> AnyElement {
    let theme = cx.theme().clone();
    let hover = theme.list_hover;
    let button = |suffix: &'static str, icon: IconName, f: fn(&mut Settings)| {
        div()
            .id(SharedString::from(format!("{id}-{suffix}")))
            .size(r(26.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(r(5.))
            .cursor_pointer()
            .hover(move |s| s.bg(hover))
            .child(Icon::new(icon).size(r(14.)))
            .on_click(move |_, _, cx| Settings::update(cx, f))
    };
    h_flex()
        .gap(r(2.))
        .p(r(2.))
        .rounded(r(7.))
        .border_1()
        .border_color(theme.border)
        .child(button("down", IconName::Minus, down))
        .child(div().w(r(52.)).text_center().text_size(r(13.)).child(value))
        .child(button("up", IconName::Plus, up))
        .into_any_element()
}

fn segmented<T: Copy + PartialEq + 'static>(
    id: &'static str,
    options: &[(&'static str, T)],
    current: T,
    set: fn(&mut Settings, T),
    cx: &App,
) -> AnyElement {
    let theme = cx.theme().clone();
    let hover = theme.list_hover;
    h_flex()
        .p(r(2.))
        .gap(r(2.))
        .rounded(r(7.))
        .bg(theme.list_hover)
        .children(options.iter().enumerate().map(|(ix, (label, value))| {
            let value = *value;
            let selected = value == current;
            div()
                .id(SharedString::from(format!("{id}-{ix}")))
                .px(r(10.))
                .h(r(24.))
                .flex()
                .items_center()
                .rounded(r(5.))
                .text_size(r(12.5))
                .cursor_pointer()
                .when(selected, |d| {
                    d.bg(theme.background)
                        .text_color(theme.foreground)
                        .font_medium()
                        .shadow_xs()
                })
                .when(!selected, |d| {
                    d.text_color(theme.muted_foreground)
                        .hover(move |s| s.bg(hover))
                })
                .child(*label)
                .on_click(move |_, _, cx| Settings::update(cx, |s| set(s, value)))
        }))
        .into_any_element()
}

/// Theme cards with a miniature window drawn in each palette.
fn theme_picker(current: ThemeChoice, cx: &App) -> AnyElement {
    let theme = cx.theme().clone();
    let swatch = |choice: ThemeChoice| -> (Hsla, Hsla, Hsla, Hsla) {
        let hex = |v: u32| -> Hsla { gpui_kit::rgb(v).into() };
        match choice {
            ThemeChoice::Light | ThemeChoice::System => {
                (hex(0xFFFFFF), hex(0xF8F8F7), hex(0x37352F), hex(0xE9E9E7))
            }
            ThemeChoice::Dark => (hex(0x191919), hex(0x202020), hex(0xD4D4D4), hex(0x2F2F2F)),
            ThemeChoice::Paper => (hex(0xFBFAF6), hex(0xF3F1EA), hex(0x3A362E), hex(0xE6E1D6)),
            ThemeChoice::Graphite => (hex(0x1E1F22), hex(0x18191B), hex(0xD6D7DB), hex(0x2E2F33)),
        }
    };
    let mini = |choice: ThemeChoice| {
        let (bg, sidebar, ink, line) = swatch(choice);
        let window = |bg: Hsla, sidebar: Hsla, ink: Hsla, line: Hsla| {
            h_flex()
                .size_full()
                .bg(bg)
                .child(
                    v_flex()
                        .w(r(30.))
                        .h_full()
                        .bg(sidebar)
                        .p(r(5.))
                        .gap(r(4.))
                        .child(div().h(r(3.)).w(r(16.)).rounded_full().bg(ink.opacity(0.5)))
                        .child(div().h(r(3.)).w(r(12.)).rounded_full().bg(ink.opacity(0.3)))
                        .child(div().h(r(3.)).w(r(14.)).rounded_full().bg(ink.opacity(0.3))),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .p(r(7.))
                        .gap(r(4.))
                        .child(div().h(r(4.)).w(r(40.)).rounded_full().bg(ink.opacity(0.8)))
                        .child(div().h(r(3.)).w(r(56.)).rounded_full().bg(line))
                        .child(div().h(r(3.)).w(r(48.)).rounded_full().bg(line))
                        .child(
                            div()
                                .h(r(3.))
                                .w(r(30.))
                                .rounded_full()
                                .bg(gpui_kit::rgb(0x2383E2)),
                        ),
                )
        };
        if choice == ThemeChoice::System {
            let (dbg, dsb, dink, dline) = swatch(ThemeChoice::Dark);
            h_flex()
                .size_full()
                .child(
                    div()
                        .w_1_2()
                        .h_full()
                        .overflow_hidden()
                        .child(window(bg, sidebar, ink, line)),
                )
                .child(
                    div()
                        .w_1_2()
                        .h_full()
                        .overflow_hidden()
                        .child(window(dbg, dsb, dink, dline)),
                )
                .into_any_element()
        } else {
            window(bg, sidebar, ink, line).into_any_element()
        }
    };
    h_flex()
        .pt(r(14.))
        .pb(r(6.))
        .gap(r(12.))
        .flex_wrap()
        .children(ThemeChoice::ALL.into_iter().map(|choice| {
            let selected = choice == current;
            v_flex()
                .id(SharedString::from(format!("theme-{}", choice.label())))
                .gap(r(6.))
                .cursor_pointer()
                .on_click(move |_, _, cx| Settings::update(cx, |s| s.theme = choice))
                .child(
                    div()
                        .w(r(112.))
                        .h(r(72.))
                        .rounded(r(8.))
                        .overflow_hidden()
                        .border_2()
                        .border_color(if selected {
                            theme.primary
                        } else {
                            theme.border
                        })
                        .child(mini(choice)),
                )
                .child(
                    div()
                        .text_size(r(12.5))
                        .text_color(if selected {
                            theme.foreground
                        } else {
                            theme.muted_foreground
                        })
                        .when(selected, |d| d.font_medium())
                        .child(choice.label()),
                )
        }))
        .into_any_element()
}

fn shortcut_row(label: &'static str, keys: &'static str, cx: &App) -> AnyElement {
    h_flex()
        .py(r(8.))
        .border_b_1()
        .border_color(cx.theme().border.opacity(0.6))
        .child(div().flex_1().text_size(r(13.5)).child(label))
        .child(
            div()
                .text_size(r(12.5))
                .text_color(cx.theme().muted_foreground)
                .child(keys),
        )
        .into_any_element()
}

fn about(cx: &App) -> AnyElement {
    let theme = cx.theme().clone();
    v_flex()
        .pt(r(36.))
        .items_center()
        .gap(r(10.))
        .child(super::welcome::logo(r(64.), cx))
        .child(
            div()
                .text_size(r(22.))
                .font_weight(FontWeight::BOLD)
                .child(brand::NAME),
        )
        .child(
            div()
                .text_size(r(13.5))
                .text_color(theme.muted_foreground)
                .child(brand::TAGLINE),
        )
        .child(
            div()
                .text_size(r(12.5))
                .text_color(theme.muted_foreground)
                .child(format!("Version {}", brand::VERSION)),
        )
        .child(
            div()
                .id("about-repo")
                .pt(r(6.))
                .text_size(r(13.))
                .text_color(theme.primary)
                .cursor_pointer()
                .child("Source code and releases")
                .on_click(|_, _, cx| cx.open_url(brand::REPOSITORY)),
        )
        .child(
            div()
                .pt(r(24.))
                .max_w(r(420.))
                .text_center()
                .text_size(r(12.))
                .text_color(theme.muted_foreground)
                .child(
                    "Built in Rust on GPUI. No extensions, no telemetry, no background indexer.",
                ),
        )
        .into_any_element()
}

impl Focusable for SettingsView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for SettingsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let content = self.render_category(cx);
        let nav = Category::ALL.into_iter().map(|category| {
            let selected = category == self.category;
            super::NavRow::new(SharedString::from(category.label()), category.label())
                .icon(category.icon())
                .selected(selected)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.category = category;
                    cx.notify();
                }))
        });
        div()
            .size_full()
            .track_focus(&self.focus)
            .key_context("Settings")
            .on_action(cx.listener(|_, _: &super::workspace::Dismiss, _, cx| cx.emit(Close)))
            .child(
                div()
                    .id("settings-backdrop")
                    .absolute()
                    .inset_0()
                    .bg(theme.overlay)
                    .on_mouse_down(MouseButton::Left, cx.listener(|_, _, _, cx| cx.emit(Close))),
            )
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        h_flex()
                            .id("settings")
                            .w(r(900.))
                            .h(r(620.))
                            .max_w(gpui_kit::relative(0.92))
                            .max_h(gpui_kit::relative(0.9))
                            .rounded(r(12.))
                            .overflow_hidden()
                            .bg(theme.popover)
                            .border_1()
                            .border_color(theme.border)
                            .shadow_2xl()
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .child(
                                v_flex()
                                    .w(r(220.))
                                    .h_full()
                                    .flex_none()
                                    .bg(theme.sidebar)
                                    .pt(r(14.))
                                    .gap(r(1.))
                                    .child(
                                        div()
                                            .px(r(16.))
                                            .pb(r(10.))
                                            .text_size(r(12.))
                                            .font_medium()
                                            .text_color(theme.muted_foreground)
                                            .child("Settings"),
                                    )
                                    .children(nav),
                            )
                            .child(
                                v_flex()
                                    .id("settings-content")
                                    .flex_1()
                                    .h_full()
                                    .px(r(40.))
                                    .pb(r(32.))
                                    .overflow_y_scroll()
                                    .children(content),
                            )
                            .child(
                                div().absolute().top(r(12.)).right(r(12.)).child(
                                    super::icon_button("settings-close", IconName::Close, cx)
                                        .on_click(cx.listener(|_, _, _, cx| cx.emit(Close))),
                                ),
                            ),
                    ),
            )
    }
}
