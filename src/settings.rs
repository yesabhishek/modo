//! User preferences and the remembered session, stored as JSON in the
//! platform configuration directory and written off the UI thread.
use std::path::{Path, PathBuf};

use gpui_kit::{App, Global};
use serde::{Deserialize, Serialize};

use crate::brand;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ThemeChoice {
    /// Follow the operating system between Light and Dark.
    #[default]
    System,
    Light,
    Dark,
    Paper,
    Graphite,
}

impl ThemeChoice {
    pub const ALL: [Self; 5] = [
        Self::System,
        Self::Light,
        Self::Dark,
        Self::Paper,
        Self::Graphite,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::System => "Match system",
            Self::Light => "Light",
            Self::Dark => "Dark",
            Self::Paper => "Paper",
            Self::Graphite => "Graphite",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ReviewLayout {
    #[default]
    Unified,
    Split,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ReviewBase {
    /// Only what is not committed yet.
    Uncommitted,
    /// Everything since the branch left its base (agents usually commit).
    #[default]
    Branch,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: ThemeChoice,
    /// Interface zoom, 1.0 = 100%.
    pub zoom: f32,
    pub editor_font_family: String,
    pub editor_font_size: f32,
    pub soft_wrap: bool,
    pub line_numbers: bool,
    pub indent_guides: bool,
    pub tab_size: u8,
    pub format_on_save_trim: bool,
    pub terminal_font_size: f32,
    /// Empty means the user's login shell.
    pub terminal_shell: String,
    pub review_layout: ReviewLayout,
    pub review_base: ReviewBase,
    pub show_hidden_files: bool,
    pub restore_session: bool,
    pub watch_files: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: ThemeChoice::System,
            zoom: 1.0,
            editor_font_family: String::new(),
            editor_font_size: 13.0,
            soft_wrap: false,
            line_numbers: true,
            indent_guides: true,
            tab_size: 4,
            format_on_save_trim: false,
            terminal_font_size: 12.5,
            terminal_shell: String::new(),
            review_layout: ReviewLayout::Unified,
            review_base: ReviewBase::Branch,
            show_hidden_files: true,
            restore_session: true,
            watch_files: true,
        }
    }
}

impl Global for Settings {}

impl Settings {
    pub fn get(cx: &App) -> &Self {
        cx.global::<Self>()
    }

    /// Apply a change, persist it, and let observers (theme, editors) react.
    pub fn update(cx: &mut App, edit: impl FnOnce(&mut Self)) {
        let mut next = cx.global::<Self>().clone();
        edit(&mut next);
        next.sanitize();
        if &next == cx.global::<Self>() {
            return;
        }
        store_json("settings.json", &next);
        cx.set_global(next);
    }

    fn sanitize(&mut self) {
        self.zoom = self.zoom.clamp(0.7, 1.6);
        self.editor_font_size = self.editor_font_size.clamp(9.0, 28.0);
        self.terminal_font_size = self.terminal_font_size.clamp(9.0, 28.0);
        self.tab_size = self.tab_size.clamp(1, 8);
    }

    pub fn load() -> Self {
        let mut settings: Self = load_json("settings.json").unwrap_or_default();
        settings.sanitize();
        settings
    }
}

/// A project the user opened, with enough state to put them back where they were.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ProjectSession {
    pub root: PathBuf,
    pub open_files: Vec<PathBuf>,
    pub active_file: Option<PathBuf>,
    pub expanded: Vec<PathBuf>,
    pub sidebar_width: f32,
    pub terminal_open: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Session {
    /// Most recent first.
    pub recent: Vec<PathBuf>,
    pub projects: Vec<ProjectSession>,
}

impl Global for Session {}

impl Session {
    const MAX_RECENT: usize = 12;

    pub fn load() -> Self {
        let mut session: Self = load_json("session.json").unwrap_or_default();
        session.recent.retain(|path| path.is_dir());
        session
    }

    pub fn project(&self, root: &Path) -> Option<&ProjectSession> {
        self.projects.iter().find(|p| p.root == root)
    }

    pub fn remember(cx: &mut App, project: ProjectSession) {
        let mut session = cx.global::<Self>().clone();
        session.recent.retain(|p| p != &project.root);
        session.recent.insert(0, project.root.clone());
        session.recent.truncate(Self::MAX_RECENT);
        session.projects.retain(|p| p.root != project.root);
        session.projects.insert(0, project);
        session.projects.truncate(Self::MAX_RECENT);
        if &session != cx.global::<Self>() {
            store_json("session.json", &session);
            cx.set_global(session);
        }
    }
}

pub fn config_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("MODO_CONFIG_DIR") {
        return Some(PathBuf::from(dir));
    }
    directories::ProjectDirs::from("dev", brand::NAME, brand::NAME)
        .map(|d| d.config_dir().to_owned())
}

fn load_json<T: for<'de> Deserialize<'de>>(name: &str) -> Option<T> {
    if std::env::var_os("MODO_TEMPORARY_SESSION").is_some() {
        return None;
    }
    let bytes = std::fs::read(config_dir()?.join(name)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Files are a few kilobytes, so they are written in place: replace through a
/// temporary file so a crash mid-write never leaves truncated JSON behind.
fn store_json<T: Serialize>(name: &'static str, value: &T) {
    if std::env::var_os("MODO_TEMPORARY_SESSION").is_some() {
        return;
    }
    let Some(dir) = config_dir() else { return };
    let Ok(bytes) = serde_json::to_vec_pretty(value) else {
        return;
    };
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    if let Ok(mut file) = tempfile::NamedTempFile::new_in(&dir) {
        use std::io::Write;
        if file.write_all(&bytes).is_ok() {
            let _ = file.persist(dir.join(name));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_or_partial_settings_fall_back_to_defaults() {
        let parsed: Settings =
            serde_json::from_str(r#"{"editor_font_size": 99, "mystery": 1}"#).unwrap();
        let mut parsed = parsed;
        parsed.sanitize();
        assert_eq!(parsed.editor_font_size, 28.0);
        assert_eq!(parsed.theme, ThemeChoice::System);
        assert!(parsed.line_numbers);
    }
}
