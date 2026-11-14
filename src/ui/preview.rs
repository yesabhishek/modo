//! Rendered markdown for `.md` files: clean prose, front matter as a small
//! table, images and links resolved beside the file, and mermaid diagrams
//! drawn natively (Mermaid to SVG, then SVG to a bitmap) in the app's colors.
use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};

use gpui_kit::component::{
    ActiveTheme,
    text::{FrontmatterPlugin, MarkdownExtensions, MarkdownNode, TextView, TextViewStyle},
};
use gpui_kit::{
    App, FontWeight, Hsla, ImageSource, IntoElement, ObjectFit, ParentElement, RenderImage, Rgba,
    SharedString, SharedUri, StyleRefinement, Styled, StyledImage as _, WeakEntity, div, img, px,
};
use markdown::mdast::Node;
use mermaid_rs_renderer as mermaid;
use resvg::{tiny_skia, usvg};

use super::r;

/// Diagrams are rasterised at this multiple of their natural size so they
/// stay sharp on high-density displays.
const SCALE: f32 = 2.0;
/// Keep one diagram's bitmap bounded, whatever the source asks for: no side
/// longer than this, and no more than `MAX_PIXELS` in all (32 MB of pixels).
const MAX_SIDE: f32 = 6000.0;
const MAX_PIXELS: f32 = 8_000_000.0;
/// Finished diagrams kept in memory, by pixel bytes.
const CACHE_BYTES: usize = 96 * 1024 * 1024;
const BLOCK: &str = "mermaid";

pub fn is_markdown(language: &str) -> bool {
    language == "markdown"
}

/// Where a link in a document leads.
#[derive(Debug, PartialEq)]
enum LinkTarget {
    /// A web page or an email address, for the browser or the mail client.
    External(String),
    /// A file in the project, with an optional `#L12` line.
    File(PathBuf, Option<u32>),
    /// A heading in the same document.
    Anchor,
    /// Anything else: another URL scheme (`file:`, an app's own scheme), a
    /// network share, or a path that climbs out of the project. A document
    /// is untrusted input, so these are neither opened nor loaded.
    Blocked,
}

/// URL schemes a document may hand to the operating system.
const OPEN_SCHEMES: [&str; 3] = ["http", "https", "mailto"];

/// Resolve a link the way a repository browser does: `/docs/a.md` from the
/// project root, anything else from the folder of the document it is in.
/// Files resolve only inside the project (or, for a file opened on its own,
/// inside its folder).
fn link_target(href: &str, document: &Path, root: Option<&Path>) -> LinkTarget {
    let href = href.trim();
    if href.starts_with('#') || href.is_empty() {
        return LinkTarget::Anchor;
    }
    // `scheme:` before any slash. One letter is a Windows drive, not a scheme.
    if let Some((scheme, _)) = href.split_once(':')
        && scheme.len() > 1
        && !scheme.contains(['/', '.', '\\'])
    {
        return if OPEN_SCHEMES
            .iter()
            .any(|allowed| scheme.eq_ignore_ascii_case(allowed))
        {
            LinkTarget::External(href.to_owned())
        } else {
            LinkTarget::Blocked
        };
    }
    let (path, fragment) = match href.split_once('#') {
        Some((path, fragment)) => (path, Some(fragment)),
        None => (href, None),
    };
    let path = path.split('?').next().unwrap_or(path);
    let line = fragment
        .and_then(|fragment| fragment.strip_prefix('L'))
        .and_then(|line| line.split('-').next())
        .and_then(|line| line.parse().ok());
    match resolve(&percent_decode(path), document, root) {
        Some(path) => LinkTarget::File(path, line),
        None => LinkTarget::Blocked,
    }
}

/// The file a relative link names, or `None` when it leaves the project.
fn resolve(relative: &str, document: &Path, root: Option<&Path>) -> Option<PathBuf> {
    // `//host/share` and `\\host\share` reach across the network on Windows.
    if relative.starts_with("//") || relative.starts_with('\\') || relative.contains('\0') {
        return None;
    }
    let folder = document.parent().unwrap_or(Path::new("."));
    let boundary = root.unwrap_or(folder);
    let joined = match relative.strip_prefix('/') {
        Some(from_root) => boundary.join(from_root),
        None => folder.join(relative),
    };
    let path = normalize(&joined);
    path.starts_with(normalize(boundary)).then_some(path)
}

/// Remove `.` and `..` without touching the disk, so containment can be
/// checked before anything is read.
fn normalize(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    return PathBuf::new();
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Some(hex) = text.get(i + 1..i + 3)
            && let Ok(byte) = u8::from_str_radix(hex, 16)
        {
            decoded.push(byte);
            i += 3;
        } else {
            decoded.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

/// The markdown body of a preview, ready to place in a scroll container.
/// `document` is the file being shown: its images and links resolve from there.
pub fn body(
    id: impl Into<SharedString>,
    text: SharedString,
    document: &Path,
    root: Option<&Path>,
    workspace: WeakEntity<super::workspace::Workspace>,
    cx: &App,
) -> impl IntoElement {
    let theme = cx.theme().clone();
    let id: SharedString = id.into();
    let mut code_block = StyleRefinement::default();
    code_block.text.font_size = Some(super::r(13.).into());
    let table_head = StyleRefinement::default().font_weight(FontWeight::MEDIUM);
    let image_document = document.to_path_buf();
    let image_root = root.map(Path::to_path_buf);
    let link_document = document.to_path_buf();
    let link_root = root.map(Path::to_path_buf);
    div()
        .w_full()
        .max_w(px(760.))
        .mx_auto()
        .px(r(32.))
        .pt(r(18.))
        .pb(r(80.))
        .text_size(r(15.))
        .line_height(gpui_kit::relative(1.6))
        .text_color(theme.foreground)
        .child(
            TextView::markdown(id, text)
                .selectable(true)
                .style(
                    TextViewStyle::default()
                        .paragraph_gap(gpui_kit::rems(0.85))
                        .heading_font_size(|level, base| match level {
                            1 => base * 1.9,
                            2 => base * 1.45,
                            3 => base * 1.2,
                            _ => base * 1.05,
                        })
                        .code_block(code_block)
                        .table_head(table_head),
                )
                // Pictures are read from the project only. Modo has no HTTP
                // client, so a web address never loads (and never reports
                // to a tracker that the document was opened).
                .image_source(move |uri| {
                    match link_target(uri.as_ref(), &image_document, image_root.as_deref()) {
                        LinkTarget::File(path, _) => ImageSource::from(path),
                        _ => ImageSource::from(SharedUri::from("about:blank")),
                    }
                })
                // Links to project files open in Modo; web and mail links go
                // to the browser; anything else is refused.
                .on_link_click(move |href, _, window, cx| {
                    let target = link_target(href, &link_document, link_root.as_deref());
                    let _ = workspace.update(cx, |workspace, cx| match target {
                        LinkTarget::External(url) => cx.open_url(&url),
                        LinkTarget::Anchor => {}
                        LinkTarget::File(path, line) => {
                            workspace.open_linked(path, line, window, cx)
                        }
                        LinkTarget::Blocked => workspace.notify_error(
                            "Modo opens web and mail links, and files inside this project.",
                            window,
                            cx,
                        ),
                    });
                })
                .markdown_extensions(MarkdownExtensions::default().frontmatter())
                .markdown_block_parser(parse_block)
                .markdown_block_renderer(BLOCK, render_block)
                .plugin(FrontmatterPlugin::new()),
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
    Some(
        MarkdownNode::new(
            BLOCK,
            Diagram {
                source: source.clone(),
            },
        )
        .text(source),
    )
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
    match diagram(&source, palette, cx) {
        Some(Ok(bitmap)) => frame.flex().justify_center().child(
            img(ImageSource::Render(bitmap.image))
                .w(px(bitmap.width))
                .max_w_full()
                .object_fit(ObjectFit::Contain),
        ),
        Some(Err(message)) => frame
            .bg(theme.muted)
            .child(
                div()
                    .text_size(r(12.))
                    .text_color(theme.muted_foreground)
                    .child(format!("Couldn't draw this diagram: {message}")),
            )
            .child(
                div()
                    .mt(r(8.))
                    .font_family(theme.mono_font_family.clone())
                    .text_size(r(12.))
                    .child(source),
            ),
        // Still being drawn on a background thread.
        None => frame
            .h(r(120.))
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .text_size(r(12.))
                    .text_color(theme.muted_foreground)
                    .child("Drawing diagram…"),
            ),
    }
}

#[derive(Clone)]
struct Bitmap {
    image: Arc<RenderImage>,
    /// Natural size in logical pixels.
    width: f32,
    /// Memory the pixels take.
    bytes: usize,
}

type Rendered = Result<Bitmap, String>;

/// `None` marks a diagram that is being drawn.
fn cache() -> &'static Mutex<HashMap<u64, Option<Rendered>>> {
    static CACHE: OnceLock<Mutex<HashMap<u64, Option<Rendered>>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// The finished diagram, or `None` while it is drawn off the UI thread.
/// Layout, rasterising, and the first font scan can take a noticeable
/// fraction of a second, which the page should not wait for.
fn diagram(source: &SharedString, palette: Palette, cx: &mut App) -> Option<Rendered> {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    source.hash(&mut hasher);
    palette.hash(&mut hasher);
    let key = hasher.finish();
    {
        let mut cache = cache().lock().ok()?;
        if let Some(found) = cache.get(&key) {
            return found.clone();
        }
        cache.insert(key, None);
    }
    let source = source.clone();
    cx.spawn(async move |cx| {
        let rendered = cx
            .background_executor()
            .spawn(async move { rasterize(&source, &palette) })
            .await;
        if let Ok(mut cache) = cache().lock() {
            let held: usize = cache
                .values()
                .filter_map(|slot| slot.as_ref()?.as_ref().ok())
                .map(|bitmap| bitmap.bytes)
                .sum();
            let adding = rendered.as_ref().map_or(0, |bitmap| bitmap.bytes);
            if held + adding > CACHE_BYTES {
                // Drop the finished ones; those in flight fill themselves in.
                cache.retain(|_, slot| slot.is_none());
            }
            cache.insert(key, Some(rendered));
        }
        cx.update(|cx| cx.refresh_windows());
    })
    .detach();
    None
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

fn rasterize(source: &str, palette: &Palette) -> Rendered {
    let options = mermaid::RenderOptions {
        theme: palette.mermaid_theme(),
        layout: Default::default(),
    };
    let svg = mermaid::render_with_options(source, options).map_err(|e| e.to_string())?;
    let parse = usvg::Options {
        fontdb: fonts(),
        font_family: FONT_FAMILY
            .split(',')
            .next()
            .unwrap_or("Arial")
            .trim()
            .into(),
        ..Default::default()
    };
    let tree = usvg::Tree::from_str(&svg, &parse).map_err(|e| e.to_string())?;
    let size = tree.size();
    let (width, height) = (size.width(), size.height());
    if !(width > 0.0 && height > 0.0) {
        return Err("the diagram is empty".into());
    }
    let scale = SCALE
        .min(MAX_SIDE / width)
        .min(MAX_SIDE / height)
        .min((MAX_PIXELS / (width * height)).sqrt());
    let (columns, rows) = (
        (width * scale).ceil().max(1.0) as u32,
        (height * scale).ceil().max(1.0) as u32,
    );
    let mut pixmap = tiny_skia::Pixmap::new(columns, rows).ok_or("the diagram is too large")?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    // Straight to the BGRA GPUI uploads, without a PNG round trip.
    let mut pixels = Vec::with_capacity(pixmap.data().len());
    for pixel in pixmap.pixels() {
        let color = pixel.demultiply();
        pixels.extend_from_slice(&[color.blue(), color.green(), color.red(), color.alpha()]);
    }
    let bytes = pixels.len();
    let buffer = image::RgbaImage::from_raw(columns, rows, pixels).ok_or("the diagram is empty")?;
    Ok(Bitmap {
        image: Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])),
        width,
        bytes,
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
    fn links_resolve_like_a_repository_browser() {
        let document = Path::new("/project/docs/guide.md");
        let root = Some(Path::new("/project"));
        assert_eq!(
            link_target("./api.md", document, root),
            LinkTarget::File("/project/docs/api.md".into(), None)
        );
        assert_eq!(
            link_target("../src/lib.rs", document, root),
            LinkTarget::File("/project/src/lib.rs".into(), None)
        );
        assert_eq!(
            link_target("/src/main.rs#L42-L50", document, root),
            LinkTarget::File("/project/src/main.rs".into(), Some(42))
        );
        assert_eq!(
            link_target("My%20Notes.md?plain=1", document, root),
            LinkTarget::File("/project/docs/My Notes.md".into(), None)
        );
        assert_eq!(link_target("#usage", document, root), LinkTarget::Anchor);
        for url in ["https://example.com/a#b", "mailto:me@example.com"] {
            assert_eq!(
                link_target(url, document, root),
                LinkTarget::External(url.into())
            );
        }
        // A Windows drive letter is a path, not a URL scheme.
        assert!(!matches!(
            link_target("C:/notes.md", document, None),
            LinkTarget::External(_)
        ));
    }

    #[test]
    fn documents_cannot_reach_outside_the_project_or_launch_apps() {
        let document = Path::new("/project/docs/guide.md");
        let root = Some(Path::new("/project"));
        for href in [
            "file:///Applications/Calculator.app",
            "FILE:///etc/passwd",
            "vscode://open?file=/etc/passwd",
            "javascript:alert(1)",
            "smb://server/share",
            "../../etc/passwd",
            "/../../etc/passwd",
            "//attacker/share/pixel.png",
            "\\\\attacker\\share\\pixel.png",
            "%2e%2e/%2e%2e/etc/passwd",
        ] {
            assert_eq!(
                link_target(href, document, root),
                LinkTarget::Blocked,
                "{href}"
            );
        }
        assert_eq!(
            link_target("HTTPS://example.com", document, root),
            LinkTarget::External("HTTPS://example.com".into())
        );
        // A file opened on its own may link beside itself, not above.
        assert_eq!(
            link_target("notes.md", document, None),
            LinkTarget::File("/project/docs/notes.md".into(), None)
        );
        assert_eq!(
            link_target("../secret.md", document, None),
            LinkTarget::Blocked
        );
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
        assert!(bitmap.bytes > 0);
        assert_eq!(
            bitmap.image.as_bytes(0).map(<[u8]>::len),
            Some(bitmap.bytes)
        );
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
