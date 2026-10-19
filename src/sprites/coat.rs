use serde::{Deserialize, Serialize};

use super::Rgb;

const OUTLINE: Rgb = [42, 34, 48];
const WHITE: Rgb = [255, 246, 233];
const PINK: Rgb = [244, 166, 183];
const RED: Rgb = [224, 67, 90];

/// The cat's fur and eyes. The sprites name body parts, each coat colors them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Coat {
    /// The original orange tabby.
    #[default]
    Ginger,
    /// Pale lynx point: cream fur, tabby forehead, smoky tail, blue eyes.
    Willow,
    /// Grey and white bicolour: white blaze, muzzle and paws, odd eyes.
    Alfie,
}

/// Colors of the parts that change between coats.
struct Palette {
    fur: Rgb,
    stripe: Rgb,
    blaze: Rgb,
    muzzle: Rgb,
    legs: Rgb,
    tail: Rgb,
    near_eye: Rgb,
    far_eye: Rgb,
}

const GINGER: Palette = Palette {
    fur: [242, 166, 90],
    stripe: [214, 128, 52],
    blaze: [242, 166, 90],
    muzzle: [242, 166, 90],
    legs: [242, 166, 90],
    tail: [242, 166, 90],
    near_eye: OUTLINE,
    far_eye: OUTLINE,
};

const WILLOW: Palette = Palette {
    fur: [222, 210, 198],
    stripe: [148, 134, 128],
    blaze: [128, 114, 110],
    muzzle: WHITE,
    legs: [190, 178, 170],
    tail: [104, 94, 98],
    near_eye: [92, 150, 210],
    far_eye: [92, 150, 210],
};

const ALFIE: Palette = Palette {
    fur: [112, 116, 128],
    stripe: [112, 116, 128],
    blaze: WHITE,
    muzzle: WHITE,
    legs: WHITE,
    tail: [88, 90, 102],
    near_eye: [110, 168, 222],
    far_eye: [214, 158, 56],
};

impl Coat {
    pub const ALL: [Coat; 3] = [Coat::Ginger, Coat::Willow, Coat::Alfie];

    pub fn name(self) -> &'static str {
        match self {
            Coat::Ginger => "Ginger",
            Coat::Willow => "Willow",
            Coat::Alfie => "Alfie",
        }
    }

    fn palette(self) -> &'static Palette {
        match self {
            Coat::Ginger => &GINGER,
            Coat::Willow => &WILLOW,
            Coat::Alfie => &ALFIE,
        }
    }

    /// The color of a sprite cell; `None` for anything else (transparent).
    pub(super) fn color(self, c: char) -> Option<Rgb> {
        let palette = self.palette();
        match c {
            'k' => Some(OUTLINE),
            'w' => Some(WHITE),
            'p' => Some(PINK),
            'r' => Some(RED),
            'o' => Some(palette.fur),
            's' => Some(palette.stripe),
            'm' => Some(palette.blaze),
            'c' => Some(palette.muzzle),
            'l' => Some(palette.legs),
            't' => Some(palette.tail),
            'e' => Some(palette.near_eye),
            'f' => Some(palette.far_eye),
            _ => None,
        }
    }
}
