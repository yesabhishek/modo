//! Settings: a short category list on the left, and on the right a few
//! bordered groups of one-line rows, each ending in a single control.
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::{
    ActiveTheme, Disableable as _, Icon, IconName, Sizable, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu, PopupMenuItem},
    switch::Switch,
    v_flex,
};
use gpui_kit::{
    Anchor, AnyElement, App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable,
    FontWeight, Hsla, InteractiveElement, IntoElement, MouseButton, ParentElement, Render,
    SharedString, StatefulInteractiveElement, Styled, Subscription, Task, Window, div,
    prelude::FluentBuilder, px,
};

use super::{r, shortcut};
use crate::{
    brand,
    settings::{AutoSaveDelay, ReviewBase, ReviewLayout, Settings, ThemeChoice},
    shells,
    updater::{self, Phase, UpdateState},
};

pub enum SettingsEvent {
    Close,
    ProjectGradient(bool),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Category {
    General,
    Editor,
    Terminal,
    Shortcuts,
    About,
}

impl Category {
    const ALL: [Self; 5] = [
        Self::General,
        Self::Editor,
        Self::Terminal,
        Self::Shortcuts,
        Self::About,
    ];
    fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Editor => "Editor",
            Self::Terminal => "Terminal",
            Self::Shortcuts => "Shortcuts",
            Self::About => "About",
        }
    }
    fn icon(self) -> Lucide {
        match self {
            Self::General => Lucide::SlidersHorizontal,
            Self::Editor => Lucide::Code,
            Self::Terminal => Lucide::Terminal,
            Self::Shortcuts => Lucide::Keyboard,
            Self::About => Lucide::Info,
        }
    }
}

pub struct SettingsView {
    focus: FocusHandle,
    category: Category,
    gradient_enabled: bool,
    shell_input: Entity<InputState>,
    /// Installed monospace families; `None` while the scan runs.
    fonts: Option<Vec<SharedString>>,
    _fonts: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<SettingsEvent> for SettingsView {}

impl SettingsView {
    pub fn new(gradient_enabled: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = Settings::get(cx).clone();
        let shell_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(if cfg!(windows) {
                    r#""C:\Tools\nu.exe" --login"#
                } else {
                    "/opt/homebrew/bin/fish -l"
                })
                .default_value(settings.terminal_shell.clone())
        });
        let subscriptions = vec![
            cx.subscribe_in(
                &shell_input,
                window,
                |_, input, event: &InputEvent, _, cx| {
                    if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                        let value = input.read(cx).value().trim().to_owned();
                        Settings::update(cx, |s| {
                            // Typing a command means "use it"; clearing it goes back to a shell.
                            if value.is_empty() {
                                if s.terminal_profile == shells::CUSTOM {
                                    s.terminal_profile.clear();
                                }
                            } else if value != s.terminal_shell {
                                s.terminal_profile = shells::CUSTOM.to_owned();
                            }
                            s.terminal_shell = value;
                        });
                    }
                },
            ),
            cx.observe_global::<Settings>(|_, cx| cx.notify()),
            cx.observe_global::<UpdateState>(|_, cx| cx.notify()),
        ];
        // Both steps are slow enough to notice (listing every family, then
        // reading which are monospace), so neither runs on the UI thread.
        let text_system = cx.text_system().clone();
        let fonts = cx.spawn(async move |this, cx| {
            let families = cx
                .background_executor()
                .spawn(
                    async move { crate::theme::monospace_families(&text_system.all_font_names()) },
                )
                .await;
            let _ = this.update(cx, |this, cx| {
                this.fonts = Some(families.into_iter().map(SharedString::from).collect());
                cx.notify();
            });
        });
        Self {
            focus: cx.focus_handle(),
            category: Category::General,
            gradient_enabled,
            shell_input,
            fonts: None,
            _fonts: fonts,
            _subscriptions: subscriptions,
        }
    }

    pub fn show_about(&mut self, cx: &mut Context<Self>) {
        self.category = Category::About;
        cx.notify();
    }

    pub fn set_gradient_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.gradient_enabled = enabled;
        cx.notify();
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.focus.focus(window, cx);
    }

    #[cfg(feature = "capture")]
    pub fn show_terminal(&mut self, cx: &mut Context<Self>) {
        self.category = Category::Terminal;
        cx.notify();
    }

    #[cfg(feature = "capture")]
    pub fn show_editor(&mut self, cx: &mut Context<Self>) {
        self.category = Category::Editor;
        cx.notify();
    }

    fn render_category(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let s = Settings::get(cx).clone();
        match self.category {
            Category::General => vec![
                group(
                    "Theme",
                    vec![
                        theme_picker(s.theme, cx),
                        row(
                            "Interface size",
                            stepper(
                                "zoom",
                                format!("{}%", (s.zoom * 100.).round()),
                                |s| s.zoom -= 0.1,
                                |s| s.zoom += 0.1,
                                cx,
                            ),
                        ),
                        row(
                            "Tint each project",
                            Switch::new("project-gradients")
                                .small()
                                .checked(self.gradient_enabled)
                                .on_click(cx.listener(|this, enabled: &bool, _, cx| {
                                    this.gradient_enabled = *enabled;
                                    Settings::update(cx, |settings| {
                                        settings.window_colors = *enabled
                                    });
                                    cx.emit(SettingsEvent::ProjectGradient(*enabled));
                                    cx.notify();
                                }))
                                .into_any_element(),
                        ),
                    ],
                    cx,
                ),
                group(
                    "Files",
                    vec![
                        toggle("Show hidden files", s.show_hidden_files, |s, v| {
                            s.show_hidden_files = v
                        }),
                        toggle("Show ignored files", s.show_ignored_files, |s, v| {
                            s.show_ignored_files = v
                        }),
                        toggle("Follow changes on disk", s.watch_files, |s, v| {
                            s.watch_files = v
                        }),
                        toggle("Reopen tabs on launch", s.restore_session, |s, v| {
                            s.restore_session = v
                        }),
                    ],
                    cx,
                ),
                group(
                    "Review",
                    vec![
                        row(
                            "Compare against",
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
                        ),
                        row(
                            "Layout",
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
                        ),
                    ],
                    cx,
                ),
            ],
            Category::Editor => vec![
                group(
                    "Text",
                    vec![
                        row("Font", self.font_picker(&s, cx)),
                        row(
                            "Font size",
                            stepper(
                                "font-size",
                                format!("{}", s.editor_font_size),
                                |s| s.editor_font_size -= 1.,
                                |s| s.editor_font_size += 1.,
                                cx,
                            ),
                        ),
                        row(
                            "Tab size",
                            segmented(
                                "tab",
                                &[("2", 2u8), ("4", 4), ("8", 8)],
                                s.tab_size,
                                |s, v| s.tab_size = v,
                                cx,
                            ),
                        ),
                    ],
                    cx,
                ),
                group(
                    "Display",
                    vec![
                        toggle("Word wrap", s.soft_wrap, |s, v| s.soft_wrap = v),
                        toggle("Line numbers", s.line_numbers, |s, v| s.line_numbers = v),
                        toggle("Indent guides", s.indent_guides, |s, v| s.indent_guides = v),
                    ],
                    cx,
                ),
                group(
                    "Saving",
                    vec![
                        row("Auto save", auto_save_picker(&s, cx)),
                        toggle("Trim trailing whitespace", s.format_on_save_trim, |s, v| {
                            s.format_on_save_trim = v
                        }),
                    ],
                    cx,
                ),
            ],
            Category::Terminal => vec![group(
                "Shell",
                vec![
                    row("Default profile", profile_picker(&s, cx)),
                    row(
                        "Custom command",
                        div()
                            .w(r(220.))
                            .child(Input::new(&self.shell_input).small())
                            .into_any_element(),
                    ),
                    row(
                        "Font size",
                        stepper(
                            "term-size",
                            format!("{}", s.terminal_font_size),
                            |s| s.terminal_font_size -= 0.5,
                            |s| s.terminal_font_size += 0.5,
                            cx,
                        ),
                    ),
                ],
                cx,
            )],
            Category::Shortcuts => vec![group(
                "",
                SHORTCUTS
                    .iter()
                    .map(|(label, mac, other)| shortcut_row(label, shortcut(mac, other), cx))
                    .collect(),
                cx,
            )],
            Category::About => vec![
                about_header(cx),
                group(
                    "Updates",
                    vec![
                        update_row(cx),
                        toggle("Update automatically", s.auto_update, |s, v| {
                            s.auto_update = v
                        }),
                    ],
                    cx,
                ),
                group(
                    "Links",
                    vec![
                        link_row("Source code", "GitHub", brand::REPOSITORY.to_owned()),
                        link_row(
                            "Release notes",
                            "Releases",
                            format!("{}/releases", brand::REPOSITORY),
                        ),
                        link_row(
                            brand::PUBLISHER,
                            brand::PUBLISHER_URL
                                .trim_start_matches("https://")
                                .trim_start_matches("http://"),
                            brand::PUBLISHER_URL.to_owned(),
                        ),
                        row(
                            "Startup log",
                            Button::new("about-startup-log")
                                .outline()
                                .small()
                                .label("Open")
                                .on_click(|_, _, _| {
                                    if let Some(dir) = crate::settings::config_dir() {
                                        let _ = open::that(dir.join("startup.log"));
                                    }
                                })
                                .into_any_element(),
                        ),
                    ],
                    cx,
                ),
                div()
                    .text_size(r(11.5))
                    .text_color(cx.theme().muted_foreground)
                    .child("Built with Rust and GPUI · MIT license")
                    .into_any_element(),
            ],
        }
    }

    /// Installed monospace families in a menu, the platform default first.
    fn font_picker(&self, settings: &Settings, cx: &App) -> AnyElement {
        let default: SharedString = format!("Default ({})", crate::theme::mono_font("", cx)).into();
        let current = settings.editor_font_family.trim().to_owned();
        let label: SharedString = if current.is_empty() {
            default.clone()
        } else {
            current.clone().into()
        };
        let fonts = self.fonts.clone();
        Button::new("editor-font")
            .outline()
            .small()
            .min_w(r(180.))
            .max_w(r(260.))
            .label(label)
            .dropdown_caret(true)
            .text_color(cx.theme().foreground)
            .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, window, _| {
                let height = px(320.).min(window.viewport_size().height - px(120.));
                let mut menu = menu.min_w(px(220.)).max_h(height).scrollable(true).item(
                    PopupMenuItem::new(default.clone())
                        .checked(current.is_empty())
                        .on_click(|_, _, cx| {
                            Settings::update(cx, |s| s.editor_font_family.clear())
                        }),
                );
                let Some(fonts) = &fonts else {
                    return menu.separator().label("Looking for fonts…");
                };
                menu = menu.separator();
                // A family typed into settings.json by hand stays selectable.
                if !current.is_empty() && !fonts.iter().any(|f| f.eq_ignore_ascii_case(&current)) {
                    menu = menu.item(PopupMenuItem::new(current.clone()).checked(true));
                }
                for font in fonts {
                    let family = font.to_string();
                    menu = menu.item(
                        PopupMenuItem::new(font.clone())
                            .checked(font.eq_ignore_ascii_case(&current))
                            .on_click(move |_, _, cx| {
                                let family = family.clone();
                                Settings::update(cx, |s| s.editor_font_family = family);
                            }),
                    );
                }
                menu
            })
            .into_any_element()
    }
}

const SHORTCUTS: &[(&str, &str, &str)] = &[
    ("Go to file", "⌘P", "Ctrl+P"),
    ("Command palette", "⌘K  or  ⌘⇧P", "Ctrl+K  or  Ctrl+Shift+P"),
    ("Search in files", "⌘⇧F", "Ctrl+Shift+F"),
    ("Review changes", "⌘⇧G", "Ctrl+Shift+G"),
    ("Show files", "⌘⇧E", "Ctrl+Shift+E"),
    ("Find in file", "⌘F", "Ctrl+F"),
    ("File preview", "⌘⇧V", "Ctrl+Shift+V"),
    ("Format JSON / CSV", "⌥⇧F", "Alt+Shift+F"),
    ("Replace in file", "⌥⌘F", "Ctrl+H"),
    ("Go to line", "⌃G", "Ctrl+G"),
    ("Toggle terminal", "⌘J  or  ⌃`", "Ctrl+J  or  Ctrl+`"),
    ("New terminal", "⌃⇧`", "Ctrl+Shift+`"),
    ("Full-window terminal", "⌘⇧J", "Ctrl+Shift+J"),
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

/// A titled, bordered block of rows separated by hairlines.
fn group(title: &'static str, rows: Vec<AnyElement>, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let border = theme.border;
    v_flex()
        .w_full()
        .flex_none()
        .gap(r(6.))
        .when(!title.is_empty(), |group| {
            group.child(
                div()
                    .px(r(2.))
                    .text_size(r(12.))
                    .font_medium()
                    .text_color(theme.muted_foreground)
                    .child(title),
            )
        })
        .child(
            v_flex()
                .w_full()
                .rounded(r(8.))
                .border_1()
                .border_color(border)
                .bg(theme.background)
                .overflow_hidden()
                .children(rows.into_iter().enumerate().map(|(ix, row)| {
                    div()
                        .w_full()
                        .when(ix > 0, |d| d.border_t_1().border_color(border.opacity(0.7)))
                        .child(row)
                })),
        )
        .into_any_element()
}

fn row(label: impl Into<SharedString>, control: AnyElement) -> AnyElement {
    h_flex()
        .w_full()
        .min_h(r(40.))
        .px(r(12.))
        .py(r(6.))
        .gap(r(16.))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(r(13.))
                .child(label.into()),
        )
        .child(div().flex_none().child(control))
        .into_any_element()
}

/// A row whose label needs one short line of explanation.
fn row_hint(
    label: &'static str,
    hint: impl Into<SharedString>,
    control: AnyElement,
    cx: &App,
) -> AnyElement {
    h_flex()
        .w_full()
        .min_h(r(40.))
        .px(r(12.))
        .py(r(7.))
        .gap(r(16.))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap(r(1.))
                .child(div().truncate().text_size(r(13.)).child(label))
                .child(
                    div()
                        .text_size(r(12.))
                        .text_color(cx.theme().muted_foreground)
                        .child(hint.into()),
                ),
        )
        .child(div().flex_none().child(control))
        .into_any_element()
}

fn switch(label: &'static str, value: bool, set: fn(&mut Settings, bool)) -> AnyElement {
    Switch::new(SharedString::from(label))
        .small()
        .checked(value)
        .on_click(move |checked, _, cx| {
            let checked = *checked;
            Settings::update(cx, |s| set(s, checked));
        })
        .into_any_element()
}

fn toggle(label: &'static str, value: bool, set: fn(&mut Settings, bool)) -> AnyElement {
    row(label, switch(label, value, set))
}

/// Off, or how soon after typing an edited file is written.
fn auto_save_picker(settings: &Settings, cx: &App) -> AnyElement {
    const CHOICES: [(&str, Option<AutoSaveDelay>); 4] = [
        ("Off", None),
        ("After 0.25 s", Some(AutoSaveDelay::Quick)),
        ("After 1 s", Some(AutoSaveDelay::Relaxed)),
        ("When leaving the file", Some(AutoSaveDelay::OnLeave)),
    ];
    let current = settings.auto_save.then_some(settings.auto_save_delay);
    let label = CHOICES
        .iter()
        .find(|(_, choice)| *choice == current)
        .map_or("Off", |(label, _)| label);
    Button::new("auto-save")
        .outline()
        .small()
        .min_w(r(180.))
        .label(label)
        .dropdown_caret(true)
        .text_color(cx.theme().foreground)
        .dropdown_menu_with_anchor(Anchor::TopRight, move |mut menu, _, _| {
            menu = menu.min_w(px(200.));
            for (label, choice) in CHOICES {
                menu = menu.item(
                    PopupMenuItem::new(label)
                        .checked(choice == current)
                        .on_click(move |_, _, cx| {
                            Settings::update(cx, |s| {
                                s.auto_save = choice.is_some();
                                if let Some(delay) = choice {
                                    s.auto_save_delay = delay;
                                }
                            })
                        }),
                );
            }
            menu
        })
        .into_any_element()
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
            .size(r(22.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(r(4.))
            .cursor_pointer()
            .text_color(theme.muted_foreground)
            .hover(move |s| s.bg(hover))
            .child(Icon::new(icon).size(r(13.)))
            .on_click(move |_, _, cx| Settings::update(cx, f))
    };
    h_flex()
        .p(r(1.))
        .rounded(r(6.))
        .border_1()
        .border_color(theme.border)
        .child(button("down", IconName::Minus, down))
        .child(div().w(r(48.)).text_center().text_size(r(13.)).child(value))
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
        .rounded(r(6.))
        .bg(theme.list_hover)
        .children(options.iter().enumerate().map(|(ix, (label, value))| {
            let value = *value;
            let selected = value == current;
            div()
                .id(SharedString::from(format!("{id}-{ix}")))
                .px(r(9.))
                .h(r(22.))
                .min_w(r(28.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(r(4.))
                .text_size(r(12.))
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

/// A menu of the shells found on this machine, plus the custom command.
fn profile_picker(settings: &Settings, cx: &App) -> AnyElement {
    let current = shells::resolve(&settings.terminal_profile, &settings.terminal_shell)
        .map(|p| p.name)
        .unwrap_or_else(|| "PowerShell".into());
    Button::new("terminal-profile")
        .outline()
        .small()
        .min_w(r(180.))
        .label(current.clone())
        .dropdown_caret(true)
        .text_color(cx.theme().foreground)
        .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, cx| {
            let custom = Settings::get(cx).terminal_shell.clone();
            let mut menu = menu.min_w(px(200.));
            for profile in shells::available(&custom) {
                let name = profile.name.clone();
                let detail = if name == shells::CUSTOM {
                    custom.clone()
                } else {
                    profile.program.clone()
                };
                menu = menu.item(
                    PopupMenuItem::element(move |_, cx| {
                        v_flex()
                            .py(r(1.))
                            .child(div().text_size(r(13.)).child(name.clone()))
                            .child(
                                div()
                                    .max_w(r(320.))
                                    .truncate()
                                    .text_size(r(11.5))
                                    .text_color(cx.theme().muted_foreground)
                                    .child(detail.clone()),
                            )
                    })
                    .checked(profile.name == current)
                    .on_click(move |_, _, cx| {
                        let name = profile.name.clone();
                        Settings::update(cx, |s| s.terminal_profile = name);
                    }),
                );
            }
            menu
        })
        .into_any_element()
}

/// Small swatches, each a miniature window drawn in its palette.
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
            ThemeChoice::Midnight => (hex(0x0D0F14), hex(0x090B10), hex(0xD6D7DB), hex(0x232732)),
            ThemeChoice::Black => (hex(0x000000), hex(0x080808), hex(0xD6D7DB), hex(0x252525)),
        }
    };
    let window = |(bg, sidebar, ink, line): (Hsla, Hsla, Hsla, Hsla)| {
        h_flex()
            .size_full()
            .bg(bg)
            .child(div().w(r(16.)).h_full().bg(sidebar))
            .child(
                v_flex()
                    .flex_1()
                    .p(r(6.))
                    .gap(r(3.))
                    .child(
                        div()
                            .h(r(3.))
                            .w(r(22.))
                            .rounded_full()
                            .bg(ink.opacity(0.75)),
                    )
                    .child(div().h(r(3.)).w(r(30.)).rounded_full().bg(line))
                    .child(
                        div()
                            .h(r(3.))
                            .w(r(16.))
                            .rounded_full()
                            .bg(gpui_kit::rgb(0x2383E2)),
                    ),
            )
    };
    let mini = |choice: ThemeChoice| {
        if choice == ThemeChoice::System {
            // Half light, half dark.
            h_flex()
                .size_full()
                .child(
                    div()
                        .w_1_2()
                        .h_full()
                        .overflow_hidden()
                        .child(window(swatch(ThemeChoice::Light))),
                )
                .child(
                    div()
                        .w_1_2()
                        .h_full()
                        .overflow_hidden()
                        .child(window(swatch(ThemeChoice::Dark))),
                )
                .into_any_element()
        } else {
            window(swatch(choice)).into_any_element()
        }
    };
    h_flex()
        .w_full()
        .px(r(12.))
        .py(r(12.))
        .gap(r(10.))
        .flex_wrap()
        .children(ThemeChoice::ALL.into_iter().map(|choice| {
            let selected = choice == current;
            v_flex()
                .id(SharedString::from(format!("theme-{}", choice.label())))
                .gap(r(5.))
                .items_center()
                .cursor_pointer()
                .on_click(move |_, _, cx| Settings::update(cx, |s| s.theme = choice))
                .child(
                    div()
                        .w(r(62.))
                        .h(r(40.))
                        .rounded(r(6.))
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
                        .text_size(r(11.5))
                        .text_color(if selected {
                            theme.foreground
                        } else {
                            theme.muted_foreground
                        })
                        .when(selected, |d| d.font_medium())
                        .child(if choice == ThemeChoice::System {
                            "System"
                        } else {
                            choice.label()
                        }),
                )
        }))
        .into_any_element()
}

fn shortcut_row(label: &'static str, keys: &'static str, cx: &App) -> AnyElement {
    h_flex()
        .w_full()
        .h(r(32.))
        .px(r(12.))
        .gap(r(16.))
        .child(div().flex_1().truncate().text_size(r(13.)).child(label))
        .child(
            div()
                .flex_none()
                .text_size(r(12.))
                .text_color(cx.theme().muted_foreground)
                .child(keys),
        )
        .into_any_element()
}

fn about_header(cx: &App) -> AnyElement {
    let theme = cx.theme();
    h_flex()
        .w_full()
        .flex_none()
        .gap(r(12.))
        .child(super::welcome::logo(r(40.), cx))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap(r(2.))
                .child(
                    h_flex()
                        .gap(r(8.))
                        .child(
                            div()
                                .text_size(r(16.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(brand::NAME),
                        )
                        .child(
                            div()
                                .px(r(6.))
                                .py(r(1.))
                                .rounded(r(4.))
                                .bg(theme.list_hover)
                                .text_size(r(11.5))
                                .text_color(theme.muted_foreground)
                                .child(brand::VERSION),
                        ),
                )
                .child(
                    div()
                        .text_size(r(12.))
                        .text_color(theme.muted_foreground)
                        .child(brand::TAGLINE),
                ),
        )
        .into_any_element()
}

/// The version, what the last check found, and the one action that fits it.
fn update_row(cx: &App) -> AnyElement {
    let state = UpdateState::get(cx);
    let (label, enabled): (&'static str, bool) = match &state.phase {
        Phase::Checking => ("Checking…", false),
        Phase::Downloading { .. } => ("Downloading…", false),
        Phase::Available {
            installable: true, ..
        } => ("Download and install", true),
        Phase::Available { .. } => ("Open download page", true),
        Phase::Ready { .. } if cfg!(target_os = "windows") => ("Quit to install", true),
        Phase::Ready { .. } => ("Restart now", true),
        Phase::Package { .. } => ("Install…", true),
        Phase::Idle | Phase::UpToDate | Phase::Failed(_) => ("Check for updates", true),
    };
    let primary = matches!(
        state.phase,
        Phase::Available { .. } | Phase::Ready { .. } | Phase::Package { .. }
    );
    let phase = state.phase.clone();
    let button = Button::new("check-for-updates")
        .small()
        .label(label)
        .disabled(!enabled)
        .map(|button| {
            if primary {
                button.primary()
            } else {
                button.outline()
            }
        })
        .on_click(move |_, _, cx| match phase {
            Phase::Available { .. } | Phase::Package { .. } => updater::install_available(cx),
            Phase::Ready { .. } => updater::restart(cx),
            _ => updater::check(cx),
        });
    row_hint("Version", state.status(), button.into_any_element(), cx)
}

fn link_row(label: &'static str, detail: &'static str, url: String) -> AnyElement {
    row(
        label,
        Button::new(SharedString::from(format!("link-{label}")))
            .ghost()
            .small()
            .label(detail)
            .icon(Icon::new(Lucide::ExternalLink).size(r(13.)))
            .on_click(move |_, _, cx| cx.open_url(&url))
            .into_any_element(),
    )
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
        let category = self.category;
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
            .on_action(
                cx.listener(|_, _: &super::workspace::Dismiss, _, cx| {
                    cx.emit(SettingsEvent::Close)
                }),
            )
            .child(
                // Occluding keeps the wheel and the pointer off the page behind.
                div()
                    .id("settings-backdrop")
                    .absolute()
                    .inset_0()
                    .occlude()
                    .bg(theme.overlay)
                    .on_scroll(|_, _, cx| cx.stop_propagation())
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|_, _, _, cx| cx.emit(SettingsEvent::Close)),
                    ),
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
                            .occlude()
                            .w(r(760.))
                            .h(r(540.))
                            .max_w(gpui_kit::relative(0.94))
                            .max_h(gpui_kit::relative(0.9))
                            .rounded(r(12.))
                            .overflow_hidden()
                            .bg(theme.popover)
                            .border_1()
                            .border_color(theme.border)
                            .shadow_2xl()
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_scroll(|_, _, cx| cx.stop_propagation())
                            .child(
                                v_flex()
                                    .w(r(176.))
                                    .h_full()
                                    .flex_none()
                                    .bg(theme.sidebar)
                                    .border_r_1()
                                    .border_color(theme.border.opacity(0.7))
                                    .pt(r(12.))
                                    .gap(r(1.))
                                    .child(
                                        div()
                                            .px(r(14.))
                                            .pb(r(8.))
                                            .text_size(r(12.))
                                            .font_medium()
                                            .text_color(theme.muted_foreground)
                                            .child("Settings"),
                                    )
                                    .children(nav),
                            )
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .h_full()
                                    // The title stays put; only the rows scroll.
                                    .child(
                                        h_flex()
                                            .flex_none()
                                            .h(r(46.))
                                            .pl(r(24.))
                                            .pr(r(10.))
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .text_size(r(15.))
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .child(category.label()),
                                            )
                                            .child(
                                                super::icon_button(
                                                    "settings-close",
                                                    IconName::Close,
                                                    cx,
                                                )
                                                .on_click(cx.listener(|_, _, _, cx| {
                                                    cx.emit(SettingsEvent::Close)
                                                })),
                                            ),
                                    )
                                    .child(
                                        // One scroll position per category.
                                        div()
                                            .id(("settings-content", category as usize))
                                            .flex_1()
                                            .min_h_0()
                                            .overflow_y_scroll()
                                            .child(
                                                v_flex()
                                                    .w_full()
                                                    .px(r(24.))
                                                    .pt(r(2.))
                                                    .pb(r(24.))
                                                    .gap(r(18.))
                                                    .children(content),
                                            ),
                                    ),
                            ),
                    ),
            )
    }
}
