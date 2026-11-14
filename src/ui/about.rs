//! Shared product identity and links for the About dialog and Settings page.
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::{
    ActiveTheme, Icon, Sizable, StyledExt as _, WindowExt,
    button::{Button, ButtonVariants},
    h_flex, v_flex,
};
use gpui_kit::{
    AnyElement, App, FontWeight, IntoElement, ParentElement, SharedString, Styled, Window, div, px,
    relative,
};

use super::r;
use crate::brand;

pub fn identity(cx: &App) -> AnyElement {
    let theme = cx.theme();
    v_flex()
        .w_full()
        .min_w_0()
        .gap(r(16.))
        .child(
            h_flex()
                .gap(r(14.))
                .child(super::welcome::logo(r(56.), cx))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap(r(6.))
                        .child(
                            h_flex()
                                .gap(r(10.))
                                .child(
                                    div()
                                        .text_size(r(26.))
                                        .font_weight(FontWeight::BOLD)
                                        .child(brand::NAME),
                                )
                                .child(
                                    div()
                                        .flex_none()
                                        .px(r(7.))
                                        .py(r(2.))
                                        .rounded(r(6.))
                                        .bg(theme.list_hover)
                                        .text_size(r(11.5))
                                        .text_color(theme.muted_foreground)
                                        .child(format!("Version {}", brand::VERSION)),
                                ),
                        )
                        .child(
                            div()
                                .text_size(r(12.))
                                .text_color(theme.muted_foreground)
                                .child("A quiet workspace for code."),
                        ),
                ),
        )
        .child(
            div()
                .text_size(r(14.))
                .line_height(relative(1.5))
                .text_color(theme.muted_foreground)
                .child(brand::TAGLINE),
        )
        .into_any_element()
}

pub fn publisher(cx: &App) -> AnyElement {
    let domain = brand::PUBLISHER_URL
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    v_flex()
        .w_full()
        .min_w_0()
        .gap(r(8.))
        .child(label("Published by", cx))
        .child(link(
            "about-publisher",
            Lucide::CircleDot,
            brand::PUBLISHER,
            domain,
            brand::PUBLISHER_URL,
            cx,
        ))
        .into_any_element()
}

fn label(text: &'static str, cx: &App) -> AnyElement {
    div()
        .text_size(r(12.))
        .font_medium()
        .text_color(cx.theme().muted_foreground)
        .child(text)
        .into_any_element()
}

pub fn resources(cx: &App) -> AnyElement {
    h_flex()
        .w_full()
        .min_w_0()
        .gap(r(8.))
        .child(
            Button::new("about-source")
                .outline()
                .small()
                .flex_1()
                .min_w_0()
                .icon(
                    Icon::new(Lucide::Code)
                        .size(r(14.))
                        .text_color(cx.theme().muted_foreground),
                )
                .label("Source code")
                .tooltip("View Modo on GitHub")
                .on_click(|_, _, cx| cx.open_url(brand::REPOSITORY)),
        )
        .child(
            Button::new("about-releases")
                .outline()
                .small()
                .flex_1()
                .min_w_0()
                .icon(
                    Icon::new(Lucide::Clock)
                        .size(r(14.))
                        .text_color(cx.theme().muted_foreground),
                )
                .label("Release notes")
                .on_click(|_, _, cx| cx.open_url(&format!("{}/releases", brand::REPOSITORY))),
        )
        .into_any_element()
}

fn link(
    id: &'static str,
    icon: Lucide,
    title: &'static str,
    detail: &str,
    url: &str,
    cx: &App,
) -> AnyElement {
    let detail: SharedString = detail.to_owned().into();
    let url = url.to_owned();
    let theme = cx.theme();
    Button::new(id)
        .ghost()
        .w_full()
        .h(r(56.))
        .px(r(12.))
        .rounded(px(8.))
        .bg(theme.list_hover)
        .child(
            h_flex()
                .w_full()
                .min_w_0()
                .gap(r(10.))
                .child(
                    Icon::new(icon)
                        .size(r(17.))
                        .flex_none()
                        .text_color(theme.muted_foreground),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap(r(3.))
                        .child(
                            div()
                                .truncate()
                                .text_size(r(13.))
                                .font_medium()
                                .child(title),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_size(r(12.))
                                .text_color(theme.muted_foreground)
                                .child(detail),
                        ),
                )
                .child(
                    Icon::new(Lucide::ExternalLink)
                        .size(r(14.))
                        .flex_none()
                        .text_color(theme.muted_foreground),
                ),
        )
        .on_click(move |_, _, cx| cx.open_url(&url))
        .into_any_element()
}

pub fn open(window: &mut Window, cx: &mut App) {
    if window.has_active_dialog(cx) {
        return;
    }
    window.open_dialog(cx, |dialog, window, cx| {
        let width = r(440.).to_pixels(window.rem_size());
        dialog
            .title(
                div()
                    .text_size(r(12.))
                    .text_color(cx.theme().muted_foreground)
                    .child("About Modo"),
            )
            .w(width)
            .centered()
            .p(r(24.))
            .rounded(r(12.))
            .bg(cx.theme().popover)
            .close_button(true)
            .child(
                v_flex()
                    .w_full()
                    .min_w_0()
                    .gap(r(22.))
                    .child(identity(cx))
                    .child(publisher(cx))
                    .child(resources(cx)),
            )
            .footer(
                h_flex()
                    .w_full()
                    .gap(r(12.))
                    .child(
                        div()
                            .flex_1()
                            .text_size(r(11.5))
                            .text_color(cx.theme().muted_foreground)
                            .child("Built with Rust and GPUI · MIT license"),
                    )
                    .child(
                        Button::new("about-done")
                            .outline()
                            .small()
                            .w(r(72.))
                            .label("Done")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    ),
            )
    });
}
