//! The first screen: what Modo is for, and the projects you were just in.
use std::path::PathBuf;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::{ActiveTheme, Icon, StyledExt as _, h_flex, v_flex};
use gpui_kit::{
    App, Context, FontWeight, InteractiveElement, IntoElement, ParentElement, Rems, SharedString,
    StatefulInteractiveElement, Styled, Window, div, img, prelude::FluentBuilder,
};

use super::{
    r, shortcut,
    workspace::{CommandPalette, OpenFolder, QuickOpen, ReviewChanges, SwitchProject, Workspace},
};
use crate::{brand, settings::Session};

pub fn logo(size: Rems, cx: &App) -> impl IntoElement {
    let path = if cx.theme().is_dark() {
        "brand/mark-dark.svg"
    } else {
        "brand/mark.svg"
    };
    img(path).size(size).flex_none()
}

fn action_row(
    id: &'static str,
    icon: Lucide,
    label: &'static str,
    keys: &'static str,
    action: Box<dyn gpui_kit::Action>,
    cx: &App,
) -> impl IntoElement {
    let theme = cx.theme().clone();
    let hover = theme.list_hover;
    h_flex()
        .id(id)
        .h(r(34.))
        .px(r(10.))
        .gap(r(10.))
        .rounded(r(6.))
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .child(
            Icon::new(icon)
                .size(r(16.))
                .text_color(theme.muted_foreground),
        )
        .child(div().flex_1().text_size(r(14.)).child(label))
        .child(
            div()
                .text_size(r(12.5))
                .text_color(theme.muted_foreground)
                .child(keys),
        )
        .on_click(move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx))
}

pub fn render(
    _this: &mut Workspace,
    _window: &mut Window,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
    let theme = cx.theme().clone();
    let recent: Vec<PathBuf> = Session::get(cx).recent.iter().take(6).cloned().collect();
    let hover = theme.list_hover;
    div().id("welcome").size_full().overflow_y_scroll().child(
        v_flex()
            .w_full()
            .items_center()
            .pt(gpui_kit::relative(0.14))
            .pb(r(48.))
            .child(
                v_flex()
                    .w(r(460.))
                    .max_w(gpui_kit::relative(0.9))
                    .gap(r(6.))
                    .child(logo(r(52.), cx))
                    .child(
                        div()
                            .pt(r(14.))
                            .text_size(r(28.))
                            .font_weight(FontWeight::BOLD)
                            .child(brand::NAME),
                    )
                    .child(
                        div()
                            .text_size(r(15.))
                            .text_color(theme.muted_foreground)
                            .child(brand::TAGLINE),
                    )
                    .child(
                        v_flex()
                            .pt(r(28.))
                            .gap(r(2.))
                            .child(action_row(
                                "w-open",
                                Lucide::FolderOpen,
                                "Open folder…",
                                shortcut("⌘O", "Ctrl+O"),
                                Box::new(OpenFolder),
                                cx,
                            ))
                            .child(action_row(
                                "w-switch",
                                Lucide::FolderGit2,
                                "Switch project",
                                "",
                                Box::new(SwitchProject),
                                cx,
                            ))
                            .child(action_row(
                                "w-commands",
                                Lucide::Command,
                                "All commands",
                                shortcut("⌘K", "Ctrl+K"),
                                Box::new(CommandPalette),
                                cx,
                            )),
                    )
                    .when(!recent.is_empty(), |col| {
                        col.child(
                            div()
                                .pt(r(28.))
                                .pb(r(6.))
                                .px(r(10.))
                                .text_size(r(12.))
                                .font_medium()
                                .text_color(theme.muted_foreground)
                                .child("Recent"),
                        )
                        .children(recent.into_iter().enumerate().map(|(ix, path)| {
                            let name: SharedString = path
                                .file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_default()
                                .into();
                            let parent = path
                                .parent()
                                .map(|p| {
                                    let home = std::env::var_os("HOME").map(PathBuf::from);
                                    match home.as_ref().and_then(|h| p.strip_prefix(h).ok()) {
                                        Some(rest) => format!("~/{}", rest.display()),
                                        None => p.display().to_string(),
                                    }
                                })
                                .unwrap_or_default();
                            h_flex()
                                .id(("recent", ix))
                                .h(r(34.))
                                .px(r(10.))
                                .gap(r(10.))
                                .rounded(r(6.))
                                .cursor_pointer()
                                .hover(move |s| s.bg(hover))
                                .child(
                                    Icon::new(Lucide::FolderGit2)
                                        .size(r(16.))
                                        .text_color(theme.muted_foreground),
                                )
                                .child(div().flex_none().text_size(r(14.)).child(name))
                                .child(
                                    div()
                                        .flex_1()
                                        .truncate()
                                        .text_size(r(12.5))
                                        .text_color(theme.muted_foreground)
                                        .child(parent),
                                )
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.open_path(path.clone(), window, cx)
                                }))
                        }))
                    }),
            ),
    )
}

/// A project is open but no tab is: point at the useful next steps.
pub fn render_empty_project(this: &mut Workspace, cx: &mut Context<Workspace>) -> impl IntoElement {
    let theme = cx.theme().clone();
    let changes = this.change_count();
    v_flex().size_full().items_center().justify_center().child(
        v_flex()
            .w(r(380.))
            .gap(r(2.))
            .child(
                div()
                    .pb(r(10.))
                    .px(r(10.))
                    .text_size(r(20.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(
                        this.project
                            .as_ref()
                            .map(|p| p.name.clone())
                            .unwrap_or_default(),
                    ),
            )
            .child(action_row(
                "e-review",
                Lucide::GitCompare,
                if changes > 0 {
                    "Review changes"
                } else {
                    "Review branch"
                },
                shortcut("⌘⇧G", "Ctrl+Shift+G"),
                Box::new(ReviewChanges),
                cx,
            ))
            .child(action_row(
                "e-goto",
                Lucide::File,
                "Go to file",
                shortcut("⌘P", "Ctrl+P"),
                Box::new(QuickOpen),
                cx,
            ))
            .child(action_row(
                "e-commands",
                Lucide::Command,
                "All commands",
                shortcut("⌘K", "Ctrl+K"),
                Box::new(CommandPalette),
                cx,
            ))
            .when(changes > 0, |col| {
                col.child(
                    div()
                        .pt(r(14.))
                        .px(r(10.))
                        .text_size(r(12.5))
                        .text_color(theme.muted_foreground)
                        .child(format!(
                            "{changes} file{} changed in this working tree.",
                            if changes == 1 { "" } else { "s" }
                        )),
                )
            }),
    )
}
