//! Native JSON tree and CSV table. Parsing and filtering never run in a frame.
use super::{r, workspace::FormatDocument};
use crate::structured::{self, Kind, Model};
use gpui_kit::component::{
    ActiveTheme, Icon, Sizable,
    button::Button,
    h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};
use gpui_kit::{
    AnyElement, App, AppContext, Context, Entity, FocusHandle, InteractiveElement, IntoElement,
    KeyBinding, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled,
    Subscription, Task, UniformListScrollHandle, Window, actions, div, prelude::FluentBuilder,
    uniform_list,
};
use std::{
    collections::HashSet,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

const COLUMN_PAGE: usize = 12;
actions!(structured_view, [FindData]);
pub fn init(cx: &mut App) {
    cx.bind_keys([KeyBinding::new(
        if super::is_mac() { "cmd-f" } else { "ctrl-f" },
        FindData,
        Some("DataView"),
    )]);
}

pub struct DataView {
    kind: Kind,
    source: SharedString,
    model: Option<Arc<Model>>,
    visible: Vec<usize>,
    expanded: HashSet<usize>,
    headers: bool,
    column_start: usize,
    search: Entity<InputState>,
    focus: FocusHandle,
    scroll: UniformListScrollHandle,
    error: Option<SharedString>,
    loading: bool,
    filtering: bool,
    generation: u64,
    query_generation: u64,
    cancel: Arc<AtomicBool>,
    filter_cancel: Arc<AtomicBool>,
    parse_task: Option<Task<()>>,
    filter_task: Option<Task<()>>,
    _subscription: Subscription,
}
impl Drop for DataView {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.filter_cancel.store(true, Ordering::Relaxed);
    }
}

impl DataView {
    pub fn new(
        kind: Kind,
        text: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| {
            InputState::new(window, cx).placeholder(if kind == Kind::Json {
                "Search keys and values"
            } else {
                "Search all cells"
            })
        });
        let subscription = cx.subscribe(&search, |this: &mut Self, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                this.filter(true, cx);
            }
        });
        let mut view = Self {
            kind,
            source: "".into(),
            model: None,
            visible: Vec::new(),
            expanded: HashSet::from([0]),
            headers: true,
            column_start: 0,
            search,
            focus: cx.focus_handle(),
            scroll: UniformListScrollHandle::new(),
            error: None,
            loading: false,
            filtering: false,
            generation: 0,
            query_generation: 0,
            cancel: Arc::new(AtomicBool::new(false)),
            filter_cancel: Arc::new(AtomicBool::new(false)),
            parse_task: None,
            filter_task: None,
            _subscription: subscription,
        };
        view.set_source(text, cx);
        view
    }
    #[cfg(feature = "capture")]
    pub fn capture_state(&self) -> (usize, bool, Option<SharedString>) {
        (
            self.visible.len(),
            self.loading || self.filtering,
            self.error.clone(),
        )
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.focus.focus(window, cx);
    }
    pub fn set_source(&mut self, text: SharedString, cx: &mut Context<Self>) {
        if self.generation > 0 && text == self.source {
            return;
        }
        self.cancel.store(true, Ordering::Relaxed);
        self.filter_cancel.store(true, Ordering::Relaxed);
        self.cancel = Arc::new(AtomicBool::new(false));
        self.filter_task = None;
        self.query_generation += 1;
        self.generation += 1;
        let generation = self.generation;
        self.source = text.clone();
        self.model = None;
        self.visible.clear();
        self.expanded = HashSet::from([0]);
        self.error = None;
        self.loading = true;
        self.column_start = 0;
        let kind = self.kind;
        let cancel = self.cancel.clone();
        self.parse_task = Some(cx.spawn(async move |this, cx| {
            let parsed = cx
                .background_executor()
                .spawn(async move { structured::parse(kind, &text, &cancel) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if generation != this.generation {
                    return;
                }
                this.loading = false;
                match parsed {
                    Ok(model) => {
                        this.model = Some(Arc::new(model));
                        this.filter(false, cx);
                    }
                    Err(error) => {
                        this.error = Some(error.into());
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
    fn filter(&mut self, debounce: bool, cx: &mut Context<Self>) {
        self.filter_cancel.store(true, Ordering::Relaxed);
        self.filter_cancel = Arc::new(AtomicBool::new(false));
        self.query_generation += 1;
        let generation = self.query_generation;
        let Some(model) = self.model.clone() else {
            return;
        };
        let query = self.search.read(cx).value().to_string();
        let expanded = self.expanded.clone();
        let headers = self.headers;
        let cancel = self.filter_cancel.clone();
        self.filtering = true;
        self.filter_task = Some(cx.spawn(async move |this, cx| {
            if debounce {
                cx.background_executor()
                    .timer(Duration::from_millis(150))
                    .await;
            }
            let rows = cx
                .background_executor()
                .spawn(
                    async move { structured::visible(&model, &expanded, &query, headers, &cancel) },
                )
                .await;
            let _ = this.update(cx, |this, cx| {
                if generation != this.query_generation {
                    return;
                }
                this.visible = rows;
                this.filtering = false;
                if debounce {
                    this.scroll.scroll_to_item(0, gpui_kit::ScrollStrategy::Top);
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
    fn row(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some(model) = self.model.as_deref() else {
            return div().into_any_element();
        };
        let Some(&id) = self.visible.get(ix) else {
            return div().into_any_element();
        };
        match model {
            Model::Json { source, nodes } => {
                let node = &nodes[id];
                let container = !node.children.is_empty();
                let label = node
                    .key
                    .as_ref()
                    .map(|key| short(&source[key.clone()]))
                    .unwrap_or_else(|| {
                        if node.parent.is_none() {
                            "$".into()
                        } else {
                            format!("[{}]", node.index)
                        }
                    });
                let value = if container {
                    format!(
                        "{} · {} {}",
                        if source.as_bytes()[node.value.start] == b'{' {
                            "Object"
                        } else {
                            "Array"
                        },
                        node.children.len(),
                        "items"
                    )
                } else {
                    short(&source[node.value.clone()])
                };
                let expanded =
                    self.expanded.contains(&id) || !self.search.read(cx).value().is_empty();
                h_flex()
                    .id(("json-row", id))
                    .h(r(28.))
                    .w_full()
                    .pl(r(12. + node.depth.min(16) as f32 * 16.))
                    .pr(r(12.))
                    .gap(r(8.))
                    .text_size(r(13.))
                    .hover(|s| s.bg(cx.theme().list_hover))
                    .child(
                        Icon::new(if container {
                            if expanded {
                                gpui_kit::component::IconName::ChevronDown
                            } else {
                                gpui_kit::component::IconName::ChevronRight
                            }
                        } else {
                            gpui_kit::component::IconName::Minus
                        })
                        .size(r(13.))
                        .text_color(cx.theme().muted_foreground),
                    )
                    .child(
                        div()
                            .max_w(r(280.))
                            .truncate()
                            .text_color(cx.theme().muted_foreground)
                            .child(label),
                    )
                    .child(div().flex_1().truncate().child(value))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if container {
                            if !this.expanded.remove(&id) {
                                this.expanded.insert(id);
                            }
                            this.filter(false, cx);
                        } else if let Some(Model::Json { source, nodes }) = this.model.as_deref() {
                            cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(
                                source[nodes[id].value.clone()].to_owned(),
                            ));
                        }
                    }))
                    .into_any_element()
            }
            Model::Csv { rows, columns, .. } => h_flex()
                .h(r(30.))
                .w(r(table_width(*columns, self.column_start)))
                .bg(if ix.is_multiple_of(2) {
                    cx.theme().background
                } else {
                    cx.theme().list_hover.opacity(0.35)
                })
                .child(
                    div()
                        .flex_none()
                        .w(r(58.))
                        .px(r(8.))
                        .text_size(r(12.))
                        .text_color(cx.theme().muted_foreground)
                        .child((id + 1).to_string()),
                )
                .children(
                    (self.column_start..(*columns).min(self.column_start + COLUMN_PAGE)).map(
                        |col| {
                            let cell = rows[id].get(col).map(String::as_str).unwrap_or("");
                            div()
                                .id(("csv-cell", col))
                                .flex_none()
                                .w(r(180.))
                                .h(r(30.))
                                .px(r(8.))
                                .py(r(5.))
                                .text_size(r(13.))
                                .truncate()
                                .border_l_1()
                                .border_color(cx.theme().border)
                                .child(short(cell))
                                .cursor_pointer()
                                .hover(|s| s.bg(cx.theme().list_hover))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(Model::Csv { rows, .. }) = this.model.as_deref() {
                                        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(
                                            rows[id].get(col).cloned().unwrap_or_default(),
                                        ));
                                    }
                                }))
                        },
                    ),
                )
                .into_any_element(),
        }
    }
}

fn short(value: &str) -> String {
    // Never shape megabytes of a single string or cell on the UI thread.
    let mut chars = value.chars();
    let mut text: String = chars
        .by_ref()
        .take(500)
        .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
        .collect();
    if chars.next().is_some() {
        text.push('…');
    }
    text
}
fn table_width(columns: usize, start: usize) -> f32 {
    58. + columns.saturating_sub(start).min(COLUMN_PAGE) as f32 * 180.
}

impl Render for DataView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let status = if self.loading {
            "Loading…".into()
        } else if self.filtering {
            "Searching…".into()
        } else {
            format!(
                "{} {}",
                self.visible.len(),
                if self.kind == Kind::Json {
                    "visible values"
                } else {
                    "rows"
                }
            )
        };
        let columns = match self.model.as_deref() {
            Some(Model::Csv { columns, .. }) => Some(*columns),
            _ => None,
        };
        let header = columns.map(|columns| {
            h_flex()
                .flex_none()
                .h(r(32.))
                .w(r(table_width(columns, self.column_start)))
                .bg(cx.theme().list_hover)
                .child(div().flex_none().w(r(58.)).px(r(8.)).child("Row"))
                .children(
                    (self.column_start..columns.min(self.column_start + COLUMN_PAGE)).map(|col| {
                        let label = match self.model.as_deref() {
                            Some(Model::Csv { rows, .. }) if self.headers => {
                                rows.first().and_then(|row| row.get(col)).map(|s| short(s))
                            }
                            _ => None,
                        }
                        .unwrap_or_else(|| format!("Column {}", col + 1));
                        div()
                            .flex_none()
                            .w(r(180.))
                            .px(r(8.))
                            .truncate()
                            .child(label)
                    }),
                )
        });
        v_flex()
            .key_context("DataView")
            .track_focus(&self.focus)
            .size_full()
            .on_action(cx.listener(|this, _: &FindData, window, cx| {
                this.search.update(cx, |input, cx| input.focus(window, cx))
            }))
            .child(
                h_flex()
                    .flex_none()
                    .px(r(12.))
                    .py(r(8.))
                    .gap(r(8.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .max_w(r(360.))
                            .child(Input::new(&self.search).small()),
                    )
                    .child(
                        div()
                            .text_size(r(12.))
                            .text_color(cx.theme().muted_foreground)
                            .child(status),
                    )
                    .child(div().flex_1())
                    .child(Button::new("format-data").small().label("Format").on_click(
                        |_, window, cx| window.dispatch_action(Box::new(FormatDocument), cx),
                    )),
            )
            .when_some(columns, |view, columns| {
                view.child(
                    h_flex()
                        .flex_none()
                        .px(r(12.))
                        .pb(r(6.))
                        .gap(r(8.))
                        .text_size(r(12.))
                        .child(
                            Button::new("csv-headers")
                                .small()
                                .label(if self.headers {
                                    "First row: headers"
                                } else {
                                    "First row: data"
                                })
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.headers = !this.headers;
                                    this.filter(false, cx);
                                })),
                        )
                        .child(div().text_color(cx.theme().muted_foreground).child(format!(
                            "Columns {}–{} of {columns} · click a cell to copy",
                            usize::from(columns > 0) + self.column_start,
                            columns.min(self.column_start + COLUMN_PAGE)
                        )))
                        .when(columns > COLUMN_PAGE, |bar| {
                            bar.child(
                                Button::new("csv-prev-columns")
                                    .small()
                                    .label("Previous columns")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.column_start =
                                            this.column_start.saturating_sub(COLUMN_PAGE);
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("csv-next-columns")
                                    .small()
                                    .label("Next columns")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if this.column_start + COLUMN_PAGE < columns {
                                            this.column_start += COLUMN_PAGE;
                                            cx.notify();
                                        }
                                    })),
                            )
                        }),
                )
            })
            .when_some(self.error.clone(), |view, error| {
                view.child(
                    div()
                        .p(r(16.))
                        .text_size(r(13.))
                        .text_color(cx.theme().red)
                        .child(error)
                        .child(" Switch to Source to edit the file."),
                )
            })
            .child(
                div()
                    .id("data-horizontal-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_x_scroll()
                    .child(
                        v_flex()
                            .h_full()
                            .when_some(columns, |view, columns| {
                                view.w(r(table_width(columns, self.column_start)))
                            })
                            .when(columns.is_none(), |view| view.w_full())
                            .when_some(header, |view, header| view.child(header))
                            .child(
                                uniform_list(
                                    "structured-rows",
                                    self.visible.len(),
                                    cx.processor(
                                        |this: &mut Self, range: std::ops::Range<usize>, _, cx| {
                                            range.map(|ix| this.row(ix, cx)).collect::<Vec<_>>()
                                        },
                                    ),
                                )
                                .track_scroll(&self.scroll)
                                .flex_1()
                                .min_h_0(),
                            ),
                    ),
            )
    }
}
