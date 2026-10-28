//! Rendered markdown for `.md` files: clean prose, with mermaid diagrams
//! drawn natively (Mermaid to SVG, then SVG to a bitmap) in the app's colors.
use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    sync::{Arc, Mutex, OnceLock},
};

use gpui_kit::component::{
    ActiveTheme,
    text::{MarkdownNode, TextView, TextViewStyle},
};
use gpui_kit::{
    App, Hsla, Image, ImageFormat, ImageSource, IntoElement, ObjectFit, ParentElement, Rgba,
    SharedString, Styled, StyledImage as _, div, img, px,
};
use markdown::mdast::Node;
use mermaid_rs_renderer as mermaid;
use resvg::{tiny_skia, usvg};

use super::r;

/// Diagrams are rasterised at this multiple of their natural size so they
/// stay sharp on high-density displays.
const SCALE: f32 = 2.0;
/// Keep one diagram's bitmap bounded, whatever the source asks for.
const MAX_SIDE: f32 = 6000.0;
const CACHE_LIMIT: usize = 64;
const BLOCK: &str = "mermaid";

pub fn is_markdown(language: &str) -> bool {
    language == "markdown"
}

/// The markdown body of a preview, ready to place in a scroll container.
pub fn body(id: impl Into<SharedString>, text: SharedString, cx: &App) -> impl IntoElement {
    let theme = cx.theme().clone();
    let id: SharedString = id.into();
    div()
        .w_full()
        .max_w(px(780.))
        .mx_auto()
        .px(r(32.))
        .pt(r(20.))
        .pb(r(80.))
        .text_size(r(15.))
        .text_color(theme.foreground)
        .child(
            TextView::markdown(id, text)
                .selectable(true)
                .style(
                    TextViewStyle::default()
                        .paragraph_gap(gpui_kit::rems(0.9))
                        .heading_font_size(|level, base| match level {
                            1 => base * 2.0,
                            2 => base * 1.55,
                            3 => base * 1.25,
                            _ => base * 1.05,
                        }),
                )
                .markdown_block_parser(parse_block)
                .markdown_block_renderer(BLOCK, render_block),
        )
}

/// A fenced block tagged `mermaid` becomes a diagram instead of a code block.
fn parse_block(
    node: &Node,
    _: &gpui_kit::component::text::MarkdownParseContext<'_>,
) -> Option<MarkdownNode> {
    let Node::Code(code) = node else {
        return None;
    };
    let lang = code.lang.as_deref()?.trim();
    if !lang.eq_ignore_ascii_case(BLOCK) {
        return None;
    }
    let source = SharedString::from(code.value.clone());
    Some(MarkdownNode::new(BLOCK, Diagram { source: source.clone() }).text(source))
}

struct Diagram {
    source: SharedString,
}

fn render_block(
    node: &MarkdownNode,
    _: &mut gpui_kit::Window,
    cx: &mut App,
) -> impl IntoElement + use<> {
    let theme = cx.theme().clone();
    let source = node
        .data::<Diagram>()
        .map(|d| d.source.clone())
        .unwrap_or_default();
    let palette = Palette::from_theme(cx);
    let frame = div()
        .w_full()
        .my(r(8.))
        .px(r(16.))
        .py(r(16.))
        .rounded(r(8.))
        .border_1()
        .border_color(theme.border);
    match render_cached(&source, &palette) {
        Ok(bitmap) => frame.flex().justify_center().child(
            img(ImageSource::Image(bitmap.image))
                .w(px(bitmap.width))
                .max_w_full()
                .object_fit(ObjectFit::Contain),
        ),
        Err(message) => frame
            .bg(theme.muted)
            .child(
                div()
                    .text_size(r(12.5))
                    .text_color(theme.muted_foreground)
                    .child(format!("Couldn't draw this diagram: {message}")),
            )
            .child(
                div()
                    .mt(r(8.))
                    .font_family(theme.mono_font_family.clone())
                    .text_size(r(12.5))
                    .child(source),
            ),
    }
}

#[derive(Clone)]
struct Bitmap {
    image: Arc<Image>,
    /// Natural size in logical pixels.
    width: f32,
}

type Rendered = Result<Bitmap, String>;

fn cache() -> &'static Mutex<HashMap<u64, Rendered>> {
    static CACHE: OnceLock<Mutex<HashMap<u64, Rendered>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

fn render_cached(source: &str, palette: &Palette) -> Rendered {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    source.hash(&mut hasher);
    palette.hash(&mut hasher);
    let key = hasher.finish();
    if let Some(found) = cache().lock().ok().and_then(|c| c.get(&key).cloned()) {
        return found;
    }
    let rendered = rasterize(source, palette);
    if let Ok(mut cache) = cache().lock() {
        if cache.len() >= CACHE_LIMIT {
            cache.clear();
        }
        cache.insert(key, rendered.clone());
    }
    rendered
}

fn fonts() -> Arc<usvg::fontdb::Database> {
    static FONTS: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    FONTS
        .get_or_init(|| {
            let mut db = usvg::fontdb::Database::new();
            db.load_system_fonts();
            Arc::new(db)
        })
        .clone()
}

/// Load system fonts off the UI thread so the first diagram appears quickly.
pub fn warm_up() {
    std::thread::spawn(|| {
        fonts();
    });
}

fn rasterize(source: &str, palette: &Palette) -> Rendered {
    let options = mermaid::RenderOptions {
        theme: palette.mermaid_theme(),
        layout: Default::default(),
    };
    let svg = mermaid::render_with_options(source, options).map_err(|e| e.to_string())?;
    let parse = usvg::Options {
        fontdb: fonts(),
        font_family: FONT_FAMILY.split(',').next().unwrap_or("Arial").trim().into(),
        ..Default::default()
    };
    let tree = usvg::Tree::from_str(&svg, &parse).map_err(|e| e.to_string())?;
    let size = tree.size();
    let (width, height) = (size.width(), size.height());
    if !(width > 0.0 && height > 0.0) {
        return Err("the diagram is empty".into());
    }
    let scale = SCALE.min(MAX_SIDE / width).min(MAX_SIDE / height);
    let mut pixmap = tiny_skia::Pixmap::new(
        (width * scale).ceil().max(1.0) as u32,
        (height * scale).ceil().max(1.0) as u32,
    )
    .ok_or("the diagram is too large")?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    let png = pixmap.encode_png().map_err(|e| e.to_string())?;
    Ok(Bitmap {
        image: Arc::new(Image::from_bytes(ImageFormat::Png, png)),
        width,
    })
}

const FONT_FAMILY: &str = if cfg!(target_os = "macos") {
    "Helvetica Neue, Helvetica, Arial, sans-serif"
} else if cfg!(target_os = "windows") {
    "Segoe UI, Arial, sans-serif"
} else {
    "Inter, DejaVu Sans, Liberation Sans, Arial, sans-serif"
};

/// The few colors a diagram needs, taken from the active Modo theme.
#[derive(Hash)]
struct Palette {
    background: String,
    text: String,
    line: String,
    border: String,
    node: String,
    node_alt: String,
    cluster: String,
    cluster_border: String,
    note: String,
    note_border: String,
}

impl Palette {
    fn from_theme(cx: &App) -> Self {
        let theme = cx.theme();
        let (bg, fg) = (theme.background, theme.foreground);
        Self {
            background: hex(bg),
            text: hex(fg),
            line: hex(mix(fg, bg, 0.55)),
            border: hex(mix(fg, bg, 0.32)),
            node: hex(mix(fg, bg, 0.05)),
            node_alt: hex(mix(fg, bg, 0.10)),
            cluster: hex(mix(fg, bg, 0.03)),
            cluster_border: hex(mix(fg, bg, 0.16)),
            note: hex(mix(theme.yellow, bg, 0.22)),
            note_border: hex(mix(theme.yellow, bg, 0.6)),
        }
    }

    fn mermaid_theme(&self) -> mermaid::Theme {
        let mut t = mermaid::Theme::modern();
        t.font_family = FONT_FAMILY.into();
        t.font_size = 14.0;
        t.background = "transparent".into();
        t.primary_color = self.node.clone();
        t.primary_text_color = self.text.clone();
        t.primary_border_color = self.border.clone();
        t.line_color = self.line.clone();
        t.secondary_color = self.node_alt.clone();
        t.tertiary_color = self.background.clone();
        t.edge_label_background = self.background.clone();
        t.cluster_background = self.cluster.clone();
        t.cluster_border = self.cluster_border.clone();
        t.text_color = self.text.clone();
        t.sequence_actor_fill = self.node.clone();
        t.sequence_actor_border = self.border.clone();
        t.sequence_actor_line = self.line.clone();
        t.sequence_note_fill = self.note.clone();
        t.sequence_note_border = self.note_border.clone();
        t.sequence_activation_fill = self.node_alt.clone();
        t.sequence_activation_border = self.border.clone();
        t.pie_title_text_color = self.text.clone();
        t.pie_section_text_color = self.text.clone();
        t.pie_legend_text_color = self.text.clone();
        t.pie_stroke_color = self.background.clone();
        t
    }
}

/// `fg` laid over `bg` at `alpha`, as an opaque color.
fn mix(fg: Hsla, bg: Hsla, alpha: f32) -> Rgba {
    let (f, b) = (Rgba::from(fg), Rgba::from(bg));
    let lerp = |a: f32, b: f32| b + (a - b) * alpha;
    Rgba {
        r: lerp(f.r, b.r),
        g: lerp(f.g, b.g),
        b: lerp(f.b, b.b),
        a: 1.0,
    }
}

fn hex(color: impl Into<Rgba>) -> String {
    let c: Rgba = color.into();
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", byte(c.r), byte(c.g), byte(c.b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixes_toward_the_background() {
        let black = gpui_kit::hsla(0., 0., 0., 1.);
        let white = gpui_kit::hsla(0., 0., 1., 1.);
        assert_eq!(hex(mix(black, white, 0.0)), "#ffffff");
        assert_eq!(hex(mix(black, white, 1.0)), "#000000");
    }

    #[test]
    fn draws_a_flowchart() {
        let palette = Palette {
            background: "#ffffff".into(),
            text: "#111111".into(),
            line: "#888888".into(),
            border: "#aaaaaa".into(),
            node: "#f5f5f5".into(),
            node_alt: "#eeeeee".into(),
            cluster: "#fafafa".into(),
            cluster_border: "#dddddd".into(),
            note: "#fff7d6".into(),
            note_border: "#e6c200".into(),
        };
        let bitmap = rasterize("flowchart LR; A[Start] --> B{Ok?}; B --> C[Done]", &palette)
            .expect("renders");
        assert!(bitmap.width > 50.0);
        assert!(!bitmap.image.bytes.is_empty());
    }

    #[test]
    fn rejects_garbage_without_panicking() {
        let palette = Palette {
            background: "#fff".into(),
            text: "#000".into(),
            line: "#000".into(),
            border: "#000".into(),
            node: "#fff".into(),
            node_alt: "#fff".into(),
            cluster: "#fff".into(),
            cluster_border: "#000".into(),
            note: "#fff".into(),
            note_border: "#000".into(),
        };
        assert!(rasterize("this is not a diagram", &palette).is_err());
    }
}
