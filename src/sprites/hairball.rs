//! Coughing up a hairball: the hunched, heaving pose, and the hairball left
//! on the floor until it is wiped away.

use super::play::{HEAD_ROWS, HEAD_X};
use super::{BUBBLE_H, CANVAS_H, CANVAS_W, Grid, HEAD_TOP, LEG_ROW, Rgb};

/// `Pose::Hack` phase with the head down to the floor, spitting.
pub const SPIT: u8 = 2;
/// Head drop and forward reach for each phase: catching its breath,
/// heaving, spitting.
const HACK_HEADS: [(usize, isize); 3] = [(1, 0), (2, 1), (3, 1)];
/// Columns of the back that hunch up on a heave.
const HUNCH_XS: std::ops::RangeInclusive<usize> = 5..=11;

/// The hairball, most of it left first.
const HAIRBALLS: [&[&str]; 3] = [
    &["...j..h....", ".qqkhkkqq.."],
    &["....k.k....", "...khkhk...", "..khhjhhk..", ".qqkkkkkqq."],
    &[
        "....h.j....",
        "...khkhk...",
        "..khjjhhk..",
        ".khhhhhhhk.",
        "qqkkkkkkkqq",
    ],
];
/// Glints where the hairball was, once it is wiped clean.
const SPARKLES: [[&str; 6]; 2] = [
    [
        "...y.......",
        "..yyy...w..",
        "...y...www.",
        ".w......w..",
        "www....y...",
        ".w....yyy..",
    ],
    [
        "........y..",
        ".w.....yyy.",
        "www.....y..",
        ".w..y......",
        "...yyy...w.",
        "....y...www",
    ],
];

/// The standing cat, eyes shut, with its head pushed down and forward and,
/// on a heave, its back hunched up.
pub(super) fn draw_hacking(grid: &mut Grid, phase: u8) {
    let phase = usize::from(phase).min(HACK_HEADS.len() - 1);
    let (drop, reach) = HACK_HEADS[phase];
    let hunched = phase > 0;
    let mut standing = Grid::new();
    super::draw_standing(&mut standing, true, [0; 4]);
    let back = BUBBLE_H + HEAD_TOP.len() + 2..BUBBLE_H + LEG_ROW;
    let is_head = |x: usize, y: usize| x >= HEAD_X && (BUBBLE_H..BUBBLE_H + HEAD_ROWS).contains(&y);
    let cells: Vec<_> = (0..CANVAS_H)
        .flat_map(|y| (0..CANVAS_W).map(move |x| (x, y)))
        .map(|(x, y)| (x, y, standing.cells[y * CANVAS_W + x]))
        .filter(|&(_, _, c)| c != '.')
        .collect();
    for &(x, y, c) in cells.iter().filter(|&&(x, y, _)| !is_head(x, y)) {
        grid.set(x as isize, y, c);
    }
    if hunched {
        // The back rises a pixel; its lowest row stays, stretching the body.
        for &(x, y, c) in &cells {
            if HUNCH_XS.contains(&x) && back.contains(&y) {
                grid.set(x as isize, y - 1, c);
            }
        }
    }
    for &(x, y, c) in cells.iter().filter(|&&(x, y, _)| is_head(x, y)) {
        grid.set(x as isize + reach, y + drop, c);
    }
}

/// Rows of the hairball with `dirt` left (1 to 3), or of its glint once wiped.
pub(super) fn hairball_rows(dirt: u8) -> &'static [&'static str] {
    HAIRBALLS[usize::from(dirt.clamp(1, 3)) - 1]
}

pub(super) fn sparkle_rows(frame: u8) -> &'static [&'static str] {
    &SPARKLES[usize::from(frame) % SPARKLES.len()]
}

pub(super) fn color(c: char) -> Option<Rgb> {
    match c {
        'h' => Some([96, 76, 62]),
        'j' => Some([146, 122, 100]),
        'q' => Some([190, 186, 146]),
        'y' => Some([255, 226, 120]),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Bubble, Coat, Facing, Frame, Look, PROP_H, PROP_W, Pose, Prop};
    use super::super::{render, render_prop};
    use super::*;

    fn pose(pose: Pose) -> Frame {
        let look = Look {
            pose,
            bubble: Bubble::None,
            facing: Facing::Right,
        };
        render(&look, Coat::Ginger)
    }

    fn top_row(frame: &Frame, xs: std::ops::RangeInclusive<usize>) -> usize {
        (0..frame.height)
            .find(|&y| xs.clone().any(|x| frame.get(x, y).is_some()))
            .unwrap()
    }

    fn painted(frame: &Frame) -> usize {
        frame.pixels.iter().filter(|p| p.is_some()).count()
    }

    #[test]
    fn hairball_art_fits_and_uses_known_colors() {
        let all = HAIRBALLS
            .iter()
            .copied()
            .flatten()
            .chain(SPARKLES.iter().flatten());
        for row in all {
            assert_eq!(row.len(), PROP_W, "{row}");
            assert!(
                row.chars()
                    .all(|c| c == '.' || color(c).is_some() || Coat::Ginger.color(c).is_some()),
                "{row}"
            );
        }
        assert!(HAIRBALLS.iter().all(|rows| rows.len() <= PROP_H));
    }

    #[test]
    fn heaving_hunches_the_back_and_lowers_the_head() {
        let stand = pose(Pose::Stand);
        let breath = pose(Pose::Hack(0));
        let heave = pose(Pose::Hack(1));
        assert!(top_row(&heave, HUNCH_XS) < top_row(&stand, HUNCH_XS));
        assert_eq!(top_row(&breath, HUNCH_XS), top_row(&stand, HUNCH_XS));
        let head = HEAD_X..=CANVAS_W - 1;
        assert!(top_row(&heave, head.clone()) > top_row(&stand, head.clone()));
        assert!(top_row(&pose(Pose::Hack(SPIT)), head.clone()) > top_row(&heave, head));
    }

    #[test]
    fn hairball_shrinks_as_it_is_wiped() {
        let dirt = |level| painted(&render_prop(Prop::Hairball(level), Facing::Right));
        assert!(dirt(3) > dirt(2) && dirt(2) > dirt(1) && dirt(1) > 0);
    }

    #[test]
    fn wiped_hairball_sparkles() {
        let glint = |frame| render_prop(Prop::Sparkle(frame), Facing::Right);
        assert!(painted(&glint(0)) > 0);
        assert_ne!(glint(0), glint(1));
    }
}
