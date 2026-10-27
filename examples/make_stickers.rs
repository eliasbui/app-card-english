//! Turns the generated sticker artwork into the one sheet the page draws its pictures from, whenever it changes:
//!
//!   cargo run --example make_stickers [folder]
//!
//! The folder (by default .pi/generated-images/stickers, where the image generator saves) holds a picture for
//! each name of STICKERS: a cartoon sticker on white. The white around it is taken away, the sticker is cut to
//! its size and laid in a square cell of ui/stickers.webp, in the order of STICKERS, and the `.st-<name>` rules
//! between the sticker marks of ui/index.html are written again to match. A name without a picture takes the one
//! FALLBACK gives it, until it has its own. Needs cwebp (brew install webp): WebP keeps the page small enough for
//! the 2 MB that WebView2 takes as a page.

use image::{ImageEncoder, Rgba, RgbaImage, codecs::png::PngEncoder, imageops::FilterType};
use std::{collections::VecDeque, path::Path, process::Command};

const STICKERS: [&str; 33] = [
    "mascot", "mascot-cheer", "mascot-sleep", "mascot-empty", "mascot-think", "mascot-zh", "nav-home", "nav-words", "nav-practice",
    "nav-stats", "nav-settings", "mode-flash", "mode-fill", "mode-choice", "mode-dict", "mode-speak", "mode-write",
    "mode-story", "ai", "clock", "streak", "fb-right", "fb-wrong", "ach-first", "ach-streak7", "ach-streak30",
    "ach-streak100", "ach-words", "ach-learned", "ach-reviews", "ach-perfect", "ach-goal", "ach-xp",
];
/// What stands in for a sticker not drawn yet.
const FALLBACK: [(&str, &str); 4] = [("mascot-", "mascot"), ("ach-", "nav-stats"), ("streak", "nav-home"), ("fb-", "mascot")];
/// The side of a cell: sharp at 80 points on a Retina screen.
const CELL: u32 = 160;
const COLUMNS: u32 = 8;
const BEGIN: &str = "/* stickers: written by examples/make_stickers.rs */";
const END: &str = "/* stickers: end */";

fn main() {
    let folder = std::env::args().nth(1).unwrap_or_else(|| ".pi/generated-images/stickers".into());
    let picture = |name: &str| ["jpg", "png", "jpeg"].iter().map(|ext| format!("{folder}/{name}.{ext}")).find(|p| Path::new(p).exists());
    let rows = (STICKERS.len() as u32).div_ceil(COLUMNS);
    let mut sheet = RgbaImage::new(COLUMNS * CELL, rows * CELL);
    let mut missing = Vec::new();
    for (i, name) in STICKERS.iter().enumerate() {
        let path = picture(name).or_else(|| {
            missing.push(*name);
            FALLBACK.iter().find(|(prefix, _)| name.starts_with(prefix)).and_then(|(_, stand_in)| picture(stand_in))
        });
        let Some(path) = path else { continue };
        // By what the file holds, not its name: the generator names some JPEG pictures .png.
        let art = image::ImageReader::open(&path)
            .and_then(|r| r.with_guessed_format())
            .map_err(|e| e.to_string())
            .and_then(|r| r.decode().map_err(|e| e.to_string()))
            .unwrap_or_else(|e| panic!("{path}: {e}"))
            .to_rgba8();
        let (x, y) = ((i as u32 % COLUMNS) * CELL, (i as u32 / COLUMNS) * CELL);
        image::imageops::overlay(&mut sheet, &sticker(&art), x.into(), y.into());
    }
    let png = std::env::temp_dir().join("card-english-stickers.png");
    PngEncoder::new(std::fs::File::create(&png).unwrap())
        .write_image(sheet.as_raw(), sheet.width(), sheet.height(), image::ExtendedColorType::Rgba8)
        .unwrap();
    let status = Command::new("cwebp")
        .args(["-quiet", "-q", "80", "-alpha_q", "100", "-m", "6", "-sharp_yuv"])
        .arg(&png)
        .args(["-o", "ui/stickers.webp"])
        .status()
        .expect("cwebp: brew install webp");
    assert!(status.success(), "cwebp failed");
    write_css(rows);
    println!("ui/stickers.webp written, {} of {} stickers drawn", STICKERS.len() - missing.len(), STICKERS.len());
    if !missing.is_empty() {
        println!("standing in for: {}", missing.join(", "));
    }
}

/// The rules that place each sticker of the sheet, between the marks in ui/index.html.
fn write_css(rows: u32) {
    let page = std::fs::read_to_string("ui/index.html").unwrap();
    let (start, end) = (page.find(BEGIN).expect("the sticker marks in ui/index.html"), page.find(END).unwrap());
    let mut css = format!("{BEGIN}\n  .st {{ background-size: {}% {}%; }}\n", COLUMNS * 100, rows * 100);
    for (i, name) in STICKERS.iter().enumerate() {
        let (c, r) = (i as u32 % COLUMNS, i as u32 / COLUMNS);
        let x = f64::from(c) * 100.0 / f64::from(COLUMNS - 1);
        let y = if rows > 1 { f64::from(r) * 100.0 / f64::from(rows - 1) } else { 0.0 };
        css.push_str(&format!("  .st-{name} {{ background-position: {x:.4}% {y:.4}%; }}\n"));
    }
    css.push_str("  ");
    std::fs::write("ui/index.html", format!("{}{css}{}", &page[..start], &page[end..])).unwrap();
}

/// The sticker without the white around it, centred in a square cell.
fn sticker(art: &RgbaImage) -> RgbaImage {
    let (w, h) = art.dimensions();
    let white = |x: u32, y: u32| {
        let p = art.get_pixel(x, y);
        p[0] >= 238 && p[1] >= 238 && p[2] >= 238
    };
    // The white that reaches the edge of the picture is the backdrop. The white border of the sticker sits inside
    // a faint grey line, so the fill stops there and the border stays.
    let mut backdrop = vec![false; (w * h) as usize];
    let mut queue = VecDeque::new();
    let edges = (0..w).flat_map(|x| [(x, 0), (x, h - 1)]).chain((0..h).flat_map(|y| [(0, y), (w - 1, y)]));
    for (x, y) in edges {
        if white(x, y) && !backdrop[(y * w + x) as usize] {
            backdrop[(y * w + x) as usize] = true;
            queue.push_back((x, y));
        }
    }
    while let Some((x, y)) = queue.pop_front() {
        let near = [(x.wrapping_sub(1), y), (x + 1, y), (x, y.wrapping_sub(1)), (x, y + 1)];
        for (nx, ny) in near {
            if nx < w && ny < h && !backdrop[(ny * w + nx) as usize] && white(nx, ny) {
                backdrop[(ny * w + nx) as usize] = true;
                queue.push_back((nx, ny));
            }
        }
    }
    let mut cut = art.clone();
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for (x, y, p) in cut.enumerate_pixels_mut() {
        if backdrop[(y * w + x) as usize] {
            // White and clear, so that the soft edge the resize makes is white like the border, not grey.
            *p = Rgba([255, 255, 255, 0]);
        } else {
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
        }
    }
    if x1 < x0 {
        return RgbaImage::new(CELL, CELL);
    }
    let (cw, ch) = (x1 - x0 + 1, y1 - y0 + 1);
    let side = cw.max(ch);
    let margin = side / 32;
    let mut square = RgbaImage::from_pixel(side + 2 * margin, side + 2 * margin, Rgba([255, 255, 255, 0]));
    let crop = image::imageops::crop_imm(&cut, x0, y0, cw, ch).to_image();
    image::imageops::overlay(&mut square, &crop, i64::from(margin + (side - cw) / 2), i64::from(margin + (side - ch) / 2));
    image::imageops::resize(&square, CELL, CELL, FilterType::Lanczos3)
}
