//! Review: everything an agent changed, as one virtualized, syntax-highlighted
//! diff: either uncommitted work or the whole branch since it left its base.
use std::path::{Path, PathBuf};

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::{
    ActiveTheme, Icon, IconName, Sizable, StyledExt as _,
    diff::{
        Diff, DiffChangeIndicator, DiffFile, DiffFileStatus, DiffHoverHighlight, DiffHunkSeparator,
        DiffMode, DiffSide, DiffState,
    },
    h_flex,
    spinner::Spinner,
    v_flex,
};
use gpui_kit::{
    App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight,
    InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Task, Window, div, prelude::FluentBuilder,
};

use super::{icon_button, r};
use crate::{
    files,
    git::{self, ReviewScope},
    settings::{ReviewBase, ReviewLayout, Settings},
};

pub enum ReviewEvent {
    OpenFile(PathBuf, Option<u32>),
}

#[derive(Clone)]
pub struct ReviewFile {
    pub path: SharedString,
    pub additions: usize,
    pub deletions: usize,
    pub badge: &'static str,
}

pub struct Summary {
    pub label: SharedString,
    pub additions: usize,
    pub deletions: usize,
    pub loading: bool,
}

pub struct Review {
    root: Option<PathBuf>,
    repo: Option<git::Repository>,
    base: Option<String>,
    scope: ReviewBase,
    state: Entity<DiffState>,
    files: Vec<ReviewFile>,
    loading: bool,
    loaded: bool,
    error: Option<SharedString>,
    skipped: usize,
    generation: u64,
    focus: FocusHandle,
    _task: Option<Task<()>>,
    reload_task: Option<Task<()>>,
    /// Frames left to redraw after new files arrive: the diff list measures its
    /// viewport while drawing, so its width settles on the following frame.
    settle_frames: u8,
}

impl EventEmitter<ReviewEvent> for Review {}

impl Review {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = Settings::get(cx).clone();
        let state = cx.new(|cx| DiffState::new([], cx).with_mode(mode(settings.review_layout)));
        Self {
            root: None,
            repo: None,
            base: None,
            scope: settings.review_base,
            state,
            files: Vec::new(),
            loading: false,
            loaded: false,
            error: None,
            skipped: 0,
            generation: 0,
            focus: cx.focus_handle(),
            _task: None,
            reload_task: None,
            settle_frames: 0,
        }
    }

    pub fn is_loaded(&self) -> bool {
        self.loaded
    }

    pub fn files(&self) -> &[ReviewFile] {
        &self.files
    }

    pub fn set_root(&mut self, root: Option<PathBuf>, cx: &mut Context<Self>) {
        self.root = root;
        self.repo = None;
        self.base = None;
        self.files.clear();
        self.loaded = false;
        self.error = None;
        self.state.update(cx, |state, cx| state.set_files([], cx));
        cx.notify();
    }

    pub fn repository_changed(&mut self, repo: Option<git::Repository>, cx: &mut Context<Self>) {
        let first = self.repo.is_none();
        self.repo = repo;
        if first && let Some(root) = self.repo.as_ref().map(|r| r.root.clone()) {
            // Finding the base is a couple of Git calls; keep it off the UI thread.
            self._task = Some(cx.spawn(async move |this, cx| {
                let base = cx
                    .background_executor()
                    .spawn(async move { git::default_base(&root) })
                    .await;
                this.update(cx, |this, cx| {
                    this.base = base;
                    if this.loaded {
                        this.reload(cx);
                    }
                    cx.notify();
                })
                .ok();
            }));
        } else if self.loaded {
            self.reload(cx);
        }
        cx.notify();
    }

    pub fn ensure_loaded(&mut self, cx: &mut Context<Self>) {
        if !self.loaded && !self.loading {
            self.reload(cx);
        }
    }

    /// Something in the working tree changed. Refresh shortly if we are showing it.
    pub fn mark_stale(&mut self, cx: &mut Context<Self>) {
        if !self.loaded {
            return;
        }
        self.reload_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(400))
                .await;
            this.update(cx, |this, cx| this.reload(cx)).ok();
        }));
    }

    fn effective_scope(&self) -> ReviewScope {
        match (&self.scope, &self.base, self.repo.as_ref()) {
            (ReviewBase::Branch, Some(base), Some(repo))
                if repo.branch != *base && !base.ends_with(&format!("/{}", repo.branch)) =>
            {
                ReviewScope::Branch(base.clone())
            }
            _ => ReviewScope::Uncommitted,
        }
    }

    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self
            .repo
            .as_ref()
            .map(|r| r.root.clone())
            .or_else(|| self.root.clone())
        else {
            return;
        };
        if self.repo.is_none() {
            self.error = Some("This folder is not a Git repository.".into());
            self.loaded = true;
            cx.notify();
            return;
        }
        self.generation += 1;
        let generation = self.generation;
        let scope = self.effective_scope();
        self.loading = true;
        cx.notify();
        self._task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let review = git::review_patch(&root, scope)?;
                    let parsed = DiffFile::parse(&review.patch).map_err(|e| e.to_string())?;
                    let files: Vec<DiffFile> = parsed
                        .into_iter()
                        .map(|file| {
                            let language = files::language_for(Path::new(file.path().as_ref()));
                            file.with_language(language)
                        })
                        .collect();
                    Ok::<_, String>((files, review.skipped))
                })
                .await;
            this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.loading = false;
                this.loaded = true;
                match result {
                    Ok((files, skipped)) => {
                        this.error = None;
                        this.skipped = skipped;
                        this.files = files
                            .iter()
                            .map(|f| ReviewFile {
                                path: f.path().clone(),
                                additions: f.additions(),
                                deletions: f.deletions(),
                                badge: match f.status() {
                                    DiffFileStatus::Added => "A",
                                    DiffFileStatus::Deleted => "D",
                                    DiffFileStatus::Renamed | DiffFileStatus::Copied => "R",
                                    DiffFileStatus::Conflicted => "!",
                                    _ => "M",
                                },
                            })
                            .collect();
                        this.state
                            .update(cx, |state, cx| state.set_files(files, cx));
                        this.settle_frames = 2;
                    }
                    Err(error) => this.error = Some(error.into()),
                }
                cx.notify();
            })
            .ok();
        }));
    }

    pub fn set_scope(&mut self, scope: ReviewBase, cx: &mut Context<Self>) {
        if self.scope != scope {
            self.scope = scope;
            Settings::update(cx, |s| s.review_base = scope);
            self.reload(cx);
        }
    }

    pub fn apply_settings(&mut self, settings: &Settings, cx: &mut Context<Self>) {
        let mode = mode(settings.review_layout);
        self.state.update(cx, |state, cx| {
            if state.mode() != mode {
                state.set_mode(mode, cx);
            }
        });
    }

    pub fn scroll_to_file(&mut self, path: &str, cx: &mut Context<Self>) {
        let path = path.to_owned();
        self.state.update(cx, |state, cx| {
            state.set_file_collapsed(&path, false, cx);
            state.scroll_to_file(&path, cx);
        });
    }

    #[cfg(feature = "capture")]
    pub fn next_change(&mut self, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.next_change(cx));
    }

    pub fn focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let handle = self.state.read(cx).focus_handle(cx);
        handle.focus(window, cx);
    }

    pub fn summary(&self, _cx: &App) -> Summary {
        let additions = self.files.iter().map(|f| f.additions).sum();
        let deletions = self.files.iter().map(|f| f.deletions).sum();
        let label: SharedString = if !self.loaded && !self.loading {
            "Open Review to load the diff".into()
        } else if self.files.is_empty() && self.loading {
            "Loading changes…".into()
        } else if self.files.is_empty() {
            "No changes".into()
        } else if self.files.len() == 1 {
            "1 file changed".into()
        } else {
            format!("{} files changed", self.files.len()).into()
        };
        Summary {
            label,
            additions,
            deletions,
            loading: self.loading,
        }
    }

    fn scope_title(&self) -> SharedString {
        match self.effective_scope() {
            ReviewScope::Branch(base) => {
                let branch = self
                    .repo
                    .as_ref()
                    .map(|r| r.branch.clone())
                    .unwrap_or_default();
                format!("{branch} since {base}").into()
            }
            ReviewScope::Uncommitted => "Uncommitted changes".into(),
        }
    }
}

fn mode(layout: ReviewLayout) -> DiffMode {
    match layout {
        ReviewLayout::Unified => DiffMode::Unified,
        ReviewLayout::Split => DiffMode::Split,
    }
}

/// Two small pills that choose between branch and uncommitted review.
pub fn scope_switcher<T: 'static>(
    review: &Entity<Review>,
    cx: &mut Context<T>,
) -> impl IntoElement {
    let theme = cx.theme().clone();
    let current = review.read(cx).scope;
    let base = review.read(cx).base.clone();
    let pill = |id: &'static str, label: SharedString, value: ReviewBase| {
        let selected = current == value;
        let review = review.clone();
        let hover = theme.list_hover;
        div()
            .id(id)
            .flex_1()
            .h(r(24.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(r(5.))
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
            .child(label)
            .on_click(move |_, _, cx| review.update(cx, |review, cx| review.set_scope(value, cx)))
    };
    h_flex()
        .mx(r(14.))
        .mt(r(4.))
        .p(r(2.))
        .gap(r(2.))
        .rounded(r(7.))
        .bg(theme.list_hover)
        .child(pill(
            "scope-branch",
            base.map(|b| format!("Since {b}"))
                .unwrap_or_else(|| "Branch".into())
                .into(),
            ReviewBase::Branch,
        ))
        .child(pill(
            "scope-uncommitted",
            "Uncommitted".into(),
            ReviewBase::Uncommitted,
        ))
}

impl Focusable for Review {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Review {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.settle_frames > 0 {
            self.settle_frames -= 1;
            window.request_animation_frame();
        }
        let theme = cx.theme().clone();
        let summary = self.summary(cx);
        let layout = Settings::get(cx).review_layout;
        let segment = |id: &'static str, icon: Lucide, value: ReviewLayout, tip: &'static str| {
            let selected = layout == value;
            let hover = theme.list_hover;
            div()
                .id(id)
                .size(r(24.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(r(5.))
                .cursor_pointer()
                .when(selected, |d| {
                    d.bg(theme.background)
                        .text_color(theme.foreground)
                        .shadow_xs()
                })
                .when(!selected, |d| {
                    d.text_color(theme.muted_foreground)
                        .hover(move |s| s.bg(hover))
                })
                .child(Icon::new(icon).size(r(14.)))
                .tooltip(move |window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new(tip).build(window, cx)
                })
                .on_click(move |_, _, cx| Settings::update(cx, |s| s.review_layout = value))
        };
        let toolbar = h_flex()
            .flex_none()
            .h(r(52.))
            .px(r(20.))
            .gap(r(10.))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_size(r(16.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .truncate()
                            .child(self.scope_title()),
                    )
                    .child(
                        h_flex()
                            .gap(r(8.))
                            .text_size(r(12.5))
                            .text_color(theme.muted_foreground)
                            .child(summary.label.clone())
                            .when(summary.additions > 0, |row| {
                                row.child(
                                    div()
                                        .text_color(theme.green)
                                        .child(format!("+{}", summary.additions)),
                                )
                            })
                            .when(summary.deletions > 0, |row| {
                                row.child(
                                    div()
                                        .text_color(theme.red)
                                        .child(format!("−{}", summary.deletions)),
                                )
                            })
                            .when(self.skipped > 0, |row| {
                                row.child(format!(
                                    "· {} large or extra untracked files hidden",
                                    self.skipped
                                ))
                            }),
                    ),
            )
            .when(self.loading, |bar| bar.child(Spinner::new().small()))
            .child(
                h_flex()
                    .p(r(2.))
                    .gap(r(2.))
                    .rounded(r(7.))
                    .bg(theme.list_hover)
                    .child(segment(
                        "layout-unified",
                        Lucide::Rows2,
                        ReviewLayout::Unified,
                        "Unified",
                    ))
                    .child(segment(
                        "layout-split",
                        Lucide::Columns2,
                        ReviewLayout::Split,
                        "Side by side",
                    )),
            )
            .child(
                icon_button("review-prev", IconName::ChevronUp, cx)
                    .tooltip(|window, cx| {
                        gpui_kit::component::tooltip::Tooltip::new("Previous change")
                            .build(window, cx)
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.state.update(cx, |s, cx| s.previous_change(cx))
                    })),
            )
            .child(
                icon_button("review-next", IconName::ChevronDown, cx)
                    .tooltip(|window, cx| {
                        gpui_kit::component::tooltip::Tooltip::new("Next change").build(window, cx)
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.state.update(cx, |s, cx| s.next_change(cx))
                    })),
            )
            .child(
                icon_button("review-refresh", IconName::RefreshCw, cx)
                    .tooltip(|window, cx| {
                        gpui_kit::component::tooltip::Tooltip::new("Refresh").build(window, cx)
                    })
                    .on_click(cx.listener(|this, _, _, cx| this.reload(cx))),
            );

        let body = if let Some(error) = &self.error {
            empty_state(
                Lucide::GitCompare,
                "Can't show changes",
                error.clone(),
                &theme,
            )
            .into_any_element()
        } else if self.files.is_empty() && self.loading {
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .child(Spinner::new())
                .into_any_element()
        } else if self.files.is_empty() {
            let hint: SharedString = match self.effective_scope() {
                ReviewScope::Branch(base) => format!(
                    "This branch matches {base}. When an agent edits files, they show up here."
                )
                .into(),
                ReviewScope::Uncommitted => {
                    "Everything is committed. When an agent edits files, they show up here.".into()
                }
            };
            empty_state(Lucide::GitCompare, "Nothing to review", hint, &theme).into_any_element()
        } else {
            let root = self.repo.as_ref().map(|r| r.root.clone());
            let entity = cx.entity().downgrade();
            let open_root = root.clone();
            Diff::new(&self.state)
                .hover_highlight(DiffHoverHighlight::Both)
                .hunk_separator(DiffHunkSeparator::Simple)
                .change_indicator(DiffChangeIndicator::Bars)
                // Split columns share the window, so long lines wrap instead of scrolling away.
                .soft_wrap(Settings::get(cx).soft_wrap || layout == ReviewLayout::Split)
                .render_header_suffix(move |file, _, cx| {
                    let path = open_root
                        .as_ref()
                        .map(|root| root.join(file.path().as_ref()));
                    let entity = entity.clone();
                    let deleted = file.status() == DiffFileStatus::Deleted;
                    let hover = cx.theme().list_hover;
                    div()
                        .id(SharedString::from(format!("open-{}", file.path())))
                        .px(r(8.))
                        .h(r(22.))
                        .flex()
                        .items_center()
                        .rounded(r(5.))
                        .text_size(r(12.))
                        .text_color(cx.theme().muted_foreground)
                        .when(!deleted, |d| {
                            d.cursor_pointer()
                                .hover(move |s| s.bg(hover))
                                .child("Open file")
                        })
                        .on_click(move |_, _, cx| {
                            if let (Some(path), Some(entity)) = (path.clone(), entity.upgrade()) {
                                entity
                                    .update(cx, |_, cx| cx.emit(ReviewEvent::OpenFile(path, None)));
                            }
                        })
                })
                .on_line_click({
                    let entity = cx.entity().downgrade();
                    move |position, event, _, cx| {
                        if event.click_count() < 2 || position.side() != DiffSide::Modified {
                            return;
                        }
                        if let (Some(root), Some(entity)) = (root.as_ref(), entity.upgrade()) {
                            let path = root.join(position.path().as_ref());
                            let line = position.line() as u32;
                            entity.update(cx, |_, cx| {
                                cx.emit(ReviewEvent::OpenFile(path, Some(line)))
                            });
                        }
                    }
                })
                .size_full()
                .into_any_element()
        };

        v_flex()
            .id("review")
            .track_focus(&self.focus)
            .size_full()
            .child(toolbar)
            .child(div().flex_1().min_h_0().px(r(12.)).pb(r(8.)).child(body))
    }
}

pub fn empty_state(
    icon: Lucide,
    title: &'static str,
    detail: SharedString,
    theme: &gpui_kit::component::Theme,
) -> impl IntoElement {
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .gap(r(8.))
        .child(
            Icon::new(icon)
                .size(r(28.))
                .text_color(theme.muted_foreground.opacity(0.6)),
        )
        .child(
            div()
                .text_size(r(15.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(title),
        )
        .child(
            div()
                .max_w(r(380.))
                .text_center()
                .text_size(r(13.))
                .text_color(theme.muted_foreground)
                .child(detail),
        )
}
