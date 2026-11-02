//! A project-specific color wash across the workspace. A canvas paints without
//! adding a hitbox, so the treatment cannot intercept editing or terminal input.
use gpui_kit::{Hsla, IntoElement, Styled, canvas, fill, linear_color_stop, linear_gradient};

pub fn render(color: Hsla, dark: bool) -> impl IntoElement {
    let opacity = if dark { 0.11 } else { 0.065 };
    let secondary = Hsla {
        h: (color.h + 0.18) % 1.,
        ..color
    };
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            window.paint_quad(fill(
                bounds,
                linear_gradient(
                    135.,
                    linear_color_stop(color.opacity(opacity), 0.),
                    linear_color_stop(color.opacity(0.), 0.8),
                ),
            ));
            window.paint_quad(fill(
                bounds,
                linear_gradient(
                    315.,
                    linear_color_stop(secondary.opacity(opacity * 0.7), 0.),
                    linear_color_stop(secondary.opacity(0.), 0.8),
                ),
            ));
        },
    )
    .absolute()
    .inset_0()
    .size_full()
}
