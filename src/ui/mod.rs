//! Modo's interface. Sizes are written in pixels at the 14px base and
//! converted to rems, so the interface zoom setting scales everything.
pub mod editor;
pub mod palette;
pub mod preview;
pub mod review;
pub mod settings_view;
pub mod sidebar;
pub mod terminal;
pub mod welcome;
pub mod workspace;

use gpui_kit::component::{ActiveTheme, Icon, IconNamed, StyledExt as _, h_flex};
use gpui_kit::{
    AnyElement, App, ClickEvent, Div, ElementId, InteractiveElement, IntoElement, MouseButton,
    ParentElement, Rems, RenderOnce, SharedString, Stateful, StatefulInteractiveElement, Styled,
    Window, WindowControlArea, div, prelude::FluentBuilder, rems,
};

/// Pixels at the default 14px interface size, as rems.
pub fn r(px_at_base: f32) -> Rems {
    rems(px_at_base / 14.0)
}

pub fn is_mac() -> bool {
    cfg!(target_os = "macos")
}

/// A shortcut label in the platform's notation.
pub fn shortcut(mac: &'static str, other: &'static str) -> &'static str {
    if is_mac() { mac } else { other }
}

/// Muted keyboard hint, Notion style (no boxes, just quiet glyphs).
pub fn kbd(keys: &'static str, cx: &App) -> Div {
    div()
        .text_size(r(11.5))
        .text_color(cx.theme().muted_foreground.opacity(0.8))
        .child(keys)
}

/// The small uppercase-free label above a sidebar or settings section.
pub fn section_label(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .text_size(r(12.))
        .font_medium()
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
}

/// A square, borderless icon button with a soft hover fill.
pub fn icon_button(id: impl Into<ElementId>, icon: impl IconNamed, cx: &App) -> Stateful<Div> {
    let hover = cx.theme().list_hover;
    let active = cx.theme().list_active;
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(r(26.))
        .rounded(r(6.))
        .text_color(cx.theme().muted_foreground)
        .hover(move |s| s.bg(hover))
        .active(move |s| s.bg(active))
        .cursor_pointer()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(Icon::new(icon).size(r(16.)))
}

type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

/// A full-width sidebar or menu row with an icon, label, and optional trailing content.
#[derive(IntoElement)]
pub struct NavRow {
    id: ElementId,
    icon: Option<Icon>,
    label: SharedString,
    trailing: Option<AnyElement>,
    selected: bool,
    on_click: Option<ClickHandler>,
}

impl NavRow {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            icon: None,
            label: label.into(),
            trailing: None,
            selected: false,
            on_click: None,
        }
    }
    pub fn icon(mut self, icon: impl IconNamed) -> Self {
        self.icon = Some(Icon::new(icon));
        self
    }
    pub fn trailing(mut self, element: impl IntoElement) -> Self {
        self.trailing = Some(element.into_any_element());
        self
    }
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }
    pub fn on_click(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Box::new(f));
        self
    }
}

impl RenderOnce for NavRow {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme().clone();
        let hover = theme.list_hover;
        let fg = if self.selected {
            theme.foreground
        } else {
            theme.sidebar_foreground
        };
        h_flex()
            .id(self.id)
            .h(r(28.))
            .mx(r(6.))
            .px(r(8.))
            .gap(r(8.))
            .rounded(r(6.))
            .text_size(r(13.5))
            .text_color(fg)
            .cursor_pointer()
            .when(self.selected, |row| row.bg(theme.list_active).font_medium())
            .when(!self.selected, |row| row.hover(move |s| s.bg(hover)))
            .when_some(self.icon, |row, icon| {
                row.child(icon.size(r(16.)).text_color(theme.muted_foreground))
            })
            .child(div().flex_1().truncate().child(self.label))
            .when_some(self.trailing, |row, trailing| row.child(trailing))
            .when_some(self.on_click, |row, on_click| row.on_click(on_click))
    }
}

/// Makes an area behave like a title bar: drag to move, double-click to zoom.
pub fn drag_region(id: impl Into<ElementId>) -> Stateful<Div> {
    div()
        .id(id)
        .window_control_area(WindowControlArea::Drag)
        .on_mouse_down(MouseButton::Left, |event, window, cx| {
            if event.click_count >= 2 {
                if is_mac() {
                    window.titlebar_double_click();
                } else {
                    window.zoom_window();
                }
            } else {
                window.start_window_move();
            }
            cx.stop_propagation();
        })
}
