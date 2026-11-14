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
    workspace::{CommandPalette, OpenFolder, ReviewChanges, SwitchProject, Workspace},
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
                .text_size(r(12.))
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
                                        .text_size(r(12.))
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

/// How many changed files the overview lists before it says "and N more".
const OVERVIEW_FILES: usize = 7;

/// A project is open but no tab is: show the one thing worth knowing, where
/// the branch's changes are and how much of them has been reviewed.
pub fn render_empty_project(this: &mut Workspace, cx: &mut Context<Workspace>) -> impl IntoElement {
    let theme = cx.theme().clone();
    let name = this
        .project
        .as_ref()
        .map(|p| p.name.clone())
        .unwrap_or_default();
    let branch = this.git.repo.as_ref().map(|repo| repo.branch.clone());
    if branch.is_some() {
        // The overview is drawn from the same diff the Review tab shows.
        this.review
            .update(cx, |review, cx| review.ensure_loaded(cx));
    }
    let review = this.review.read(cx);
    let summary = review.summary(cx);
    let compare = review.compare_label();
    let loading = review.is_loading();
    let mut files: Vec<_> = review.files().to_vec();
    let total = files.len();
    let reviewed = files.iter().filter(|file| file.reviewed).count();
    let additions: usize = files.iter().map(|file| file.additions).sum();
    let deletions: usize = files.iter().map(|file| file.deletions).sum();
    // The busiest files first: that is where a review should start.
    files.sort_by_key(|file| std::cmp::Reverse(file.additions + file.deletions));
    let largest = files
        .first()
        .map(|file| file.additions + file.deletions)
        .unwrap_or(0)
        .max(1);
    let more = total.saturating_sub(OVERVIEW_FILES);
    files.truncate(OVERVIEW_FILES);

    let hover = theme.list_hover;
    let rows = files.into_iter().enumerate().map(|(ix, file)| {
        let path = file.path.clone();
        let (folder, leaf) = match file.path.rsplit_once('/') {
            Some((folder, leaf)) => (format!("{folder}/"), leaf.to_owned()),
            None => (String::new(), file.path.to_string()),
        };
        let churn = file.additions + file.deletions;
        // At least a sliver, so a one-line change is still visible.
        let share = (churn as f32 / largest as f32).max(0.03);
        let added = if churn == 0 {
            0.
        } else {
            file.additions as f32 / churn as f32
        };
        h_flex()
            .id(("overview-file", ix))
            .h(r(28.))
            .px(r(8.))
            .gap(r(10.))
            .rounded(r(6.))
            .cursor_pointer()
            .hover(move |s| s.bg(hover))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.open_review(window, cx);
                let path = path.clone();
                this.review
                    .update(cx, |review, cx| review.scroll_to_file(&path, cx));
            }))
            .child(
                div()
                    .flex_none()
                    .w(r(10.))
                    .text_size(r(11.5))
                    .font_medium()
                    .text_color(super::sidebar::status_color(file.badge, cx))
                    .child(file.badge),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .text_size(r(13.))
                    .child(
                        div()
                            .flex_shrink(1.)
                            .min_w_0()
                            .truncate()
                            .text_color(theme.muted_foreground)
                            .child(folder),
                    )
                    .child(
                        div()
                            .flex_none()
                            .when(file.reviewed, |d| d.text_color(theme.muted_foreground))
                            .child(leaf),
                    ),
            )
            .when(file.reviewed, |row| {
                row.child(
                    Icon::new(Lucide::Check)
                        .size(r(13.))
                        .flex_none()
                        .text_color(theme.green),
                )
            })
            .child(
                // Length is this file's share of the largest change; the
                // split is added against removed lines.
                h_flex().flex_none().w(r(96.)).h(r(6.)).child(
                    h_flex()
                        .h_full()
                        .w(gpui_kit::relative(share))
                        .rounded(r(4.))
                        .overflow_hidden()
                        .bg(theme.red.opacity(0.75))
                        .child(
                            div()
                                .h_full()
                                .w(gpui_kit::relative(added))
                                .bg(theme.green.opacity(0.85)),
                        ),
                ),
            )
            .child(
                div()
                    .flex_none()
                    .w(r(42.))
                    .text_right()
                    .text_size(r(12.))
                    .text_color(theme.muted_foreground)
                    .child(churn.to_string()),
            )
    });

    let stat = |value: String, label: &'static str, color: gpui_kit::Hsla| {
        h_flex()
            .items_baseline()
            .gap(r(5.))
            .child(
                div()
                    .text_size(r(20.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(color)
                    .child(value),
            )
            .child(
                div()
                    .text_size(r(12.))
                    .text_color(theme.muted_foreground)
                    .child(label),
            )
    };
    let hint = |keys: &'static str, label: &'static str| {
        h_flex()
            .gap(r(6.))
            .child(
                div()
                    .text_size(r(12.))
                    .text_color(theme.muted_foreground.opacity(0.8))
                    .child(keys),
            )
            .child(div().text_size(r(12.)).child(label))
    };
    let progress = if total == 0 {
        0.
    } else {
        reviewed as f32 / total as f32
    };

    v_flex().size_full().items_center().justify_center().child(
        v_flex()
            .w(r(520.))
            .max_w(gpui_kit::relative(0.9))
            .gap(r(16.))
            .child(
                h_flex()
                    .px(r(8.))
                    .gap(r(10.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(r(16.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(name),
                    )
                    .when_some(branch, |row, branch| {
                        row.child(
                            h_flex()
                                .flex_none()
                                .max_w(r(240.))
                                .gap(r(5.))
                                .text_size(r(12.))
                                .text_color(theme.muted_foreground)
                                .child(Icon::new(Lucide::GitBranch).size(r(13.)))
                                .child(div().truncate().child(branch)),
                        )
                    }),
            )
            .map(|col| {
                if total > 0 {
                    col.child(
                        v_flex()
                            .gap(r(10.))
                            .p(r(12.))
                            .rounded(r(8.))
                            .border_1()
                            .border_color(theme.border)
                            .child(
                                h_flex()
                                    .px(r(4.))
                                    .gap(r(18.))
                                    .child(stat(
                                        total.to_string(),
                                        if total == 1 { "file" } else { "files" },
                                        theme.foreground,
                                    ))
                                    .child(stat(format!("+{additions}"), "added", theme.green))
                                    .child(stat(format!("−{deletions}"), "removed", theme.red))
                                    .child(div().flex_1())
                                    .child(
                                        div()
                                            .text_size(r(12.))
                                            .text_color(theme.muted_foreground)
                                            .child(compare),
                                    ),
                            )
                            .child(
                                // Review progress across every changed file.
                                h_flex()
                                    .px(r(4.))
                                    .gap(r(10.))
                                    .child(
                                        div()
                                            .flex_1()
                                            .h(r(4.))
                                            .rounded_full()
                                            .overflow_hidden()
                                            .bg(theme.list_hover)
                                            .child(
                                                div()
                                                    .h_full()
                                                    .w(gpui_kit::relative(progress))
                                                    .rounded_full()
                                                    .bg(theme.green),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .flex_none()
                                            .text_size(r(12.))
                                            .text_color(theme.muted_foreground)
                                            .child(format!("{reviewed} of {total} reviewed")),
                                    ),
                            )
                            .child(v_flex().children(rows))
                            .child(
                                h_flex()
                                    .id("overview-review")
                                    .h(r(28.))
                                    .px(r(8.))
                                    .gap(r(8.))
                                    .rounded(r(6.))
                                    .cursor_pointer()
                                    .text_size(r(13.))
                                    .hover(move |s| s.bg(hover))
                                    .on_click(|_, window, cx| {
                                        window.dispatch_action(Box::new(ReviewChanges), cx)
                                    })
                                    .child(
                                        Icon::new(Lucide::GitCompare)
                                            .size(r(15.))
                                            .text_color(theme.muted_foreground),
                                    )
                                    .child(div().flex_1().child(if more > 0 {
                                        format!("Review all changes ({more} more)")
                                    } else {
                                        "Review all changes".to_owned()
                                    }))
                                    .child(
                                        div()
                                            .text_size(r(12.))
                                            .text_color(theme.muted_foreground)
                                            .child(shortcut("⌘⇧G", "Ctrl+Shift+G")),
                                    ),
                            ),
                    )
                } else {
                    col.child(
                        div()
                            .px(r(8.))
                            .text_size(r(13.))
                            .text_color(theme.muted_foreground)
                            .child(if this.git.repo.is_none() {
                                "This folder is not a Git repository.".into()
                            } else if let Some(error) = summary.error.clone() {
                                format!("Can't show changes. {error}").into()
                            } else if loading {
                                summary.label.clone()
                            } else {
                                SharedString::from(format!("{compare}: no changes."))
                            }),
                    )
                }
            })
            .when_some(summary.notice, |col, notice| {
                col.child(
                    div()
                        .px(r(8.))
                        .text_size(r(12.))
                        .text_color(theme.muted_foreground)
                        .child(notice),
                )
            })
            .child(
                h_flex()
                    .px(r(8.))
                    .gap(r(18.))
                    .flex_wrap()
                    .child(hint(shortcut("⌘P", "Ctrl+P"), "Go to file"))
                    .child(hint(shortcut("⌘⇧F", "Ctrl+Shift+F"), "Search"))
                    .child(hint(shortcut("⌘K", "Ctrl+K"), "Commands"))
                    .child(hint(shortcut("⌘J", "Ctrl+J"), "Terminal"))
                    .child(hint(shortcut("⌘O", "Ctrl+O"), "Open folder")),
            ),
    )
}
