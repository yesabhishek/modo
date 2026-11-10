//! Review: everything an agent changed, as one virtualized, syntax-highlighted
//! diff: either uncommitted work or the whole branch since it left its base.
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::{
    ActiveTheme, Icon, IconName, Sizable,
    button::{Button, ButtonVariants},
    diff::{
        Diff, DiffChangeIndicator, DiffFile, DiffFileStatus, DiffHoverHighlight, DiffHunkSeparator,
        DiffMode, DiffSide, DiffState,
    },
    h_flex,
    menu::{DropdownMenu, PopupMenu, PopupMenuItem},
    spinner::Spinner,
    v_flex,
};
use gpui_kit::{
    App, AppContext, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight,
    InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Task, WeakEntity, Window, div, prelude::FluentBuilder, px,
};

use super::{icon_button, r};
use crate::{
    files,
    git::{self, ReviewScope},
    settings::{ReviewBase, ReviewLayout, Settings},
};

pub enum ReviewEvent {
    OpenFile(PathBuf, Option<u32>),
    /// The user picked what to compare against; worth remembering per project.
    BaseChanged,
    ProgressChanged,
}

#[derive(Clone)]
pub struct ReviewFile {
    pub path: SharedString,
    pub additions: usize,
    pub deletions: usize,
    pub reviewed: bool,
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
    /// The branch this one most likely forked from.
    base: Option<String>,
    /// A base the user picked for this project, overriding `base`.
    chosen: Option<String>,
    /// Branches offered as bases, most recent first.
    branches: Vec<String>,
    commits: Vec<git::Commit>,
    commit: Option<String>,
    scope: ReviewBase,
    state: Entity<DiffState>,
    files: Vec<ReviewFile>,
    reviewed: HashMap<String, String>,
    fingerprints: HashMap<String, String>,
    loading: bool,
    loaded: bool,
    /// The working tree changed while the diff was not on screen.
    stale: bool,
    error: Option<SharedString>,
    skipped: usize,
    generation: u64,
    focus: FocusHandle,
    _task: Option<Task<()>>,
    reload_task: Option<Task<()>>,
    branch_task: Option<Task<()>>,
    base_task: Option<Task<()>>,
    base_loading: bool,
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
            chosen: None,
            branches: Vec::new(),
            commits: Vec::new(),
            commit: None,
            scope: settings.review_base,
            state,
            files: Vec::new(),
            reviewed: HashMap::new(),
            fingerprints: HashMap::new(),
            loading: false,
            loaded: false,
            stale: false,
            error: None,
            skipped: 0,
            generation: 0,
            focus: cx.focus_handle(),
            _task: None,
            reload_task: None,
            branch_task: None,
            base_task: None,
            base_loading: false,
            settle_frames: 0,
        }
    }

    pub fn is_loaded(&self) -> bool {
        self.loaded
    }

    /// A diff is being read and none has arrived yet.
    pub fn is_loading(&self) -> bool {
        self.loading && self.files.is_empty()
    }

    pub fn files(&self) -> &[ReviewFile] {
        &self.files
    }

    /// Show `root`, comparing against `chosen` when the user picked a base before.
    pub fn set_root(
        &mut self,
        root: Option<PathBuf>,
        chosen: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.reviewed = root
            .as_ref()
            .and_then(|root| crate::settings::Session::get(cx).project(root))
            .map(|s| s.reviewed_files.clone())
            .unwrap_or_default();
        self.commit = root
            .as_ref()
            .and_then(|root| crate::settings::Session::get(cx).project(root))
            .and_then(|session| session.review_commit.clone());
        self.commits.clear();
        self.fingerprints.clear();
        self.generation += 1;
        self._task = None;
        self.reload_task = None;
        self.branch_task = None;
        self.base_task = None;
        self.base_loading = false;
        self.loading = false;
        self.root = root;
        self.repo = None;
        self.base = None;
        self.chosen = chosen;
        self.branches.clear();
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
            self.load_branches(cx);
            // Finding the base is a couple of Git calls; keep it off the UI thread.
            self.base_loading = true;
            self.base_task = Some(cx.spawn(async move |this, cx| {
                let base = cx
                    .background_executor()
                    .spawn(async move { git::default_base(&root) })
                    .await;
                this.update(cx, |this, cx| {
                    this.base = base;
                    this.base_loading = false;
                    this.loading = false;
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
        if (!self.loaded || self.stale) && !self.loading {
            self.reload(cx);
        }
    }

    /// The working tree changed but nothing shows the diff right now: read it
    /// again when it is next needed, instead of after every save.
    pub fn mark_stale_later(&mut self) {
        if self.loaded {
            self.stale = true;
            self.reload_task = None;
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

    /// Refresh the branches the compare menu offers.
    pub fn load_branches(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.repo.as_ref().map(|r| r.root.clone()) else {
            return;
        };
        self.branch_task = Some(cx.spawn(async move |this, cx| {
            let (branches, commits) = cx
                .background_executor()
                .spawn(async move { (git::branches(&root), git::recent_commits(&root)) })
                .await;
            this.update(cx, |this, cx| {
                this.branches = branches;
                this.commits = commits;
                cx.notify();
            })
            .ok();
        }));
    }

    pub fn reviewed_files(&self) -> HashMap<String, String> {
        self.reviewed.clone()
    }

    pub fn is_reviewed(&self, path: &str) -> bool {
        self.fingerprints
            .get(path)
            .is_some_and(|fingerprint| self.reviewed.get(path) == Some(fingerprint))
    }

    pub fn toggle_reviewed(&mut self, path: &str, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        let Some(fingerprint) = self.fingerprints.get(path).cloned() else {
            return;
        };
        if self.is_reviewed(path) {
            self.reviewed.remove(path);
        } else {
            self.reviewed.insert(path.to_owned(), fingerprint);
        }
        let reviewed = self.is_reviewed(path);
        if let Some(file) = self
            .files
            .iter_mut()
            .find(|file| file.path.as_ref() == path)
        {
            file.reviewed = reviewed;
        }
        self.state
            .update(cx, |state, cx| state.set_file_collapsed(path, reviewed, cx));
        cx.emit(ReviewEvent::ProgressChanged);
        cx.notify();
    }

    pub fn chosen_commit(&self) -> Option<String> {
        self.commit.clone()
    }

    pub fn choose_commit(&mut self, commit: String, cx: &mut Context<Self>) {
        self.commit = Some(commit);
        cx.emit(ReviewEvent::BaseChanged);
        self.reload(cx);
    }

    pub fn chosen_base(&self) -> Option<String> {
        self.chosen.clone()
    }

    fn effective_scope(&self) -> ReviewScope {
        let Some(repo) = self.repo.as_ref() else {
            return ReviewScope::Uncommitted;
        };
        if let Some(commit) = &self.commit {
            return ReviewScope::Commit(commit.clone());
        }
        if self.scope == ReviewBase::Uncommitted {
            return ReviewScope::Uncommitted;
        }
        // A base the user picked is used as is; the detected one is skipped on
        // the branch it names (main against origin/main shows nothing useful).
        let chosen = self.chosen.as_ref().filter(|b| **b != repo.branch);
        let detected = self
            .base
            .as_ref()
            .filter(|b| **b != repo.branch && !b.ends_with(&format!("/{}", repo.branch)));
        match chosen.or(detected) {
            Some(base) => ReviewScope::Branch(base.clone()),
            None => ReviewScope::Uncommitted,
        }
    }

    pub fn reload(&mut self, cx: &mut Context<Self>) {
        if self.base_loading
            && self.scope == ReviewBase::Branch
            && self.chosen.is_none()
            && self.commit.is_none()
        {
            self.loaded = true;
            self.loading = true;
            cx.notify();
            return;
        }
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
        self.stale = false;
        let generation = self.generation;
        let scope = self.effective_scope();
        let context = format!(
            "{}:{scope:?}",
            self.repo
                .as_ref()
                .map(|r| r.branch.as_str())
                .unwrap_or_default()
        );
        self.loading = true;
        cx.notify();
        self._task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let review = git::review_patch(&root, scope)?;
                    let fingerprints = patch_fingerprints(&review.patch, &context);
                    let parsed = DiffFile::parse(&review.patch).map_err(|e| e.to_string())?;
                    let files: Vec<DiffFile> = parsed
                        .into_iter()
                        .map(|file| {
                            let language = files::language_for(Path::new(file.path().as_ref()));
                            file.with_language(language)
                        })
                        .collect();
                    Ok::<_, String>((files, review.skipped, fingerprints))
                })
                .await;
            this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.loading = false;
                this.loaded = true;
                match result {
                    Ok((files, skipped, fingerprints)) => {
                        this.fingerprints = files
                            .iter()
                            .zip(fingerprints)
                            .map(|(file, fingerprint)| (file.path().to_string(), fingerprint))
                            .collect();
                        this.reviewed.retain(|path, fingerprint| {
                            this.fingerprints.get(path) == Some(fingerprint)
                        });
                        this.error = None;
                        this.skipped = skipped;
                        this.files = files
                            .iter()
                            .map(|f| ReviewFile {
                                path: f.path().clone(),
                                reviewed: this.is_reviewed(f.path().as_ref()),
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
                        this.state.update(cx, |state, cx| {
                            state.set_files(files, cx);
                            for file in &this.files {
                                if file.reviewed {
                                    state.set_file_collapsed(&file.path, true, cx);
                                }
                            }
                        });
                        cx.emit(ReviewEvent::ProgressChanged);
                        this.settle_frames = 2;
                    }
                    Err(error) => this.error = Some(error.into()),
                }
                cx.notify();
            })
            .ok();
        }));
    }

    /// Compare against uncommitted work (`None`) or everything since `branch`.
    pub fn choose(&mut self, branch: Option<String>, cx: &mut Context<Self>) {
        let scope = if branch.is_some() {
            ReviewBase::Branch
        } else {
            ReviewBase::Uncommitted
        };
        let chosen = branch.or_else(|| self.chosen.clone());
        if self.scope == scope && self.chosen == chosen && self.commit.is_none() {
            return;
        }
        self.commit = None;
        self.scope = scope;
        self.chosen = chosen;
        Settings::update(cx, |s| s.review_base = scope);
        cx.emit(ReviewEvent::BaseChanged);
        self.reload(cx);
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
        } else {
            format!(
                "{} {} changed · {} reviewed",
                self.files.len(),
                if self.files.len() == 1 {
                    "file"
                } else {
                    "files"
                },
                self.files.iter().filter(|f| f.reviewed).count()
            )
            .into()
        };
        Summary {
            label,
            additions,
            deletions,
            loading: self.loading,
        }
    }

    /// What the review shows, for the compare menu's button.
    pub fn compare_label(&self) -> SharedString {
        match self.effective_scope() {
            ReviewScope::Branch(base) => format!("Since {base}").into(),
            ReviewScope::Commit(commit) => {
                format!("Commit {}", &commit[..commit.len().min(8)]).into()
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

/// The compare menu: uncommitted work, or everything since any local or
/// remote branch, the likely base first and the rest by recent commits.
fn compare_menu(
    review: WeakEntity<Review>,
) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static {
    move |menu, _, cx| {
        let Some(entity) = review.upgrade() else {
            return menu;
        };
        let this = entity.read(cx);
        let current = this.effective_scope();
        let branch = this.repo.as_ref().map(|r| r.branch.clone());
        let mut bases: Vec<String> = Vec::new();
        for base in this
            .base
            .iter()
            .chain(this.chosen.iter())
            .chain(this.branches.iter())
        {
            if Some(base) != branch.as_ref() && !bases.contains(base) {
                bases.push(base.clone());
            }
        }
        let detected = this.base.clone();
        let pick = |choice: Option<String>| {
            let review = review.clone();
            move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                if let Some(review) = review.upgrade() {
                    review.update(cx, |review, cx| review.choose(choice.clone(), cx));
                }
            }
        };
        let mut menu = menu.min_w(px(240.)).max_h(px(420.)).scrollable(true).item(
            PopupMenuItem::new("Uncommitted changes")
                .checked(current == ReviewScope::Uncommitted)
                .on_click(pick(None)),
        );
        if !bases.is_empty() {
            menu = menu.separator().label("Everything since");
        }
        for base in bases {
            let label = if Some(&base) == detected.as_ref() {
                format!("{base}  (default)")
            } else {
                base.clone()
            };
            let checked = current == ReviewScope::Branch(base.clone());
            menu = menu.item(
                PopupMenuItem::new(label)
                    .checked(checked)
                    .on_click(pick(Some(base))),
            );
        }
        if !this.commits.is_empty() {
            menu = menu.separator().label("Recent commits");
        }
        for commit in &this.commits {
            let review = review.clone();
            let id = commit.id.clone();
            menu = menu.item(
                PopupMenuItem::new(format!("{} {}", &id[..id.len().min(8)], commit.subject))
                    .checked(current == ReviewScope::Commit(id.clone()))
                    .on_click(move |_, _, cx| {
                        if let Some(review) = review.upgrade() {
                            review.update(cx, |review, cx| review.choose_commit(id.clone(), cx));
                        }
                    }),
            );
        }
        menu
    }
}

/// A full-width row in the Changes panel that opens the compare menu.
pub fn compare_picker(review: &Entity<Review>, cx: &App) -> impl IntoElement {
    let theme = cx.theme().clone();
    let label = review.read(cx).compare_label();
    let weak = review.downgrade();
    div().px(r(6.)).pt(r(2.)).child(
        Button::new("compare-picker")
            .ghost()
            .small()
            .w_full()
            .h(r(28.))
            .px(r(8.))
            .rounded(px(6.))
            .tab_stop(false)
            .child(
                h_flex()
                    .w_full()
                    .gap(r(8.))
                    .text_size(r(13.))
                    .child(
                        Icon::new(Lucide::GitCompare)
                            .size(r(15.))
                            .text_color(theme.muted_foreground),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(theme.sidebar_foreground)
                            .child(label),
                    )
                    .child(
                        Icon::new(IconName::ChevronDown)
                            .size(r(14.))
                            .text_color(theme.muted_foreground),
                    ),
            )
            .dropdown_menu(compare_menu(weak.clone()))
            .on_open_change(move |open, _, cx| {
                if *open && let Some(review) = weak.upgrade() {
                    review.update(cx, |review, cx| review.load_branches(cx));
                }
            }),
    )
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
        let branch_name: SharedString = self
            .repo
            .as_ref()
            .map(|r| r.branch.clone())
            .unwrap_or_else(|| "Review".into())
            .into();
        let weak = cx.entity().downgrade();
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
                        h_flex()
                            .min_w_0()
                            .gap(r(4.))
                            .child(
                                div()
                                    .min_w_0()
                                    .text_size(r(16.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .truncate()
                                    .child(branch_name),
                            )
                            .child(
                                Button::new("review-compare")
                                    .ghost()
                                    .xsmall()
                                    .label(self.compare_label())
                                    .dropdown_caret(true)
                                    .text_color(theme.muted_foreground)
                                    .text_size(r(13.))
                                    .tab_stop(false)
                                    .dropdown_menu(compare_menu(weak.clone()))
                                    .on_open_change(move |open, _, cx| {
                                        if *open && let Some(review) = weak.upgrade() {
                                            review
                                                .update(cx, |review, cx| review.load_branches(cx));
                                        }
                                    }),
                            ),
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
                ReviewScope::Commit(_) => "This commit contains no file changes.".into(),
                ReviewScope::Uncommitted => {
                    "Everything is committed. When an agent edits files, they show up here.".into()
                }
            };
            empty_state(Lucide::GitCompare, "Nothing to review", hint, &theme).into_any_element()
        } else {
            let root = self.repo.as_ref().map(|r| r.root.clone());
            let entity = cx.entity().downgrade();
            let open_root = root.clone();
            let reviewed = self.reviewed.clone();
            let fingerprints = self.fingerprints.clone();
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
                    let file_path = file.path().to_string();
                    let checked = fingerprints
                        .get(&file_path)
                        .is_some_and(|fingerprint| reviewed.get(&file_path) == Some(fingerprint));
                    let review_entity = entity.clone();
                    h_flex()
                        .gap(r(6.))
                        .child(
                            Button::new(SharedString::from(format!("reviewed-{file_path}")))
                                .ghost()
                                .xsmall()
                                .label(if checked { "Reviewed" } else { "Mark reviewed" })
                                .icon(if checked {
                                    Lucide::Check
                                } else {
                                    Lucide::Square
                                })
                                .on_click(move |_, _, cx| {
                                    cx.stop_propagation();
                                    if let Some(entity) = review_entity.upgrade() {
                                        entity.update(cx, |review, cx| {
                                            review.toggle_reviewed(&file_path, cx)
                                        });
                                    }
                                }),
                        )
                        .when(!deleted, |row| {
                            row.child(
                                div()
                                    .id(SharedString::from(format!("open-{}", file.path())))
                                    .px(r(8.))
                                    .h(r(22.))
                                    .flex()
                                    .items_center()
                                    .rounded(r(5.))
                                    .text_size(r(12.))
                                    .text_color(cx.theme().muted_foreground)
                                    .cursor_pointer()
                                    .hover(move |s| s.bg(hover))
                                    .child("Open file")
                                    .on_click(move |_, _, cx| {
                                        cx.stop_propagation();
                                        if let (Some(path), Some(entity)) =
                                            (path.clone(), entity.upgrade())
                                        {
                                            entity.update(cx, |_, cx| {
                                                cx.emit(ReviewEvent::OpenFile(path, None))
                                            });
                                        }
                                    }),
                            )
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

// Hash each raw file patch, including metadata and the comparison context.
// Equal line counts do not imply the content was already reviewed.
fn patch_fingerprints(patch: &str, context: &str) -> Vec<String> {
    if patch.is_empty() {
        return Vec::new();
    }
    patch
        .strip_prefix("diff --git ")
        .unwrap_or(patch)
        .split("\ndiff --git ")
        .map(|part| {
            let mut hash = blake3::Hasher::new();
            hash.update(context.as_bytes());
            hash.update(&[0]);
            hash.update(part.trim_end_matches('\n').as_bytes());
            hash.finalize().to_hex().to_string()
        })
        .collect()
}

#[cfg(test)]
mod progress_tests {
    use super::patch_fingerprints;
    #[test]
    fn changing_one_file_or_comparison_invalidates_only_its_review() {
        let patch = "diff --git a/a b/a\n-old\n+new\ndiff --git a/b b/b\n-one\n+two\n";
        let original = patch_fingerprints(patch, "main");
        let changed = patch_fingerprints(&patch.replace("+new", "+NEW"), "main");
        assert_ne!(original[0], changed[0]);
        assert_eq!(original[1], changed[1]);
        assert_ne!(original, patch_fingerprints(patch, "other"));
        assert!(patch_fingerprints("", "main").is_empty());
    }
}
