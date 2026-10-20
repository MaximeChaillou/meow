//! A workspace's custom emoji (`:partyparrot:`): Slack's `emoji.list` maps
//! each name to an image URL, or to another name (`alias:…`). Standard emoji
//! are turned into characters by [`super::format`]; what is left as
//! `:name:` in a message may be one of these, drawn as an image.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

/// Aliases are followed at most this deep, so a loop cannot hang.
const MAX_ALIAS_DEPTH: usize = 4;
const IMAGE_EXTENSIONS: [&str; 4] = ["png", "gif", "jpg", "jpeg"];

/// What a custom emoji name stands for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Custom {
    /// An image to download and draw.
    Image(String),
    /// An alias of a standard emoji, as its character.
    Standard(String),
}

/// Resolves `emoji.list`'s names and aliases; names that lead nowhere are dropped.
pub fn resolve(raw: &HashMap<String, String>) -> HashMap<String, Custom> {
    raw.keys()
        .filter_map(|name| Some((name.clone(), resolve_one(raw, name)?)))
        .collect()
}

fn resolve_one(raw: &HashMap<String, String>, name: &str) -> Option<Custom> {
    let mut value = raw.get(name)?;
    for _ in 0..MAX_ALIAS_DEPTH {
        let Some(target) = value.strip_prefix("alias:") else {
            return value
                .starts_with("https://")
                .then(|| Custom::Image(value.clone()));
        };
        match raw.get(target) {
            Some(next) => value = next,
            None => {
                return emojis::get_by_shortcode(target)
                    .map(|emoji| Custom::Standard(emoji.as_str().to_string()));
            }
        }
    }
    None
}

/// Every `:name:` in `text`, as (byte start, byte end, name).
pub fn shortcodes(text: &str) -> Vec<(usize, usize, &str)> {
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(offset) = text[from..].find(':') {
        let start = from + offset;
        let name_start = start + 1;
        let name_len = text[name_start..]
            .find(|c: char| !super::format::is_shortcode_char(c))
            .unwrap_or(text.len() - name_start);
        let end = name_start + name_len;
        if name_len > 0 && text[end..].starts_with(':') {
            found.push((start, end + 1, &text[name_start..end]));
            from = end + 1;
        } else {
            from = name_start;
        }
    }
    found
}

/// Where an emoji's image is cached: its name, a hash of its URL (a changed
/// image gets a new file) and the image's extension.
pub fn cache_file(dir: &Path, name: &str, url: &str) -> PathBuf {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    url.hash(&mut hasher);
    let extension = url
        .rsplit('/')
        .next()
        .and_then(|file| file.split('?').next())
        .and_then(|file| file.rsplit_once('.'))
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .filter(|ext| IMAGE_EXTENSIONS.contains(&ext.as_str()))
        .unwrap_or_else(|| "png".to_string());
    let safe: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    dir.join(format!("{safe}-{:016x}.{extension}", hasher.finish()))
}

/// `~/Library/Caches/Meow/emoji`.
pub fn cache_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Library/Caches/Meow/emoji"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn names_and_aliases_resolve() {
        let list = raw(&[
            ("parrot", "https://emoji.slack-edge.com/T1/parrot/abc.gif"),
            ("party", "alias:parrot"),
            ("yes", "alias:+1"),
            ("loop", "alias:loop"),
            ("broken", "alias:nothing_like_this"),
        ]);
        let resolved = resolve(&list);
        let parrot = Custom::Image("https://emoji.slack-edge.com/T1/parrot/abc.gif".into());
        assert_eq!(resolved.get("parrot"), Some(&parrot));
        assert_eq!(resolved.get("party"), Some(&parrot));
        assert_eq!(resolved.get("yes"), Some(&Custom::Standard("👍".into())));
        assert_eq!(resolved.get("loop"), None);
        assert_eq!(resolved.get("broken"), None);
    }

    #[test]
    fn finds_shortcodes_but_not_times() {
        let text = "hi :parrot::party: at 10:30 :x";
        let names: Vec<_> = shortcodes(text).into_iter().map(|(_, _, n)| n).collect();
        assert_eq!(names, ["parrot", "party"]);
        let (start, end, _) = shortcodes(text)[0];
        assert_eq!(&text[start..end], ":parrot:");
    }

    #[test]
    fn cache_files_are_safe_and_keep_the_extension() {
        let dir = Path::new("/tmp/emoji");
        let gif = cache_file(dir, "party/parrot", "https://e.com/T1/p/abc.GIF?x=1");
        let name = gif.file_name().unwrap().to_str().unwrap();
        assert!(
            name.starts_with("party_parrot-") && name.ends_with(".gif"),
            "{name}"
        );
        let other = cache_file(dir, "party/parrot", "https://e.com/T1/p/def.gif");
        assert_ne!(gif, other, "a new image gets a new file");
        assert!(
            cache_file(dir, "a", "https://e.com/a")
                .to_str()
                .unwrap()
                .ends_with(".png")
        );
    }
}
