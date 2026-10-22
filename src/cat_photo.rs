//! A theme made from a photo of the user's cat: the photo is shrunk to a
//! small PNG, then a Claude run that may only read that file reports the color
//! of each body part, and a name, in a fenced `meow-coat` JSON block.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::sprites::{Coat, Palette, Rgb};
use crate::tasks::take_fence;

pub const COAT_FENCE: &str = "meow-coat";
/// Longest side of the copy Claude looks at: plenty to tell colors apart.
const PREVIEW_PIXELS: u32 = 512;
const PREVIEW_NAME: &str = "cat.png";
const MAX_NAME_CHARS: usize = 20;
const FALLBACK_NAME: &str = "My Cat";

/// A theme of the user's own, kept in the settings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomCoat {
    pub name: String,
    pub palette: Palette,
}

impl CustomCoat {
    pub fn coat(&self) -> Coat {
        Coat::Custom(self.palette)
    }
}

/// Copies `photo` into `dir` as a PNG of at most [`PREVIEW_PIXELS`] a side,
/// with `sips`: it also reads HEIC and the other formats Photos exports.
pub fn prepare(photo: &Path, dir: &Path) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("Could not prepare the photo: {e}"))?;
    let preview = dir.join(PREVIEW_NAME);
    let output = Command::new("/usr/bin/sips")
        .args(["-s", "format", "png", "-Z", &PREVIEW_PIXELS.to_string()])
        .arg(photo)
        .arg("--out")
        .arg(&preview)
        .output()
        .map_err(|e| format!("Could not run sips: {e}"))?;
    if output.status.success() && preview.is_file() {
        Ok(preview)
    } else {
        Err(format!(
            "Could not read {} as an image.",
            photo
                .file_name()
                .map(|name| name.to_string_lossy())
                .unwrap_or_default()
        ))
    }
}

/// The prompt for the run that looks at `preview`; `photo_name` (the file
/// name the user picked) may hold the cat's name.
pub fn prompt(preview: &Path, photo_name: &str) -> String {
    format!(
        "Read the image at `{}` with the Read tool. It should be a photo of my cat; I want a \
         pixel-art cat colored like it.\n\n\
         Pick one color for each part, as it looks on the cat in good light (not shadows or \
         highlights), as #rrggbb:\n\
         - fur: the main coat color, on the head and back.\n\
         - stripe: tabby stripes on the head and back; same as fur if it has none.\n\
         - blaze: the forehead between the ears (white on a cat with a white blaze).\n\
         - muzzle: around the mouth and chin.\n\
         - legs: the legs and paws (white on a cat with white socks).\n\
         - tail: the tail.\n\
         - near_eye and far_eye: the iris colors; different only if its eyes are odd.\n\n\
         Also give it a name: the photo's file is `{photo_name}`; if that looks like the \
         cat's name use it, otherwise a short word for the coat (e.g. Tuxedo, Calico). At \
         most {MAX_NAME_CHARS} characters.\n\n\
         Reply with only a fenced code block with the info string `{COAT_FENCE}` holding \
         {{\"name\": …, \"fur\": …, \"stripe\": …, \"blaze\": …, \"muzzle\": …, \"legs\": …, \
         \"tail\": …, \"near_eye\": …, \"far_eye\": …}}. If there is no cat in the photo, the \
         block holds {{\"error\": \"<why, in a few words>\"}} instead.",
        preview.display()
    )
}

#[derive(Deserialize)]
struct Report {
    error: Option<String>,
    name: Option<String>,
    fur: Option<String>,
    stripe: Option<String>,
    blaze: Option<String>,
    muzzle: Option<String>,
    legs: Option<String>,
    tail: Option<String>,
    near_eye: Option<String>,
    far_eye: Option<String>,
}

/// Reads the theme out of Claude's answer.
pub fn parse(answer: &str) -> Result<CustomCoat, String> {
    let (_, body) = take_fence(answer, COAT_FENCE);
    let body = body.ok_or("Claude sent back no colors")?;
    let report: Report = serde_json::from_str(&body)
        .map_err(|e| format!("Claude's colors are not valid JSON ({e})"))?;
    if let Some(error) = report.error.filter(|e| !e.trim().is_empty()) {
        return Err(error.trim().to_string());
    }
    let fur = color("fur", report.fur.as_deref())?;
    // Only the fur is a must: a missing marking takes the fur's color.
    let or_fur = |name: &str, value: Option<&str>| match value {
        Some(_) => color(name, value),
        None => Ok(fur),
    };
    let near_eye = color("near_eye", report.near_eye.as_deref())?;
    let palette = Palette {
        fur,
        stripe: or_fur("stripe", report.stripe.as_deref())?,
        blaze: or_fur("blaze", report.blaze.as_deref())?,
        muzzle: or_fur("muzzle", report.muzzle.as_deref())?,
        legs: or_fur("legs", report.legs.as_deref())?,
        tail: or_fur("tail", report.tail.as_deref())?,
        near_eye,
        far_eye: match report.far_eye.as_deref() {
            Some(_) => color("far_eye", report.far_eye.as_deref())?,
            None => near_eye,
        },
    };
    Ok(CustomCoat {
        name: clean_name(report.name.as_deref()),
        palette,
    })
}

fn color(part: &str, value: Option<&str>) -> Result<Rgb, String> {
    let value = value.ok_or_else(|| format!("Claude gave no {part} color"))?;
    parse_hex(value).ok_or_else(|| format!("{value:?} is not a color (for {part})"))
}

/// `#rrggbb` (the `#` may be left out).
fn parse_hex(value: &str) -> Option<Rgb> {
    let hex = value.trim().trim_start_matches('#');
    if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let channel = |at: usize| u8::from_str_radix(&hex[at..at + 2], 16).ok();
    Some([channel(0)?, channel(2)?, channel(4)?])
}

/// One line, short enough for a menu item.
fn clean_name(name: Option<&str>) -> String {
    let words: Vec<&str> = name.unwrap_or_default().split_whitespace().collect();
    let name: String = words.join(" ").chars().take(MAX_NAME_CHARS).collect();
    match name.trim() {
        "" => FALLBACK_NAME.to_string(),
        name => name.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer(json: &str) -> String {
        format!("Here you go.\n```{COAT_FENCE}\n{json}\n```")
    }

    #[test]
    fn reads_every_part_of_the_coat() {
        let coat = parse(&answer(
            r##"{"name": "Mochi", "fur": "#202020", "stripe": "#101010", "blaze": "#FFFFFF",
                "muzzle": "ffffff", "legs": "#fefefe", "tail": "#202020",
                "near_eye": "#3a8f3a", "far_eye": "#d0a020"}"##,
        ))
        .unwrap();
        assert_eq!(coat.name, "Mochi");
        assert_eq!(coat.palette.fur, [0x20, 0x20, 0x20]);
        assert_eq!(coat.palette.blaze, [0xff, 0xff, 0xff]);
        assert_eq!(coat.palette.muzzle, [0xff, 0xff, 0xff]);
        assert_eq!(coat.palette.far_eye, [0xd0, 0xa0, 0x20]);
        assert_eq!(coat.coat(), Coat::Custom(coat.palette));
    }

    #[test]
    fn missing_markings_take_the_fur_and_one_eye_color_does_both() {
        let coat = parse(&answer(r##"{"fur": "#f2a65a", "near_eye": "#2a2230"}"##)).unwrap();
        let fur = [0xf2, 0xa6, 0x5a];
        assert_eq!(
            [coat.palette.stripe, coat.palette.legs, coat.palette.tail],
            [fur; 3]
        );
        assert_eq!(coat.palette.far_eye, coat.palette.near_eye);
        assert_eq!(coat.name, FALLBACK_NAME);
    }

    #[test]
    fn no_cat_or_bad_colors_are_errors() {
        assert_eq!(
            parse(&answer(r#"{"error": "that is a dog"}"#)),
            Err("that is a dog".to_string())
        );
        assert!(parse(&answer(r##"{"fur": "orange", "near_eye": "#000000"}"##)).is_err());
        assert!(
            parse(&answer(r##"{"fur": "#000000"}"##)).is_err(),
            "no eyes"
        );
        assert!(parse("I can't see the file.").is_err());
    }

    #[test]
    fn names_fit_on_one_short_line() {
        assert_eq!(clean_name(Some("  Sir\nPounce ")), "Sir Pounce");
        assert_eq!(
            clean_name(Some("Princess Fluffington the Third"))
                .chars()
                .count(),
            MAX_NAME_CHARS
        );
        assert_eq!(clean_name(Some("   ")), FALLBACK_NAME);
    }

    #[test]
    fn prompt_points_at_the_preview_and_the_fence() {
        let text = prompt(Path::new("/tmp/x/cat.png"), "Mochi.HEIC");
        assert!(text.contains("`/tmp/x/cat.png`") && text.contains("`Mochi.HEIC`"));
        assert!(text.contains(COAT_FENCE));
    }

    #[test]
    fn prepare_shrinks_the_photo_to_a_png() {
        let dir = std::env::temp_dir().join(format!("meow-cat-photo-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let photo = dir.join("photo.png");
        let file = std::fs::File::create(&photo).unwrap();
        let mut encoder = png::Encoder::new(file, 1024, 768);
        encoder.set_color(png::ColorType::Rgb);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&vec![200; 1024 * 768 * 3]).unwrap();
        writer.finish().unwrap();

        let preview = prepare(&photo, &dir.join("out")).unwrap();
        let decoder = png::Decoder::new(std::io::BufReader::new(
            std::fs::File::open(&preview).unwrap(),
        ));
        let info = decoder.read_info().unwrap().info().clone();
        assert_eq!((info.width, info.height), (512, 384));

        std::fs::write(dir.join("notes.txt"), "not an image").unwrap();
        assert!(prepare(&dir.join("notes.txt"), &dir.join("out2")).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
