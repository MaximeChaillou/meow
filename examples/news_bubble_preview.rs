//! Writes a PNG of the cat with its news bubble, for a few sets of sources
//! and both ways it faces: `cargo run --example news_bubble_preview -- out.png`

use meow::source::{Icons, Source, news_bubble};
use meow::sprites::{BUBBLE_TAIL_X, CANVAS_W, CAT_H, Coat, Facing, Frame, Look, Pose, Rgb, render};
use meow::sprites::{Bubble, CANVAS_H};

const SCALE: usize = 6;
const BACKGROUND: Rgb = [200, 210, 225];
/// Room around each cat for the widest bubble, in sprite pixels.
const CELL_W: usize = 80;
const CELL_H: usize = CANVAS_H + 12;

fn blit(rgb: &mut [u8], w: usize, frame: &Frame, left: isize, top: isize) {
    for y in 0..frame.height {
        for x in 0..frame.width {
            let Some(color) = frame.get(x, y) else {
                continue;
            };
            let (px, py) = (left + x as isize, top + y as isize);
            for dy in 0..SCALE {
                for dx in 0..SCALE {
                    let (sx, sy) = (px as usize * SCALE + dx, py as usize * SCALE + dy);
                    let offset = (sy * w + sx) * 3;
                    rgb[offset..offset + 3].copy_from_slice(&color);
                }
            }
        }
    }
}

fn main() {
    use Source::*;
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "news-bubble.png".into());
    let sets: [&[Source]; 4] = [
        &[Slack],
        &[GitHub, Linear],
        &[Slack, GitHub, Calendar],
        &[GitHub, Linear, Slack, Mail, Notion],
    ];
    let facings = [Facing::Right, Facing::Left];
    let (w, h) = (CELL_W * sets.len() * SCALE, CELL_H * facings.len() * SCALE);
    let mut rgb: Vec<u8> = BACKGROUND.repeat(w * h);
    for (row, &facing) in facings.iter().enumerate() {
        for (col, sources) in sets.iter().enumerate() {
            let cat = render(
                &Look {
                    pose: Pose::Stand,
                    bubble: Bubble::None,
                    facing,
                },
                Coat::Ginger,
            );
            // As the app places it: the cat in the middle of its cell, the
            // bubble's tail one pixel above its head.
            let cat_left = (col * CELL_W + (CELL_W - CANVAS_W) / 2) as isize;
            let cat_top = (row * CELL_H + CELL_H - CANVAS_H) as isize;
            blit(&mut rgb, w, &cat, cat_left, cat_top);
            let bubble = news_bubble(&Icons::for_sources(sources), facing == Facing::Right);
            let head_x = match facing {
                Facing::Right => BUBBLE_TAIL_X,
                Facing::Left => CANVAS_W - 1 - BUBBLE_TAIL_X,
            };
            let left = cat_left + head_x as isize - bubble.tip_x as isize;
            let bottom = cat_top + (CANVAS_H - CAT_H) as isize - 1;
            blit(
                &mut rgb,
                w,
                &bubble.frame,
                left,
                bottom - bubble.frame.height as isize,
            );
        }
    }
    let file = std::fs::File::create(&path).expect("create output file");
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
    encoder.set_color(png::ColorType::Rgb);
    encoder
        .write_header()
        .and_then(|mut writer| writer.write_image_data(&rgb))
        .expect("write png");
    println!("wrote {path}");
}
