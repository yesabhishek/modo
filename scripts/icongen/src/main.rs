//! Render an SVG to the PNG sizes a macOS iconset needs, a multi-size Windows
//! .ico, or a single preview PNG.
use std::path::Path;

fn render(svg: &[u8], size: u32) -> resvg::tiny_skia::Pixmap {
    let tree = resvg::usvg::Tree::from_data(svg, &resvg::usvg::Options::default()).expect("valid svg");
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size, size).unwrap();
    let scale = size as f32 / tree.size().width();
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    pixmap
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let svg = std::fs::read(&args[1]).unwrap();
    let out = Path::new(&args[2]);
    std::fs::create_dir_all(out).unwrap();
    let mode = args.get(3).map(String::as_str).unwrap_or("png");
    match mode {
        "iconset" => {
            for (name, size) in [
                ("icon_16x16", 16), ("icon_16x16@2x", 32), ("icon_32x32", 32), ("icon_32x32@2x", 64),
                ("icon_128x128", 128), ("icon_128x128@2x", 256), ("icon_256x256", 256),
                ("icon_256x256@2x", 512), ("icon_512x512", 512), ("icon_512x512@2x", 1024),
            ] {
                render(&svg, size).save_png(out.join(format!("{name}.png"))).unwrap();
            }
        }
        "ico" => {
            let file = std::fs::File::create(out.join("icon.ico")).unwrap();
            let encoder = image::codecs::ico::IcoEncoder::new(file);
            let frames: Vec<_> = [16u32, 24, 32, 48, 64, 128, 256]
                .into_iter()
                .map(|size| {
                    let png = render(&svg, size).encode_png().unwrap();
                    image::codecs::ico::IcoFrame::with_encoded(png, size, size, image::ExtendedColorType::Rgba8).unwrap()
                })
                .collect();
            encoder.encode_images(&frames).unwrap();
        }
        _ => {
            let size: u32 = mode.parse().unwrap_or(512);
            render(&svg, size).save_png(out.join("preview.png")).unwrap();
        }
    }
}
