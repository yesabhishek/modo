//! Modo's palettes (Light, Dark, Paper, Graphite) and the mapping from the
//! user's choice and the OS appearance onto GPUI Kit's theme.
use std::rc::Rc;

use gpui_kit::{
    App, SharedString, Window, WindowAppearance,
    component::{Theme, ThemeConfig, ThemeMode, ThemeRegistry},
    px,
};

use crate::settings::{Settings, ThemeChoice};

const THEMES: &str = include_str!("../assets/themes/modo.json");

pub fn init(cx: &mut App) {
    if let Err(error) = ThemeRegistry::global_mut(cx).load_themes_from_str(THEMES) {
        eprintln!("modo: built-in themes failed to load: {error}");
    }
    apply(None, cx);
    // Re-theme whenever preferences change.
    let mut last = Settings::get(cx).clone();
    cx.observe_global::<Settings>(move |cx| {
        let next = Settings::get(cx).clone();
        if next.theme != last.theme
            || next.editor_font_family != last.editor_font_family
            || next.editor_font_size != last.editor_font_size
        {
            apply(None, cx);
        }
        last = next;
    })
    .detach();
}

fn config(name: &str, cx: &App) -> Option<Rc<ThemeConfig>> {
    ThemeRegistry::global(cx).themes().get(name).cloned()
}

/// Apply the current preference. `window` supplies the OS appearance when the
/// user follows the system; otherwise the platform's appearance is used.
pub fn apply(window: Option<&mut Window>, cx: &mut App) {
    let settings = Settings::get(cx).clone();
    let appearance = window
        .as_ref()
        .map(|w| w.appearance())
        .unwrap_or_else(|| cx.window_appearance());
    let (light, dark, mode) = match settings.theme {
        ThemeChoice::System => ("Modo Light", "Modo Dark", ThemeMode::from(appearance)),
        ThemeChoice::Light => ("Modo Light", "Modo Dark", ThemeMode::Light),
        ThemeChoice::Dark => ("Modo Light", "Modo Dark", ThemeMode::Dark),
        ThemeChoice::Paper => ("Modo Paper", "Modo Dark", ThemeMode::Light),
        ThemeChoice::Graphite => ("Modo Light", "Modo Graphite", ThemeMode::Dark),
    };
    let (Some(light), Some(dark)) = (config(light, cx), config(dark, cx)) else {
        return;
    };
    Theme::update(cx, |theme| {
        theme.light_theme = light;
        theme.dark_theme = dark;
    });
    Theme::change(mode, None, cx);
    let mono = mono_font(&settings.editor_font_family, cx);
    Theme::update(cx, |theme| {
        theme.mono_font_family = mono;
        theme.mono_font_size = px(settings.editor_font_size);
        theme.focus_ring = false;
        theme.shadow = false;
    });
}

/// React to the OS switching between light and dark.
pub fn appearance_changed(_appearance: WindowAppearance, window: &mut Window, cx: &mut App) {
    if Settings::get(cx).theme == ThemeChoice::System {
        apply(Some(window), cx);
    }
}

/// The user's family if installed, else the best monospace face on this platform.
pub fn mono_font(preferred: &str, cx: &App) -> SharedString {
    let installed = cx.text_system().all_font_names();
    let has = |name: &str| installed.iter().any(|f| f.eq_ignore_ascii_case(name));
    if !preferred.trim().is_empty() && has(preferred.trim()) {
        return preferred.trim().to_owned().into();
    }
    let candidates: &[&str] = if cfg!(target_os = "macos") {
        &["SF Mono", "Menlo", "Monaco"]
    } else if cfg!(target_os = "windows") {
        &["Cascadia Mono", "Cascadia Code", "Consolas"]
    } else {
        &[
            "JetBrains Mono",
            "Ubuntu Mono",
            "DejaVu Sans Mono",
            "Liberation Mono",
            "Noto Sans Mono",
        ]
    };
    candidates
        .iter()
        .find(|name| has(name))
        .map(|name| SharedString::from(*name))
        .unwrap_or_else(|| Theme::default().mono_font_family)
}
