#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
//! Modo: a fast, native workspace for reading, reviewing, and editing the code
//! your agents wrote.

mod assets;
mod brand;
#[cfg(feature = "capture")]
mod capture;
mod document;
mod files;
mod git;
mod search;
mod settings;
mod shells;
mod startup;
mod structured;
mod theme;
mod todos;
mod ui;
mod updater;

use std::{path::PathBuf, sync::OnceLock, time::Instant};

/// Process start, for `MODO_LOG_STARTUP=1` timing of the first frame.
pub static STARTED: OnceLock<Instant> = OnceLock::new();

use gpui_kit::component::{
    TitleBar,
    input::{Copy, Cut, Paste, Redo, SelectAll, Undo},
};
use gpui_kit::{
    AnyWindowHandle, App, AppContext, Bounds, Context, Entity, Menu, MenuItem, SystemMenuType,
    TitlebarOptions, Window, WindowBounds, WindowOptions, point, px, size,
};

use settings::{Session, Settings};
use ui::workspace::{self as ws, Workspace};

struct Args {
    path: Option<PathBuf>,
    #[cfg_attr(not(feature = "capture"), allow(dead_code))]
    capture: Option<(PathBuf, String)>,
}

fn parse_args() -> Option<Args> {
    let mut path = None;
    let mut capture = None;
    let mut scene = String::from("editor");
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--version" | "-v") => {
                println!("{} {}", brand::NAME, brand::VERSION);
                return None;
            }
            Some("--help" | "-h") => {
                println!(
                    "{} {}: {}\n\nUsage: {} [PATH]\n\nOpens PATH (a folder or a file) in a new window.\n\nOptions:\n  --temporary-session   Do not read or write saved settings and session\n  --version             Print the version",
                    brand::NAME,
                    brand::VERSION,
                    brand::TAGLINE,
                    brand::SLUG
                );
                return None;
            }
            Some("--temporary-session") => unsafe {
                // Single-threaded at this point; read by settings on startup.
                std::env::set_var("MODO_TEMPORARY_SESSION", "1");
            },
            Some("--capture") => capture = args.next().map(PathBuf::from),
            Some("--scene") => {
                scene = args
                    .next()
                    .and_then(|s| s.into_string().ok())
                    .unwrap_or(scene)
            }
            Some(flag) if flag.starts_with("-psn_") => {} // macOS Finder launch noise
            _ if arg.is_empty() => {}
            _ => path = Some(PathBuf::from(arg)),
        }
    }
    let path = path.map(|p| dunce::canonicalize(&p).unwrap_or(p));
    Some(Args {
        path,
        capture: capture.map(|c| (c, scene)),
    })
}

pub fn open_window(path: Option<PathBuf>, cx: &mut App) {
    open_workspace(cx, |window, cx| Workspace::new(path, window, cx));
}

/// Open the first window before it is known what goes in it. Creating a
/// window and its GPU surface takes longer than the wait for Finder's
/// "open this folder" request, so the two overlap instead of adding up.
fn open_launch_window(cx: &mut App) -> Option<(AnyWindowHandle, Entity<Workspace>)> {
    open_workspace(cx, Workspace::launching)
}

fn open_workspace(
    cx: &mut App,
    build: impl FnOnce(&mut Window, &mut Context<Workspace>) -> Workspace,
) -> Option<(AnyWindowHandle, Entity<Workspace>)> {
    let bounds = Bounds::centered(None, size(px(1320.), px(840.)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_min_size: Some(size(px(720.), px(460.))),
        titlebar: Some(TitlebarOptions {
            title: Some(brand::NAME.into()),
            appears_transparent: true,
            traffic_light_position: Some(point(px(14.), px(13.))),
        }),
        app_id: Some(brand::BUNDLE_ID.to_owned()),
        ..TitleBar::window_options()
    };
    gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| build(window, cx)))
        .map_err(|error| eprintln!("{}: cannot open a window: {error}", brand::SLUG))
        .ok()
}

fn menus() -> Vec<Menu> {
    vec![
        Menu {
            name: brand::NAME.into(),
            items: vec![
                MenuItem::action(format!("About {}", brand::NAME), ws::About),
                MenuItem::action("Check for Updates…", ws::CheckForUpdates),
                MenuItem::separator(),
                MenuItem::action("Settings…", ws::OpenSettings),
                MenuItem::separator(),
                MenuItem::os_submenu("Services", SystemMenuType::Services),
                MenuItem::separator(),
                MenuItem::action(format!("Hide {}", brand::NAME), ws::Hide),
                MenuItem::action("Hide Others", ws::HideOthers),
                MenuItem::action("Show All", ws::ShowAll),
                MenuItem::separator(),
                MenuItem::action(format!("Quit {}", brand::NAME), ws::Quit),
            ],
            disabled: false,
        },
        Menu {
            name: "File".into(),
            items: vec![
                MenuItem::action("New Window", ws::NewWindow),
                MenuItem::action("Open Folder…", ws::OpenFolder),
                MenuItem::action("Switch Project…", ws::SwitchProject),
                MenuItem::separator(),
                MenuItem::action("Save", ws::Save),
                MenuItem::action("Save All", ws::SaveAll),
                MenuItem::separator(),
                MenuItem::action("Close Tab", ws::CloseTab),
                MenuItem::action("Close All Tabs", ws::CloseAllTabs),
                MenuItem::action("Close Other Tabs", ws::CloseOtherTabs),
                MenuItem::action("Close Tabs to the Left", ws::CloseTabsLeft),
                MenuItem::action("Close Tabs to the Right", ws::CloseTabsRight),
                MenuItem::action("Close Window", ws::CloseWindow),
            ],
            disabled: false,
        },
        Menu {
            name: "Edit".into(),
            items: vec![
                MenuItem::action("Undo", Undo),
                MenuItem::action("Redo", Redo),
                MenuItem::separator(),
                MenuItem::action("Cut", Cut),
                MenuItem::action("Copy", Copy),
                MenuItem::action("Paste", Paste),
                MenuItem::action("Select All", SelectAll),
                MenuItem::separator(),
                MenuItem::action("Search in Files", ws::FindInFiles),
                MenuItem::action("Go to Line…", ws::GoToLine),
            ],
            disabled: false,
        },
        Menu {
            name: "View".into(),
            items: vec![
                MenuItem::action("Command Palette", ws::CommandPalette),
                MenuItem::action("Go to File…", ws::QuickOpen),
                MenuItem::separator(),
                MenuItem::action("Files", ws::ShowFiles),
                MenuItem::action("Review Changes", ws::ReviewChanges),
                MenuItem::action("Toggle Sidebar", ws::ToggleSidebar),
                MenuItem::action("Toggle Terminal", ws::ToggleTerminal),
                MenuItem::action("Full-window Terminal", ws::MaximizeTerminal),
                MenuItem::action("Split Terminal Side by Side", ws::SplitTerminalHorizontal),
                MenuItem::action("Split Terminal Above and Below", ws::SplitTerminalVertical),
                MenuItem::action("To-dos", ws::ShowTodos),
                MenuItem::action("Toggle Project Gradient", ws::ToggleWindowColor),
                MenuItem::action("Toggle Word Wrap", ws::ToggleSoftWrap),
                MenuItem::action("Toggle File Preview", ws::ToggleMarkdownPreview),
                MenuItem::action("Format Document", ws::FormatDocument),
                MenuItem::separator(),
                MenuItem::action("Zoom In", ws::ZoomIn),
                MenuItem::action("Zoom Out", ws::ZoomOut),
                MenuItem::action("Actual Size", ws::ZoomReset),
            ],
            disabled: false,
        },
        Menu {
            name: "Window".into(),
            items: vec![
                MenuItem::action("Minimize", ws::Minimize),
                MenuItem::action("Zoom", ws::Zoom),
            ],
            disabled: false,
        },
    ]
}

/// `file://` URLs from Finder, the Dock, or `open -a Modo <folder>`.
fn path_from_url(url: &str) -> Option<PathBuf> {
    let encoded = url.strip_prefix("file://")?;
    let bytes = encoded.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(byte) = u8::from_str_radix(&encoded[i + 1..i + 3], 16)
        {
            decoded.push(byte);
            i += 3;
        } else {
            decoded.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(decoded).ok().map(PathBuf::from)
}

/// Bring an existing window for `path` forward, or open a new one.
fn open_or_focus(path: PathBuf, cx: &mut App) {
    let path = dunce::canonicalize(&path).unwrap_or(path);
    for window in cx.windows() {
        let found = window
            .update(cx, |root, window, cx| {
                let workspace = root
                    .downcast::<gpui_kit::base::Root>()
                    .ok()
                    .and_then(|root| root.read(cx).view().clone().downcast::<Workspace>().ok())?;
                let matches = workspace
                    .read(cx)
                    .root()
                    .is_some_and(|root| path.starts_with(root));
                if matches {
                    window.activate_window();
                    workspace.update(cx, |ws, cx| ws.open_path(path.clone(), window, cx));
                }
                matches.then_some(())
            })
            .ok()
            .flatten();
        if found.is_some() {
            return;
        }
    }
    open_window(Some(path), cx);
}

/// Running `modo .` from a Linux terminal should hand the prompt back.
#[cfg(target_os = "linux")]
fn detach_from_terminal() -> bool {
    use std::{io::IsTerminal, os::unix::process::CommandExt, process::Stdio};
    if !std::io::stdin().is_terminal()
        || std::env::var_os("MODO_NO_DETACH").is_some()
        || std::env::var_os("CARGO").is_some()
    {
        return false;
    }
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    std::process::Command::new(exe)
        .args(std::env::args_os().skip(1))
        .env("MODO_NO_DETACH", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .is_ok()
}

fn main() {
    STARTED.get_or_init(Instant::now);
    let Some(args) = parse_args() else { return };
    #[cfg(target_os = "linux")]
    if args.capture.is_none() && detach_from_terminal() {
        return;
    }
    let app = gpui_kit::application().with_assets(assets::Assets);
    app.on_reopen(|cx| {
        if cx.windows().is_empty() {
            open_window(None, cx);
        }
    });
    let (open_tx, mut open_rx) = futures::channel::mpsc::unbounded::<PathBuf>();
    app.on_open_urls(move |urls| {
        for path in urls.iter().filter_map(|url| path_from_url(url)) {
            let _ = open_tx.unbounded_send(path);
        }
    });
    app.run(move |cx| {
        gpui_kit::init(cx);
        cx.set_global(Settings::load());
        cx.set_global(Session::load());
        theme::init(cx);
        ui::workspace::init(cx);
        ui::terminal::init(cx);
        ui::palette::init(cx);
        updater::init(cx);
        cx.set_menus(menus());
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() && !cfg!(target_os = "macos") {
                cx.quit();
            }
        })
        .detach();
        // Reopen the last project that still exists when launched without a path.
        let recent = if args.path.is_none() && Settings::get(cx).restore_session {
            Session::get(cx).recent.clone()
        } else {
            Vec::new()
        };
        Session::prune_missing(cx);
        #[cfg(feature = "capture")]
        if let Some((output, scene)) = args.capture.clone() {
            capture::run(args.path.clone(), output, scene, cx);
            return;
        }
        // Finder may launch us to open a folder; give that request a moment to
        // arrive so we do not also restore the previous project. The window
        // is created meanwhile and filled in once the choice is made.
        let explicit = args.path.clone();
        let launch = open_launch_window(cx);
        cx.spawn(async move |cx| {
            use futures::StreamExt as _;
            let (first, previous) = if explicit.is_some() {
                (None, None)
            } else {
                let previous = cx
                    .background_executor()
                    .spawn(async move { recent.into_iter().find(|path| path.is_dir()) });
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(60))
                    .await;
                (open_rx.try_recv().ok(), previous.await)
            };
            cx.update(|cx| {
                let target = first.or(explicit).or(previous);
                let filled = launch.is_some_and(|(window, workspace)| {
                    window
                        .update(cx, |_, window, cx| {
                            workspace.update(cx, |workspace, cx| {
                                workspace.finish_launch(target.clone(), window, cx)
                            })
                        })
                        .is_ok()
                });
                if !filled {
                    open_window(target, cx);
                }
                cx.activate(true);
            });
            while let Some(requested) = open_rx.next().await {
                cx.update(|cx| {
                    open_or_focus(requested, cx);
                    cx.activate(true);
                });
            }
        })
        .detach();
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn file_urls_decode_to_paths() {
        assert_eq!(
            super::path_from_url("file:///Users/me/My%20Project/"),
            Some(std::path::PathBuf::from("/Users/me/My Project/"))
        );
        assert_eq!(super::path_from_url("https://example.com"), None);
    }
}
