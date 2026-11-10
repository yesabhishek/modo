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
        ThemeChoice::Midnight => ("Modo Light", "Modo Midnight", ThemeMode::Dark),
        ThemeChoice::Black => ("Modo Light", "Modo Black", ThemeMode::Dark),
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

/// Whether a family is installed, asked of the text system one name at a time.
///
/// Listing every installed font takes about a tenth of a second on macOS,
/// which is too much for launch or for a theme switch. Resolving one name is
/// cheap and cached: a family that is missing resolves to a fallback, which
/// then reports a different name.
fn is_installed(family: &str, cx: &App) -> bool {
    let text_system = cx.text_system();
    let id = text_system.resolve_font(&gpui_kit::font(family.to_owned()));
    text_system
        .get_font_for_id(id)
        .is_some_and(|font| font.family.eq_ignore_ascii_case(family))
}

/// The user's family if installed, else the best monospace face on this platform.
pub fn mono_font(preferred: &str, cx: &App) -> SharedString {
    let preferred = preferred.trim();
    if !preferred.is_empty() && is_installed(preferred, cx) {
        return preferred.to_owned().into();
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
        .find(|name| is_installed(name, cx))
        .map(|name| SharedString::from(*name))
        .unwrap_or_else(|| Theme::default().mono_font_family)
}

/// Installed monospace families, for the font menu in Settings.
///
/// Reads every font's header, so call it off the UI thread. The scan is not
/// kept: holding all system fonts open would cost memory for a menu that is
/// rarely opened. `installed` is the text system's own list, so every name
/// returned is one the editor can actually use.
pub fn monospace_families(installed: &[String]) -> Vec<String> {
    let mut db = resvg::usvg::fontdb::Database::new();
    db.load_system_fonts();
    let mut families: Vec<String> = db
        .faces()
        .filter(|face| face.monospaced)
        .filter_map(|face| face.families.first().map(|(name, _)| name.clone()))
        // Hidden system faces start with a dot; bitmap faces do not scale.
        .filter(|name| !name.starts_with('.') && !name.contains("Bitmap"))
        .filter(|name| installed.iter().any(|font| font.eq_ignore_ascii_case(name)))
        .collect();
    families.sort_by_key(|name| name.to_lowercase());
    families.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    families
}
