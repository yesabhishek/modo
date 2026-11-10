//! Offscreen frame capture for docs and visual review, compiled only with the
//! `capture` feature.
//!
//! `modo --capture out.png --scene review <folder>` saves one frame.
//! `modo --capture frames/ --scene tour <folder>` drives a scripted walkthrough
//! and saves numbered frames plus `frames.txt` (an ffmpeg concat list).
use std::{path::PathBuf, time::Duration};

use gpui_kit::component::TitleBar;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, App, AppContext, AsyncApp, Bounds, Context, Modifiers, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, PlatformInput, TitlebarOptions, Window,
    WindowBounds, WindowOptions, point, px, size,
};

use crate::{
    settings::{ReviewLayout, Settings, ThemeChoice},
    ui::workspace::{Panel, Tab, Workspace},
};

type Action = Box<dyn FnOnce(&mut Workspace, &mut Window, &mut Context<Workspace>)>;

enum Step {
    Do(Action),
    /// Click at a point in window coordinates, as the pointer would.
    Click(f32, f32),
    /// Turn the wheel by `dy` pixels over a point in window coordinates.
    Scroll(f32, f32, f32),
    Key(&'static str),
    Input(&'static str),
    Wait(u64),
    /// Save a frame that the animation holds for this many milliseconds.
    Shot(u64),
}

fn act(f: impl FnOnce(&mut Workspace, &mut Window, &mut Context<Workspace>) + 'static) -> Step {
    Step::Do(Box::new(f))
}

fn click(window: &mut Window, x: f32, y: f32, cx: &mut App) {
    let position = point(px(x), px(y));
    let modifiers = Modifiers::default();
    let button = if std::env::var("MODO_CAPTURE_BUTTON").is_ok_and(|button| button == "right") {
        MouseButton::Right
    } else {
        MouseButton::Left
    };
    // Move first so hover and hit testing see the pointer where it presses.
    window.dispatch_event(
        PlatformInput::MouseMove(MouseMoveEvent {
            position,
            pressed_button: None,
            modifiers,
        }),
        cx,
    );
    window.draw(cx).clear(cx);
    window.dispatch_event(
        PlatformInput::MouseDown(MouseDownEvent {
            button,
            position,
            modifiers,
            click_count: 1,
            first_mouse: false,
        }),
        cx,
    );
    window.dispatch_event(
        PlatformInput::MouseUp(MouseUpEvent {
            button,
            position,
            modifiers,
            click_count: 1,
        }),
        cx,
    );
}

fn theme_from_env() -> Option<ThemeChoice> {
    Some(match std::env::var("MODO_CAPTURE_THEME").ok()?.as_str() {
        "light" => ThemeChoice::Light,
        "dark" => ThemeChoice::Dark,
        "paper" => ThemeChoice::Paper,
        "graphite" => ThemeChoice::Graphite,
        "midnight" => ThemeChoice::Midnight,
        "black" => ThemeChoice::Black,
        _ => ThemeChoice::System,
    })
}

/// Exercises file creation and saving through the mounted workspace in a disposable project.
fn editing_smoke() -> Vec<Step> {
    vec![
        Step::Wait(900),
        act(|ws, window, cx| {
            let root = ws.root().unwrap().to_path_buf();
            assert!(
                !root.join("smoke.txt").exists(),
                "Use an empty disposable project"
            );
            Settings::update(cx, |s| {
                s.auto_save = true;
                s.watch_files = false;
            });
            ws.start_new_file(root, window, cx);
        }),
        Step::Input("smoke.txt"),
        Step::Key("enter"),
        Step::Wait(800),
        act(|ws, window, cx| {
            let file = ws.active_file().expect("created file opens in editor");
            assert_eq!(std::fs::read_to_string(&file.path).unwrap(), "");
            assert!(ws.tree.rows.iter().any(|row| row.path == file.path));
            file.editor
                .update(cx, |editor, cx| editor.insert("first edit", window, cx));
        }),
        Step::Wait(100),
        act(|ws, window, cx| {
            let file = ws.active_file().unwrap();
            assert!(file.dirty);
            assert_eq!(std::fs::read_to_string(&file.path).unwrap(), "");
            file.editor
                .update(cx, |editor, cx| editor.insert(" and more", window, cx));
        }),
        Step::Wait(1100),
        act(|ws, window, cx| {
            let file = ws.active_file().unwrap();
            assert!(!file.dirty);
            assert_eq!(
                std::fs::read_to_string(&file.path).unwrap(),
                "first edit and more"
            );
            Settings::update(cx, |s| s.auto_save = false);
            ws.active_file()
                .unwrap()
                .editor
                .update(cx, |editor, cx| editor.insert(" manual", window, cx));
        }),
        Step::Wait(1100),
        act(|ws, window, cx| {
            let file = ws.active_file().unwrap();
            assert!(file.dirty);
            assert_eq!(
                std::fs::read_to_string(&file.path).unwrap(),
                "first edit and more"
            );
            ws.save_tab(ws.active.unwrap(), false, window, cx);
        }),
        Step::Wait(400),
        act(|ws, window, cx| {
            let file = ws.active_file().unwrap();
            assert!(!file.dirty);
            assert_eq!(
                std::fs::read_to_string(&file.path).unwrap(),
                "first edit and more manual"
            );
            Settings::update(cx, |s| s.auto_save = true);
            let file = ws.active_file().unwrap();
            file.editor
                .update(cx, |editor, cx| editor.insert(" conflict", window, cx));
            std::fs::write(&file.path, "external edit").unwrap();
        }),
        Step::Wait(1100),
        act(|ws, _, cx| {
            let file = ws.active_file().unwrap();
            assert!(file.dirty);
            assert_eq!(
                std::fs::read_to_string(&file.path).unwrap(),
                "external edit"
            );
            assert!(!file.auto_save_ready(), "failed revision should not retry");
            println!(
                "editing smoke: creation, debounce, autosave, toggle, manual save, and conflict protection passed"
            );
            cx.notify();
        }),
        act(|ws, window, cx| ws.start_new_file(ws.root().unwrap().to_path_buf(), window, cx)),
        Step::Input("smoke.txt"),
        Step::Key("enter"),
        Step::Wait(400),
        act(|ws, _, _| {
            assert!(ws.new_file.as_ref().unwrap().error.is_some());
            assert_eq!(
                std::fs::read_to_string(ws.root().unwrap().join("smoke.txt")).unwrap(),
                "external edit"
            );
        }),
        Step::Key("escape"),
        act(|ws, _, _| assert!(ws.new_file.is_none(), "Escape cancels file creation")),
        Step::Shot(1000),
    ]
}

/// Uses disposable fixtures to check native search, formatting, undo, and autosave.
fn data_smoke() -> Vec<Step> {
    const JSON: &str =
        r#"{"outer":{"needle":"café"},"precise":1.234567890123456789,"duplicate":1,"duplicate":2}"#;
    let find = if crate::ui::is_mac() {
        "cmd-f"
    } else {
        "ctrl-f"
    };
    let undo = if crate::ui::is_mac() {
        "cmd-z"
    } else {
        "ctrl-z"
    };
    vec![
        Step::Wait(900),
        act(|ws, window, cx| {
            Settings::update(cx, |s| {
                s.auto_save = false;
                s.watch_files = false;
            });
            let path = crate::document::create(ws.root().unwrap(), "data-smoke.json").unwrap();
            std::fs::write(&path, JSON).unwrap();
            ws.open_file(path, None, window, cx);
        }),
        Step::Wait(800),
        act(|ws, _, cx| {
            assert_eq!(
                ws.active_file().unwrap().capture_data_state(cx),
                Some((5, false, None))
            );
        }),
        Step::Key(find),
        Step::Input("CAFÉ"),
        Step::Wait(400),
        act(|ws, window, cx| {
            assert_eq!(
                ws.active_file().unwrap().capture_data_state(cx),
                Some((3, false, None))
            );
            ws.format_document(window, cx);
        }),
        Step::Wait(500),
        act(|ws, window, cx| {
            let ix = ws.active.unwrap();
            let Tab::File(file) = &mut ws.tabs[ix] else {
                panic!()
            };
            let text = file.editor.read(cx).value();
            assert!(file.dirty);
            assert!(text.contains("\n"));
            assert!(text.contains("1.234567890123456789"));
            assert_eq!(text.matches("\"duplicate\":").count(), 2);
            assert_eq!(file.capture_data_state(cx), Some((3, false, None)));
            file.toggle_preview(window, cx);
        }),
        Step::Key(undo),
        Step::Wait(100),
        act(|ws, window, cx| {
            assert_eq!(
                ws.active_file().unwrap().editor.read(cx).value().as_ref(),
                JSON
            );
            Settings::update(cx, |s| {
                s.auto_save = true;
                s.format_on_save_trim = true;
            });
            let path = crate::document::create(ws.root().unwrap(), "data-smoke.csv").unwrap();
            std::fs::write(
                &path,
                "\"name\",\"value\"\nAda,\"hello,\nworld\"\nLin, padded  \n",
            )
            .unwrap();
            ws.open_file(path, None, window, cx);
        }),
        Step::Wait(800),
        act(|ws, _, cx| {
            assert_eq!(
                ws.active_file().unwrap().capture_data_state(cx),
                Some((2, false, None))
            )
        }),
        Step::Key(find),
        Step::Input("world"),
        Step::Wait(400),
        act(|ws, window, cx| {
            assert_eq!(
                ws.active_file().unwrap().capture_data_state(cx),
                Some((1, false, None))
            );
            ws.format_document(window, cx);
        }),
        Step::Wait(1200),
        act(|ws, _, cx| {
            let file = ws.active_file().unwrap();
            assert!(!file.dirty);
            let disk = std::fs::read_to_string(&file.path).unwrap();
            assert_eq!(disk, "name,value\nAda,\"hello,\nworld\"\nLin, padded  \n");
            assert_eq!(file.capture_data_state(cx), Some((1, false, None)));
            println!(
                "data smoke: JSON tree search, lossless format, undo, CSV search, formatting, and autosave passed"
            );
        }),
        Step::Shot(1000),
    ]
}

fn save_race_smoke() -> Vec<Step> {
    let result = std::rc::Rc::new(std::cell::RefCell::new(None));
    let save_result = result.clone();
    vec![
        Step::Wait(900),
        act(|ws, window, cx| {
            Settings::update(cx, |s| {
                s.auto_save = false;
                s.watch_files = false;
            });
            let path = crate::document::create(ws.root().unwrap(), "save-race.txt").unwrap();
            ws.open_file(path, None, window, cx);
        }),
        Step::Wait(500),
        Step::Input("snapshot"),
        Step::Wait(100),
        act(move |ws, _, cx| {
            let Tab::File(file) = &mut ws.tabs[ws.active.unwrap()] else {
                panic!()
            };
            *save_result.borrow_mut() = Some(file.begin_save(cx).unwrap().run());
        }),
        Step::Input(" + newer edit"),
        Step::Wait(100),
        act(move |ws, window, cx| {
            let Tab::File(file) = &mut ws.tabs[ws.active.unwrap()] else {
                panic!()
            };
            file.finish_save(result.borrow_mut().take().unwrap(), cx)
                .unwrap();
            assert!(
                file.dirty,
                "a completed old snapshot must not clear newer edits"
            );
            assert_eq!(std::fs::read_to_string(&file.path).unwrap(), "snapshot");
            Settings::update(cx, |s| s.auto_save = true);
            let other = crate::document::create(ws.root().unwrap(), "other.txt").unwrap();
            ws.open_file(other, None, window, cx);
        }),
        Step::Wait(1200),
        act(|ws, _, _| {
            let path = ws.root().unwrap().join("save-race.txt");
            let ix = ws.tab_for_path(&path).unwrap();
            assert_ne!(ws.active, Some(ix));
            let Tab::File(file) = &ws.tabs[ix] else {
                panic!()
            };
            assert!(!file.dirty);
            assert_eq!(
                std::fs::read_to_string(path).unwrap(),
                "snapshot + newer edit"
            );
            println!(
                "save race smoke: newer edits survive an in-flight save and background tabs autosave"
            );
        }),
        Step::Shot(1000),
    ]
}

fn loaded_tabs(ws: &Workspace) -> usize {
    ws.tabs
        .iter()
        .filter(|tab| matches!(tab, Tab::File(_)))
        .count()
}

/// Opens more files than stay mounted, in a disposable project, and checks
/// that idle tabs give up their editor, come back when shown, and that a tab
/// with unsaved edits is never unloaded.
fn tabs_smoke() -> Vec<Step> {
    const FILES: usize = crate::ui::workspace::LOADED_TABS + 8;
    let mut steps = vec![
        Step::Wait(900),
        act(|ws, _, cx| {
            Settings::update(cx, |s| {
                s.auto_save = false;
                s.watch_files = false;
            });
            let root = ws.root().unwrap().to_path_buf();
            for ix in 0..FILES {
                let name = format!("tab-{ix:02}.txt");
                assert!(
                    !root.join(&name).exists(),
                    "Use an empty disposable project"
                );
                std::fs::write(root.join(name), format!("file {ix}\nsecond line\n")).unwrap();
            }
        }),
    ];
    for ix in 0..FILES {
        steps.push(act(move |ws, window, cx| {
            let path = ws.root().unwrap().join(format!("tab-{ix:02}.txt"));
            ws.open_file(path, None, window, cx);
        }));
        steps.push(Step::Wait(120));
        if ix == 0 {
            // Unsaved work in the first tab: it must stay mounted throughout.
            steps.push(Step::Input("unsaved "));
            steps.push(Step::Wait(60));
        }
    }
    steps.extend([
        act(|ws, window, cx| {
            assert_eq!(ws.tabs.len(), FILES);
            assert_eq!(
                loaded_tabs(ws),
                crate::ui::workspace::LOADED_TABS,
                "idle tabs beyond the limit are unloaded"
            );
            let Tab::File(first) = &ws.tabs[0] else {
                panic!("a tab with unsaved edits was unloaded")
            };
            assert!(first.dirty);
            assert!(first.editor.read(cx).value().starts_with("unsaved "));
            assert!(matches!(ws.tabs[1], Tab::Pending(_)));
            // Showing an unloaded tab reads its file again.
            ws.activate(1, window, cx);
        }),
        Step::Wait(400),
        act(|ws, _, cx| {
            let Tab::File(second) = &ws.tabs[1] else {
                panic!("an unloaded tab did not come back when shown")
            };
            assert_eq!(
                second.editor.read(cx).value().as_ref(),
                "file 1\nsecond line\n"
            );
            assert_eq!(ws.active, Some(1));
            assert!(loaded_tabs(ws) <= crate::ui::workspace::LOADED_TABS);
            assert_eq!(ws.tabs.len(), FILES);
            println!(
                "tabs smoke: idle tabs unload, return when shown, and unsaved tabs stay mounted"
            );
        }),
        Step::Shot(0),
    ]);
    steps
}

/// An agent edits an open file on disk: a clean tab follows it, a tab with
/// unsaved edits keeps them and shows the conflict banner, and Modo's own
/// saves are not mistaken for outside edits.
fn follow_smoke() -> Vec<Step> {
    vec![
        Step::Wait(900),
        act(|ws, window, cx| {
            Settings::update(cx, |s| {
                s.auto_save = true;
                s.watch_files = true;
            });
            let path = crate::document::create(ws.root().unwrap(), "follow.txt").unwrap();
            std::fs::write(&path, "first").unwrap();
            ws.open_file(path, None, window, cx);
        }),
        Step::Wait(1200),
        act(|ws, _, cx| {
            let file = ws.active_file().expect("the file opened");
            assert_eq!(file.editor.read(cx).value().as_ref(), "first");
            std::fs::write(&file.path, "second, written by an agent").unwrap();
        }),
        Step::Wait(1200),
        act(|ws, window, cx| {
            let file = ws.active_file().unwrap();
            assert_eq!(
                file.editor.read(cx).value().as_ref(),
                "second, written by an agent",
                "a clean tab follows the file on disk"
            );
            assert!(!file.dirty && !file.disk_changed);
            // Modo's own save must not come back as an outside change.
            file.editor
                .update(cx, |editor, cx| editor.insert("mine: ", window, cx));
        }),
        Step::Wait(1500),
        act(|ws, window, cx| {
            let file = ws.active_file().unwrap();
            let text = file.editor.read(cx).value().to_string();
            assert!(text.contains("mine: "), "own save kept the edit: {text:?}");
            assert!(
                !file.dirty && !file.disk_changed,
                "own save is not a conflict"
            );
            assert_eq!(std::fs::read_to_string(&file.path).unwrap(), text);
            // Now a real conflict: unsaved edits here, a different file there.
            Settings::update(cx, |s| s.auto_save = false);
            file.editor
                .update(cx, |editor, cx| editor.insert("unsaved ", window, cx));
            std::fs::write(&file.path, "third, from outside").unwrap();
        }),
        Step::Wait(1200),
        act(|ws, _, cx| {
            let file = ws.active_file().unwrap();
            assert!(file.dirty && file.disk_changed, "the conflict is flagged");
            assert!(file.editor.read(cx).value().contains("unsaved "));
            assert_eq!(
                std::fs::read_to_string(&file.path).unwrap(),
                "third, from outside"
            );
            println!(
                "follow smoke: clean tabs follow the disk, own saves are quiet, conflicts are kept"
            );
        }),
        Step::Shot(0),
    ]
}

/// With a saved session (`MODO_CONFIG_DIR`), checks that launch mounts only
/// the tab that was active and that the others load when first shown.
fn restore_smoke() -> Vec<Step> {
    vec![
        Step::Wait(900),
        act(|ws, window, cx| {
            assert!(ws.tabs.len() >= 3, "the session's tabs are all in the bar");
            assert_eq!(loaded_tabs(ws), 1, "only the active tab is read at launch");
            let active = ws.active.expect("a tab is active");
            assert!(matches!(ws.tabs[active], Tab::File(_)));
            let other = (0..ws.tabs.len()).find(|ix| *ix != active).unwrap();
            ws.activate(other, window, cx);
        }),
        Step::Wait(400),
        act(|ws, _, _| {
            assert_eq!(loaded_tabs(ws), 2);
            assert!(matches!(ws.tabs[ws.active.unwrap()], Tab::File(_)));
            println!("restore smoke: launch reads one file; the rest load when shown");
        }),
        Step::Shot(0),
    ]
}

fn single(scene: &str, file: Option<PathBuf>) -> Vec<Step> {
    let want_preview = scene == "preview";
    let want_reviewed = scene == "review-progress";
    let scene = scene.to_owned();
    let settle = std::env::var("MODO_CAPTURE_DELAY_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1600);
    let mut steps = vec![
        Step::Wait(900),
        act(move |ws, window, cx| {
            match scene.as_str() {
                "review" | "review-split" | "review-progress" => {
                    if scene == "review-split" {
                        Settings::update(cx, |s| s.review_layout = ReviewLayout::Split);
                    }
                    ws.panel = Panel::Changes;
                    ws.open_review(window, cx);
                }
                "todos" => {
                    ws.panel = Panel::Todos;
                    ws.start_todos(cx);
                }
                "terminal-split" => {
                    ws.terminal.update(cx, |terminal, cx| {
                        terminal.split(crate::ui::terminal::SplitAxis::Horizontal, window, cx);
                        terminal.split(crate::ui::terminal::SplitAxis::Vertical, window, cx);
                    });
                }
                "project-gradient-settings" => {
                    ws.window_color = Some(true);
                    ws.open_settings(window, cx);
                }
                "project-gradient-terminal" => {
                    ws.window_color = Some(true);
                    ws.terminal
                        .update(cx, |terminal, cx| terminal.show(window, cx));
                    cx.notify();
                }
                "terminal-full" => ws
                    .terminal
                    .update(cx, |terminal, cx| terminal.toggle_maximized(window, cx)),
                "window-color" | "project-gradient" => {
                    ws.window_color = Some(true);
                    cx.notify();
                }
                "palette" => ws.open_palette("", window, cx),
                "commands" => ws.open_palette(">", window, cx),
                "about" => crate::ui::about::open(window, cx),
                "settings-about" => {
                    ws.open_settings(window, cx);
                    if let Some(view) = ws.settings.clone() {
                        view.update(cx, |view, cx| view.show_about(cx));
                    }
                }
                "settings" => ws.open_settings(window, cx),
                "settings-editor" => {
                    ws.open_settings(window, cx);
                    if let Some(view) = ws.settings.clone() {
                        view.update(cx, |view, cx| view.show_editor(cx));
                    }
                }
                "new-file" => {
                    if let Some(root) = ws.root().map(std::path::Path::to_path_buf) {
                        ws.start_new_file(root, window, cx);
                    }
                }
                "settings-terminal" => {
                    ws.open_settings(window, cx);
                    if let Some(view) = ws.settings.clone() {
                        view.update(cx, |view, cx| view.show_terminal(cx));
                    }
                }
                "search" => {
                    ws.panel = Panel::Search;
                    let query =
                        std::env::var("MODO_CAPTURE_QUERY").unwrap_or_else(|_| "bucket".into());
                    ws.search
                        .update(cx, |s, cx| s.focus(Some(query), window, cx));
                }
                _ => {}
            }
            if let Some(recent) = std::env::var_os("MODO_CAPTURE_RECENT") {
                let mut session = crate::settings::Session::get(cx).clone();
                session.recent = std::env::split_paths(&recent).collect();
                cx.set_global(session);
            }
            // `MODO_CAPTURE_UPDATE=available|downloading|ready` shows an update state.
            if let Ok(state) = std::env::var("MODO_CAPTURE_UPDATE") {
                use crate::updater::{Phase, UpdateState};
                let version = "9.9.9".to_owned();
                let phase = match state.as_str() {
                    "available" => Phase::Available {
                        version,
                        url: crate::brand::REPOSITORY.to_owned(),
                        installable: true,
                    },
                    "downloading" => Phase::Downloading {
                        version,
                        percent: Some(42),
                    },
                    "ready" => Phase::Ready {
                        version: Some(version),
                    },
                    _ => Phase::UpToDate,
                };
                cx.set_global(UpdateState { phase });
            }
            if std::env::var_os("MODO_CAPTURE_TERMINAL").is_some() {
                ws.terminal.update(cx, |t, cx| t.show(window, cx));
            }
        }),
        Step::Wait(settle),
    ];
    // `MODO_CAPTURE_OPEN_DIR=dir` opens every file in a folder, to fill the tab bar.
    if let Some(dir) = std::env::var_os("MODO_CAPTURE_OPEN_DIR") {
        let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
            .map(|entries| entries.flatten().map(|entry| entry.path()).collect())
            .unwrap_or_default();
        paths.retain(|path| path.is_file());
        paths.sort();
        for (ix, path) in paths.into_iter().enumerate() {
            steps.insert(
                1 + ix * 2,
                act(move |ws, window, cx| ws.open_file(path, None, window, cx)),
            );
            steps.insert(2 + ix * 2, Step::Wait(120));
        }
    }
    if let Some(file) = file {
        steps.insert(
            1,
            act(move |ws, window, cx| ws.open_file(file, None, window, cx)),
        );
        steps.insert(2, Step::Wait(400));
    }
    if want_reviewed {
        steps.extend([
            act(|ws, _, cx| {
                let path = ws
                    .review
                    .read(cx)
                    .files()
                    .first()
                    .map(|file| file.path.clone());
                if let Some(path) = path {
                    ws.review
                        .update(cx, |review, cx| review.toggle_reviewed(&path, cx));
                }
            }),
            Step::Wait(400),
        ]);
    }
    // `MODO_CAPTURE_SCROLL=x,y,dy` turns the wheel over a point before the shot.
    if let Some(at) = std::env::var("MODO_CAPTURE_SCROLL").ok().and_then(|at| {
        let mut parts = at.split(',').map(|part| part.trim().parse::<f32>().ok());
        Some((parts.next()??, parts.next()??, parts.next()??))
    }) {
        steps.extend([Step::Scroll(at.0, at.1, at.2), Step::Wait(300)]);
    }
    // `MODO_CAPTURE_CLICK=x,y` clicks a control, such as a menu, before the shot.
    if let Some((x, y)) = std::env::var("MODO_CAPTURE_CLICK").ok().and_then(|at| {
        let (x, y) = at.split_once(',')?;
        Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
    }) {
        steps.extend([Step::Click(x, y), Step::Wait(600)]);
    }
    steps.push(Step::Shot(0));
    if want_preview {
        // The file loads in the background; switch once its tab exists.
        steps.insert(
            2,
            act(|ws, window, cx| {
                if let Some(ix) = ws.active
                    && let Some(Tab::File(file)) = ws.tabs.get_mut(ix)
                    && !file.preview
                {
                    file.toggle_preview(window, cx);
                }
            }),
        );
        steps.insert(2, Step::Wait(800));
    }
    steps
}

/// A walkthrough of the review loop: find a file, review the branch, switch
/// layouts and themes, run a command, search.
fn tour(root: PathBuf) -> Vec<Step> {
    let file = root.join("src/api/mod.rs");
    let mut steps = vec![
        Step::Wait(900),
        act(move |ws, window, cx| ws.open_file(file, None, window, cx)),
        Step::Wait(900),
        Step::Shot(1500),
        act(|ws, window, cx| ws.open_palette("", window, cx)),
        Step::Wait(300),
        Step::Shot(500),
    ];
    for query in ["l", "li", "lim", "limi"] {
        steps.push(act(move |ws, window, cx| {
            if let Some(palette) = ws.palette.clone() {
                palette.update(cx, |p, cx| p.set_query(query, window, cx));
            }
        }));
        steps.push(Step::Wait(120));
        steps.push(Step::Shot(if query == "limi" { 900 } else { 180 }));
    }
    steps.extend([
        act(|ws, _, cx| {
            if let Some(palette) = ws.palette.clone() {
                palette.update(cx, |p, cx| p.confirm(cx));
            }
        }),
        Step::Wait(700),
        Step::Shot(1400),
        act(|ws, window, cx| {
            ws.panel = Panel::Changes;
            ws.open_review(window, cx);
        }),
        Step::Wait(1400),
        Step::Shot(2200),
        act(|ws, _, cx| ws.review.update(cx, |r, cx| r.next_change(cx))),
        Step::Wait(400),
        Step::Shot(1000),
        act(|ws, _, cx| ws.review.update(cx, |r, cx| r.next_change(cx))),
        Step::Wait(400),
        Step::Shot(1000),
        act(|ws, _, cx| {
            Settings::update(cx, |s| s.review_layout = ReviewLayout::Split);
            ws.review
                .update(cx, |r, cx| r.scroll_to_file("src/api/mod.rs", cx));
        }),
        Step::Wait(700),
        Step::Shot(2000),
        act(|_, _, cx| {
            Settings::update(cx, |s| {
                s.review_layout = ReviewLayout::Unified;
                s.theme = ThemeChoice::Dark;
            })
        }),
        Step::Wait(700),
        Step::Shot(1600),
        act(|ws, window, cx| ws.terminal.update(cx, |t, cx| t.show(window, cx))),
        Step::Wait(1500),
        act(|ws, _, cx| {
            ws.terminal.update(cx, |t, cx| {
                t.send_text("git --no-pager log --oneline --graph --all\r", cx)
            })
        }),
        Step::Wait(1200),
        Step::Shot(2000),
        act(|ws, window, cx| {
            ws.panel = Panel::Search;
            ws.search
                .update(cx, |s, cx| s.focus(Some("bucket".into()), window, cx));
        }),
        Step::Wait(1200),
        Step::Shot(2200),
        act(|_, _, cx| Settings::update(cx, |s| s.theme = ThemeChoice::Paper)),
        Step::Wait(700),
        Step::Shot(1600),
        act(|ws, window, cx| ws.open_settings(window, cx)),
        Step::Wait(700),
        Step::Shot(2200),
    ]);
    steps
}

/// An agent edits the open file on disk; the editor, tree, and review follow.
fn live(file: PathBuf) -> Vec<Step> {
    let edited = file.clone();
    vec![
        Step::Wait(900),
        act(move |ws, window, cx| ws.open_file(file, None, window, cx)),
        Step::Wait(900),
        act(move |_, _, _| {
            let mut text = std::fs::read_to_string(&edited).unwrap_or_default();
            text.push_str("\n// Edited on disk while Modo was open.\n");
            let _ = std::fs::write(&edited, text);
        }),
        Step::Wait(1500),
        act(|ws, window, cx| {
            ws.panel = Panel::Changes;
            ws.open_review(window, cx);
        }),
        Step::Wait(1200),
        Step::Shot(0),
    ]
}

fn save_frame(handle: AnyWindowHandle, path: &PathBuf, cx: &mut AsyncApp) -> bool {
    // A window behind others may not be redrawn by the display link; draw now.
    // Draw a few frames, as the display link would, so anything that asked for
    // a follow-up frame (measured layouts) settles before the capture.
    let _ = handle.update(cx, |_, window, cx| {
        window.activate_window();
        for _ in 0..3 {
            window.draw(cx).clear(cx);
        }
    });
    handle
        .update(cx, |_, window, _| match window.render_to_image() {
            Ok(image) => image
                .save(path)
                .map_err(|e| eprintln!("capture: {e}"))
                .is_ok(),
            Err(e) => {
                eprintln!("capture: {e}");
                false
            }
        })
        .unwrap_or(false)
}

pub fn run(path: Option<PathBuf>, output: PathBuf, scene: String, cx: &mut App) {
    if let Some(theme) = theme_from_env() {
        Settings::update(cx, |s| s.theme = theme);
    }
    // Docs captures use a neutral shell instead of the user's own profile.
    if let Ok(shell) = std::env::var("MODO_CAPTURE_SHELL") {
        Settings::update(cx, |s| s.terminal_shell = shell);
    }
    // Settings observers activate after this startup cycle, so apply directly.
    crate::theme::apply(None, cx);
    let (width, height) = std::env::var("MODO_CAPTURE_SIZE")
        .ok()
        .and_then(|value| {
            let (width, height) = value.split_once('x')?;
            Some((width.parse::<f32>().ok()?, height.parse::<f32>().ok()?))
        })
        .unwrap_or((1320., 840.));
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(width), px(height)),
            cx,
        ))),
        titlebar: Some(TitlebarOptions {
            title: Some(crate::brand::NAME.into()),
            appears_transparent: true,
            traffic_light_position: Some(point(px(14.), px(13.))),
        }),
        ..TitleBar::window_options()
    };
    let root = path.clone();
    let opened = gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| Workspace::new(path, window, cx))
    });
    let Ok((handle, workspace)) = opened else {
        eprintln!("capture: cannot open window");
        cx.quit();
        return;
    };
    let file = std::env::var("MODO_CAPTURE_FILE").ok().map(PathBuf::from);
    let many = scene == "tour";
    let steps = match (many, root) {
        (true, Some(root)) => tour(root),
        _ if scene == "editing-smoke" => editing_smoke(),
        _ if scene == "data-smoke" => data_smoke(),
        _ if scene == "save-race-smoke" => save_race_smoke(),
        _ if scene == "tabs-smoke" => tabs_smoke(),
        _ if scene == "follow-smoke" => follow_smoke(),
        _ if scene == "restore-smoke" => restore_smoke(),
        _ if scene == "live" => live(file.expect("MODO_CAPTURE_FILE")),
        _ => single(&scene, file),
    };
    cx.spawn(async move |cx| {
        if many {
            let _ = std::fs::create_dir_all(&output);
        }
        let mut list = String::new();
        let mut index = 0;
        for step in steps {
            match step {
                Step::Wait(ms) => {
                    // Offscreen windows have no display link. Pump frames while
                    // async loads settle, including child views mounted by a load.
                    let mut remaining = ms;
                    while remaining > 0 {
                        let delay = remaining.min(100);
                        cx.background_executor()
                            .timer(Duration::from_millis(delay))
                            .await;
                        let _ = handle.update(cx, |_, window, cx| window.draw(cx).clear(cx));
                        remaining -= delay;
                    }
                }
                Step::Key(key) => {
                    let _ = handle.update(cx, |_, window, cx| window.press(key, cx));
                }
                Step::Input(text) => {
                    let _ = handle.update(cx, |_, window, cx| window.input(text, cx));
                }
                Step::Click(x, y) => {
                    let _ = handle.update(cx, |_, window, cx| click(window, x, y, cx));
                    let _ = handle.update(cx, |_, window, cx| window.draw(cx).clear(cx));
                }
                Step::Scroll(x, y, dy) => {
                    let _ = handle.update(cx, |_, window, cx| {
                        let position = point(px(x), px(y));
                        window.dispatch_event(
                            PlatformInput::MouseMove(MouseMoveEvent {
                                position,
                                pressed_button: None,
                                modifiers: Modifiers::default(),
                            }),
                            cx,
                        );
                        window.draw(cx).clear(cx);
                        window.dispatch_event(
                            PlatformInput::Scroll(gpui_kit::ScrollEvent {
                                position,
                                delta: gpui_kit::ScrollDelta::Pixels(point(px(0.), px(dy))),
                                ..Default::default()
                            }),
                            cx,
                        );
                        window.draw(cx).clear(cx);
                    });
                }
                Step::Do(f) => {
                    let _ = handle.update(cx, |_, window, cx| {
                        workspace.update(cx, |ws, cx| f(ws, window, cx))
                    });
                    // Offscreen windows get no display-link frames; lay out now so
                    // work that starts on first layout (like the shell) begins.
                    let _ = handle.update(cx, |_, window, cx| window.draw(cx).clear(cx));
                }
                Step::Shot(hold) => {
                    let path = if many {
                        output.join(format!("frame_{index:03}.png"))
                    } else {
                        output.clone()
                    };
                    if save_frame(handle, &path, cx) {
                        println!("capture: wrote {}", path.display());
                        list.push_str(&format!(
                            "file '{}'\nduration {:.2}\n",
                            path.display(),
                            hold as f64 / 1000.
                        ));
                        index += 1;
                    }
                }
            }
        }
        if many {
            let _ = std::fs::write(output.join("frames.txt"), list);
        }
        cx.update(|cx| cx.quit());
    })
    .detach();
}
