//! Offscreen frame capture for docs and visual review, compiled only with the
//! `capture` feature.
//!
//! `modo --capture out.png --scene review <folder>` saves one frame.
//! `modo --capture frames/ --scene tour <folder>` drives a scripted walkthrough
//! and saves numbered frames plus `frames.txt` (an ffmpeg concat list).
use std::{path::PathBuf, time::Duration};

use gpui_kit::component::TitleBar;
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
            button: MouseButton::Left,
            position,
            modifiers,
            click_count: 1,
            first_mouse: false,
        }),
        cx,
    );
    window.dispatch_event(
        PlatformInput::MouseUp(MouseUpEvent {
            button: MouseButton::Left,
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
        _ => ThemeChoice::System,
    })
}

fn single(scene: &str, file: Option<PathBuf>) -> Vec<Step> {
    let want_preview = scene == "preview";
    let scene = scene.to_owned();
    let settle = std::env::var("MODO_CAPTURE_DELAY_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1600);
    let mut steps = vec![
        Step::Wait(900),
        act(move |ws, window, cx| {
            if let Some(file) = file {
                ws.open_file(file, None, window, cx);
            }
            match scene.as_str() {
                "review" | "review-split" => {
                    if scene == "review-split" {
                        Settings::update(cx, |s| s.review_layout = ReviewLayout::Split);
                    }
                    ws.panel = Panel::Changes;
                    ws.open_review(window, cx);
                }
                "palette" => ws.open_palette("", window, cx),
                "commands" => ws.open_palette(">", window, cx),
                "settings" => ws.open_settings(window, cx),
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
            if std::env::var_os("MODO_CAPTURE_TERMINAL").is_some() {
                ws.terminal.update(cx, |t, cx| t.show(window, cx));
            }
        }),
        Step::Wait(settle),
    ];
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
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(1320.), px(840.)),
            cx,
        ))),
        titlebar: Some(TitlebarOptions {
            title: Some(crate::brand::NAME.into()),
            appears_transparent: true,
            traffic_light_position: Some(point(px(15.), px(15.))),
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
                    cx.background_executor()
                        .timer(Duration::from_millis(ms))
                        .await
                }
                Step::Click(x, y) => {
                    let _ = handle.update(cx, |_, window, cx| click(window, x, y, cx));
                    let _ = handle.update(cx, |_, window, cx| window.draw(cx).clear(cx));
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
