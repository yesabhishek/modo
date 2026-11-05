//! An open file: its editor state, save bookkeeping, and the page around it.
use std::path::{Path, PathBuf};

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::{
    ActiveTheme, Icon, IconName, Sizable, StyledExt as _, h_flex,
    input::{Editor, EditorState, InputEvent, Position, TabSize},
    tooltip::Tooltip,
    v_flex,
};
use gpui_kit::{
    App, AppContext, Context, Entity, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Task, Window, div, prelude::FluentBuilder,
};

use super::{
    icon_button, preview, r,
    workspace::{FormatDocument, Tab, ToggleMarkdownPreview, Workspace},
};
use crate::{
    document::{self, LineEnding, LoadedFile},
    files,
    settings::Settings,
    structured::Kind,
};

/// Files above this size open without syntax highlighting to stay instant.
const HIGHLIGHT_LIMIT: usize = 2 * 1024 * 1024;

pub struct FileTab {
    pub path: PathBuf,
    pub editor: Entity<EditorState>,
    pub language: &'static str,
    pub line_ending: LineEnding,
    pub bom: bool,
    pub fingerprint: blake3::Hash,
    pub dirty: bool,
    /// Bumps on every edit, so a save can tell whether it captured the latest text.
    pub version: u64,
    saving: Option<u64>,
    pub auto_save_task: Option<Task<()>>,
    auto_save_failed: Option<u64>,
    /// The file changed on disk while it had unsaved edits.
    pub disk_changed: bool,
    /// Show the rendered markdown instead of the source.
    pub preview: bool,
    structured_kind: Option<Kind>,
    structured_view: Option<Entity<super::data_view::DataView>>,
    structured_version: Option<u64>,
    pub formatting: bool,
    /// The text last handed to the preview, and the edit version it came from.
    preview_text: Option<(u64, SharedString)>,
    applied: Option<Settings>,
    _subscription: Subscription,
}

pub struct SaveJob {
    path: PathBuf,
    text: String,
    line_ending: LineEnding,
    bom: bool,
    expected: blake3::Hash,
    version: u64,
}

pub type SaveResult = (u64, Result<blake3::Hash, String>);

impl SaveJob {
    pub fn run(self) -> SaveResult {
        let result = document::save(
            &self.path,
            &self.text,
            self.line_ending,
            self.bom,
            Some(self.expected),
        );
        (self.version, result)
    }
}

pub struct StatusInfo {
    pub position: SharedString,
    pub indent: SharedString,
    pub encoding: SharedString,
    pub line_ending: SharedString,
    pub language: SharedString,
}

fn tab_size(settings: &Settings) -> TabSize {
    TabSize {
        tab_size: settings.tab_size as usize,
        hard_tabs: false,
    }
}

impl FileTab {
    pub fn new(file: LoadedFile, window: &mut Window, cx: &mut Context<Workspace>) -> Self {
        let settings = Settings::get(cx).clone();
        let language = if file.text.len() > HIGHLIGHT_LIMIT {
            "text"
        } else {
            files::language_for(&file.path)
        };
        let structured_kind = Kind::for_path(&file.path);
        let preview = structured_kind.is_some() && !file.text.is_empty();
        let text = file.text;
        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language(language)
                .line_number(settings.line_numbers)
                .indent_guides(settings.indent_guides)
                .soft_wrap(settings.soft_wrap)
                .tab_size(tab_size(&settings))
                .searchable(true)
                .default_value(text)
        });
        let subscription = cx.subscribe_in(
            &editor,
            window,
            |this: &mut Workspace, editor, event, window, cx| {
                if matches!(event, InputEvent::Change)
                    && let Some(ix) = this
                        .tabs
                        .iter()
                        .position(|tab| matches!(tab, Tab::File(file) if &file.editor == editor))
                {
                    if let Tab::File(file) = &mut this.tabs[ix] {
                        file.dirty = true;
                        file.version += 1;
                    }
                    this.status = None;
                    this.schedule_auto_save(ix, window, cx);
                    cx.notify();
                }
            },
        );
        Self {
            path: file.path,
            editor,
            language,
            line_ending: file.line_ending,
            bom: file.bom,
            fingerprint: file.fingerprint,
            dirty: false,
            version: 0,
            saving: None,
            auto_save_task: None,
            auto_save_failed: None,
            disk_changed: false,
            preview,
            structured_kind,
            structured_view: None,
            structured_version: None,
            formatting: false,
            preview_text: None,
            applied: Some(settings),
            _subscription: subscription,
        }
    }

    #[cfg(feature = "capture")]
    pub fn capture_data_state(&self, cx: &App) -> Option<(usize, bool, Option<SharedString>)> {
        self.structured_view
            .as_ref()
            .map(|view| view.read(cx).capture_state())
    }

    pub fn is_markdown(&self) -> bool {
        preview::is_markdown(self.language)
    }

    /// Flip between the markdown source and its rendered preview.
    pub fn toggle_preview(&mut self, window: &mut Window, cx: &mut App) {
        if !self.is_markdown() && self.structured_kind.is_none() {
            return;
        }
        self.preview = !self.preview;
        if self.preview && self.is_markdown() {
            preview::warm_up();
        }
        self.focus(window, cx);
    }

    fn preview_source(&mut self, cx: &App) -> SharedString {
        match &self.preview_text {
            Some((version, text)) if *version == self.version => text.clone(),
            _ => {
                let text = self.editor.read(cx).value();
                self.preview_text = Some((self.version, text.clone()));
                text
            }
        }
    }

    pub fn name(&self) -> SharedString {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
            .into()
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        if self.preview
            && let Some(view) = &self.structured_view
        {
            view.update(cx, |view, cx| view.focus(window, cx));
        } else {
            self.editor.update(cx, |state, cx| state.focus(window, cx));
        }
    }

    pub fn go_to_line(&mut self, line: u32, window: &mut Window, cx: &mut App) {
        self.preview = false;
        self.editor.update(cx, |state, cx| {
            state.set_cursor_position(Position::new(line.saturating_sub(1), 0), window, cx);
            state.focus(window, cx);
        });
    }

    pub fn selected_text(&self, cx: &App) -> Option<String> {
        let text = self.editor.read(cx).selected_text().to_string();
        (!text.is_empty() && !text.contains('\n') && text.len() < 200).then_some(text)
    }

    /// Settings apply lazily, when a tab is drawn, because editor setters need a window.
    pub fn apply_settings(&self, _settings: &Settings, _cx: &mut App) {}

    pub fn sync_settings(&mut self, window: &mut Window, cx: &mut App) {
        let settings = Settings::get(cx).clone();
        if self.applied.as_ref() == Some(&settings) {
            return;
        }
        self.editor.update(cx, |state, cx| {
            state.set_soft_wrap(settings.soft_wrap, window, cx);
            state.set_line_number(settings.line_numbers, window, cx);
            state.set_indent_guides(settings.indent_guides, window, cx);
            state.set_tab_size(tab_size(&settings), cx);
        });
        self.applied = Some(settings);
    }

    pub fn auto_save_ready(&self) -> bool {
        self.dirty
            && !self.disk_changed
            && self.saving.is_none()
            && self.auto_save_failed != Some(self.version)
    }

    pub fn begin_save(&mut self, cx: &App) -> Option<SaveJob> {
        if self.saving.is_some() {
            return None;
        }
        let mut text = self.editor.read(cx).value().to_string();
        if Settings::get(cx).format_on_save_trim
            && !matches!(self.structured_kind, Some(Kind::Csv(_)))
        {
            text = trim_trailing_whitespace(&text);
        }
        self.saving = Some(self.version);
        Some(SaveJob {
            path: self.path.clone(),
            text,
            line_ending: self.line_ending,
            bom: self.bom,
            expected: self.fingerprint,
            version: self.version,
        })
    }

    pub fn finish_save(
        &mut self,
        (version, result): SaveResult,
        _cx: &mut App,
    ) -> Result<(), SharedString> {
        self.saving = None;
        let hash = result.map_err(|error| {
            self.auto_save_failed = Some(version);
            SharedString::from(error)
        })?;
        self.auto_save_failed = None;
        self.fingerprint = hash;
        self.disk_changed = false;
        // Edits made while the save ran stay dirty.
        if self.version == version {
            self.dirty = false;
        }
        Ok(())
    }

    /// Pick up edits made outside Modo (an agent, a formatter, git checkout).
    pub fn reload_if_changed(&mut self, window: &mut Window, cx: &mut App) {
        if self.saving.is_some() {
            return;
        }
        let Ok(Some(current)) = document::fingerprint(&self.path) else {
            return;
        };
        if current == self.fingerprint {
            return;
        }
        if self.dirty {
            self.disk_changed = true;
            return;
        }
        let Ok(file) = document::load(&self.path) else {
            return;
        };
        self.fingerprint = file.fingerprint;
        self.line_ending = file.line_ending;
        self.bom = file.bom;
        self.editor.update(cx, |state, cx| {
            let position = state.cursor_position();
            state.set_value(file.text, window, cx);
            state.set_cursor_position(position, window, cx);
        });
        self.preview_text = None;
        self.structured_version = None;
        self.dirty = false;
        self.disk_changed = false;
    }

    /// Throw away local edits and take the version on disk.
    pub fn revert(&mut self, window: &mut Window, cx: &mut App) {
        self.dirty = false;
        self.fingerprint = blake3::hash(b"");
        self.reload_if_changed(window, cx);
    }

    pub fn status_info(&self, cx: &App) -> StatusInfo {
        let state = self.editor.read(cx);
        let position = state.cursor_position();
        let settings = Settings::get(cx);
        StatusInfo {
            position: format!("Ln {}, Col {}", position.line + 1, position.character + 1).into(),
            indent: format!("Spaces: {}", settings.tab_size).into(),
            encoding: if self.bom { "UTF-8 BOM" } else { "UTF-8" }.into(),
            line_ending: self.line_ending.label().into(),
            language: match self.structured_kind {
                Some(Kind::Csv(b'\t')) => "TSV",
                Some(Kind::Csv(_)) => "CSV",
                _ => files::language_label(self.language),
            }
            .into(),
        }
    }

    pub fn render(
        &mut self,
        root: Option<&Path>,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) -> impl IntoElement + use<> {
        self.sync_settings(window, cx);
        let theme = cx.theme().clone();
        let relative = root
            .map(|root| files::display_relative(root, &self.path))
            .unwrap_or_else(|| self.path.display().to_string());
        let parts: Vec<String> = relative.split('/').map(str::to_owned).collect();
        let last = parts.len().saturating_sub(1);
        let (muted, strong) = (theme.muted_foreground, theme.foreground);
        let crumbs = parts.into_iter().enumerate().flat_map(move |(ix, part)| {
            let sep = (ix > 0).then(|| {
                div()
                    .text_color(muted.opacity(0.6))
                    .child("/")
                    .into_any_element()
            });
            let label = div()
                .when(ix == last, |d| d.text_color(strong))
                .child(part)
                .into_any_element();
            sep.into_iter().chain(std::iter::once(label))
        });
        let path = self.path.clone();
        let is_markdown = self.is_markdown();
        let can_preview = is_markdown || self.structured_kind.is_some();
        let previewing = self.preview && can_preview;
        if previewing
            && let Some(kind) = self.structured_kind
            && self.structured_version != Some(self.version)
        {
            let source = self.preview_source(cx);
            if let Some(view) = &self.structured_view {
                view.update(cx, |view, cx| view.set_source(source, cx));
            } else {
                let view = cx.new(|cx| super::data_view::DataView::new(kind, source, window, cx));
                view.update(cx, |view, cx| view.focus(window, cx));
                self.structured_view = Some(view);
            }
            self.structured_version = Some(self.version);
        }
        let structured = previewing.then(|| self.structured_view.clone()).flatten();
        let preview_source = (previewing && is_markdown).then(|| self.preview_source(cx));
        let scroll_id = SharedString::from(format!("preview:{}", self.path.display()));
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .flex_none()
                    .h(r(30.))
                    .px(r(16.))
                    .gap(r(6.))
                    .text_size(r(12.5))
                    .text_color(theme.muted_foreground)
                    .child(h_flex().flex_1().min_w_0().gap(r(6.)).children(crumbs))
                    .when(self.structured_kind.is_some() && !previewing, |bar| {
                        bar.child(
                            gpui_kit::component::button::Button::new("format-document")
                                .small()
                                .label(if self.formatting {
                                    "Formatting…"
                                } else {
                                    "Format"
                                })
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(Box::new(FormatDocument), cx)
                                }),
                        )
                    })
                    .when_some(self.structured_kind, |bar, kind| {
                        bar.child(
                            gpui_kit::component::button::Button::new("toggle-data-view")
                                .small()
                                .label(if previewing {
                                    "Source"
                                } else if kind == Kind::Json {
                                    "JSON tree"
                                } else {
                                    "Table"
                                })
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(Box::new(ToggleMarkdownPreview), cx)
                                }),
                        )
                    })
                    .when(can_preview && self.structured_kind.is_none(), |bar| {
                        bar.child(
                            icon_button(
                                "toggle-markdown-preview",
                                if previewing {
                                    Lucide::Code
                                } else {
                                    Lucide::Eye
                                },
                                cx,
                            )
                            .tooltip(move |window, cx| {
                                Tooltip::new(if previewing {
                                    "Edit source (⇧⌘V)"
                                } else {
                                    "Preview file (⇧⌘V)"
                                })
                                .build(window, cx)
                            })
                            .on_click(|_, window, cx| {
                                window.dispatch_action(Box::new(ToggleMarkdownPreview), cx);
                            }),
                        )
                    }),
            )
            .when(self.disk_changed, |page| {
                page.child(
                    h_flex()
                        .flex_none()
                        .mx(r(16.))
                        .mb(r(6.))
                        .px(r(10.))
                        .py(r(6.))
                        .gap(r(8.))
                        .rounded(r(6.))
                        .bg(theme.yellow_light)
                        .text_size(r(12.5))
                        .child(
                            Icon::new(IconName::TriangleAlert)
                                .size(r(14.))
                                .text_color(theme.yellow),
                        )
                        .child(
                            div()
                                .flex_1()
                                .child("This file changed on disk while you were editing it."),
                        )
                        .child(
                            div()
                                .id("reload-from-disk")
                                .px(r(8.))
                                .py(r(2.))
                                .rounded(r(4.))
                                .cursor_pointer()
                                .font_medium()
                                .hover(|s| s.bg(theme.list_hover))
                                .child("Use disk version")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    if let Some(ix) = this.tab_for_path(&path)
                                        && let Some(Tab::File(file)) = this.tabs.get_mut(ix)
                                    {
                                        file.revert(window, cx);
                                        cx.notify();
                                    }
                                })),
                        ),
                )
            })
            .child(if let Some(view) = structured {
                div().flex_1().min_h_0().child(view).into_any_element()
            } else {
                match preview_source {
                    Some(text) => div()
                        .id(scroll_id.clone())
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .child(preview::body(scroll_id, text, cx))
                        .into_any_element(),
                    None => div()
                        .flex_1()
                        .min_h_0()
                        .child(
                            Editor::new(&self.editor)
                                .bordered(false)
                                .appearance(false)
                                .size_full()
                                .xsmall(),
                        )
                        .into_any_element(),
                }
            })
    }
}

fn trim_trailing_whitespace(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(line.trim_end_matches([' ', '\t']));
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn trimming_keeps_line_structure() {
        assert_eq!(
            super::trim_trailing_whitespace("a  \n\tb\t\n\n"),
            "a\n\tb\n\n"
        );
    }
}
