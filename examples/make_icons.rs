//! Turns the generated artwork into the app's icons, once, whenever the artwork changes:
//!
//!   cargo run --example make_icons
//!
//! assets/icon-source.jpg is the app icon on a backdrop: the blue rounded square is found, cut out and given
//! transparent corners. assets/tray-source.jpg is black on white and becomes black on transparent.

use image::{ImageFormat, Rgba, RgbaImage, imageops::FilterType};

fn main() {
    let source = image::open("assets/icon-source.jpg").expect("assets/icon-source.jpg").to_rgba8();
    let icon = cut_out(&source);
    icon.save("assets/icon.png").unwrap();
    image::imageops::resize(&icon, 256, 256, FilterType::Lanczos3).save("assets/icon-256.png").unwrap();
    image::imageops::resize(&icon, 64, 64, FilterType::Lanczos3).save("assets/icon-64.png").unwrap();
    // Windows wants several sizes in one file; 256 is the largest an ICO holds.
    let ico = image::imageops::resize(&icon, 256, 256, FilterType::Lanczos3);
    ico.save_with_format("assets/icon.ico", ImageFormat::Ico).unwrap();

    let tray = image::open("assets/tray-source.jpg").expect("assets/tray-source.jpg").to_rgba8();
    let mut tray = image::imageops::resize(&tray, 64, 64, FilterType::Lanczos3);
    for p in tray.pixels_mut() {
        let light = (u16::from(p[0]) + u16::from(p[1]) + u16::from(p[2])) / 3;
        *p = Rgba([0, 0, 0, 255 - light as u8]);
    }
    tray.save("assets/tray.png").unwrap();
    println!("assets/icon.png, icon-256.png, icon-64.png, icon.ico and tray.png written");
}

/// The blue square: the bounding box of the strongly blue pixels, made square, with rounded corners.
fn cut_out(img: &RgbaImage) -> RgbaImage {
    let blue = |p: &Rgba<u8>| p[2] > 150 && i16::from(p[2]) - i16::from(p[0]) > 80;
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
    for (x, y, p) in img.enumerate_pixels() {
        // Rows and columns need a run of blue, not a stray pixel of the backdrop.
        if blue(p) && x > 0 && blue(img.get_pixel(x - 1, y)) {
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
        }
    }
    let side = (x1 - x0).max(y1 - y0);
    let crop = image::imageops::crop_imm(img, x0, y0, side, side).to_image();
    let size = 1024;
    let mut out = image::imageops::resize(&crop, size, size, FilterType::Lanczos3);
    let radius = size as f32 * 0.225;
    for (x, y, p) in out.enumerate_pixels_mut() {
        let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
        let half = size as f32 / 2.0;
        let dx = ((fx - half).abs() - (half - radius)).max(0.0);
        let dy = ((fy - half).abs() - (half - radius)).max(0.0);
        // One pixel of soft edge.
        let coverage = (radius - dx.hypot(dy) + 0.5).clamp(0.0, 1.0);
        p[3] = (f32::from(p[3]) * coverage) as u8;
    }
    out
}
