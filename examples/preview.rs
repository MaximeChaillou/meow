//! Writes a sprite sheet PNG of every pose, one row per coat:
//! `cargo run --example preview -- out.png`

use meow::sprites::{
    Bubble, CANVAS_H, CANVAS_W, Coat, Facing, Frame, Kick, Look, PROP_H, PROP_W, Pose, Prop, Rgb,
    SPIT, render, render_prop,
};

const SCALE: usize = 8;
const BACKGROUND: [u8; 3] = [200, 210, 225];

/// Paints `frame`, scaled up, with its top left corner at (`left`, `top`).
fn blit(rgb: &mut [u8], w: usize, frame: &Frame, left: usize, top: usize) {
    for y in 0..frame.height * SCALE {
        for x in 0..frame.width * SCALE {
            let color: Rgb = frame.get(x / SCALE, y / SCALE).unwrap_or(BACKGROUND);
            let offset = ((top + y) * w + left + x) * 3;
            rgb[offset..offset + 3].copy_from_slice(&color);
        }
    }
}

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "sprites.png".into());
    let looks = [
        (Pose::Walk(0), Bubble::None, Facing::Right),
        (Pose::Walk(1), Bubble::None, Facing::Right),
        (Pose::Walk(2), Bubble::None, Facing::Right),
        (Pose::Stand, Bubble::Dots(3), Facing::Right),
        (Pose::Blink, Bubble::Bang, Facing::Right),
        (Pose::Sleep, Bubble::Zzz(0), Facing::Right),
        (Pose::Sleep, Bubble::Zzz(3), Facing::Right),
        (
            Pose::Dangle {
                swing: 0,
                kick: Kick::None,
            },
            Bubble::None,
            Facing::Right,
        ),
        (
            Pose::Dangle {
                swing: 0,
                kick: Kick::Left,
            },
            Bubble::None,
            Facing::Right,
        ),
        (
            Pose::Dangle {
                swing: -3,
                kick: Kick::None,
            },
            Bubble::None,
            Facing::Right,
        ),
        (
            Pose::Dangle {
                swing: 3,
                kick: Kick::Right,
            },
            Bubble::Dots(2),
            Facing::Right,
        ),
        (Pose::Fall, Bubble::None, Facing::Right),
        (Pose::Cling(0), Bubble::None, Facing::Right),
        (Pose::Cling(1), Bubble::None, Facing::Right),
        (Pose::Ceiling(1), Bubble::None, Facing::Right),
        (Pose::Blink, Bubble::Hearts(3), Facing::Right),
        (Pose::Blink, Bubble::Hearts(7), Facing::Right),
        (Pose::Walk(0), Bubble::None, Facing::Left),
        (Pose::Eat(0), Bubble::None, Facing::Right),
        (Pose::Eat(1), Bubble::None, Facing::Right),
        (Pose::Swat(0), Bubble::None, Facing::Right),
        (Pose::Swat(1), Bubble::None, Facing::Right),
        (Pose::Hack(0), Bubble::None, Facing::Right),
        (Pose::Hack(1), Bubble::None, Facing::Right),
        (Pose::Hack(SPIT), Bubble::None, Facing::Right),
        (Pose::Squat(0), Bubble::None, Facing::Right),
        (Pose::Squat(1), Bubble::None, Facing::Right),
    ];
    let props = [
        Prop::Bowl(3),
        Prop::Bowl(2),
        Prop::Bowl(1),
        Prop::Mouse(0),
        Prop::Mouse(1),
        Prop::Hairball(3),
        Prop::Hairball(2),
        Prop::Hairball(1),
        Prop::Sparkle(0),
        Prop::Sparkle(1),
        Prop::Poop(3),
        Prop::Poop(2),
        Prop::Poop(1),
    ];
    let row_h = CANVAS_H * SCALE;
    let cats_w = CANVAS_W * SCALE * looks.len();
    let (w, h) = (
        cats_w + PROP_W * SCALE * props.len(),
        row_h * Coat::ALL.len(),
    );
    let mut rgb = vec![0u8; w * h * 3];
    for (i, &prop) in props.iter().enumerate() {
        let frame = render_prop(prop, Facing::Right);
        let left = cats_w + i * PROP_W * SCALE;
        blit(&mut rgb, w, &frame, left, row_h - PROP_H * SCALE);
    }
    for (row, &coat) in Coat::ALL.iter().enumerate() {
        for (i, &(pose, bubble, facing)) in looks.iter().enumerate() {
            let look = Look {
                pose,
                bubble,
                facing,
            };
            let frame = render(&look, coat);
            blit(&mut rgb, w, &frame, i * CANVAS_W * SCALE, row * row_h);
        }
    }
    let file = std::fs::File::create(&path).expect("create output file");
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
    encoder.set_color(png::ColorType::Rgb);
    encoder
        .write_header()
        .and_then(|mut wr| wr.write_image_data(&rgb))
        .expect("write png");
}
