//! Slack `mrkdwn` → CommonMark, so [`crate::markdown::to_spans`] can render it.
//!
//! Slack wraps mentions, channel references and links in `<…>` and escapes `&`,
//! `<` and `>` as HTML entities. Its emphasis markup also differs from
//! CommonMark: bold is a single `*` and strikethrough a single `~`. This module
//! turns a raw message into Markdown so mentions read as names, channels as
//! `#name`, links stay clickable in the transcript, and `:emoji:` shortcodes
//! show as the emoji themselves.

use std::collections::HashMap;

/// Resolves the ids Slack embeds in a message to human-readable names.
pub trait Names {
    /// Display name for a user id, without the leading `@`.
    fn user(&self, id: &str) -> Option<String>;
    /// Name for a channel id, without the leading `#`.
    fn channel(&self, id: &str) -> Option<String>;
}

/// A fixed set of names: a small cache, and what tests use.
#[derive(Default, Debug)]
pub struct NameMap {
    pub users: HashMap<String, String>,
    pub channels: HashMap<String, String>,
}

impl Names for NameMap {
    fn user(&self, id: &str) -> Option<String> {
        self.users.get(id).cloned()
    }
    fn channel(&self, id: &str) -> Option<String> {
        self.channels.get(id).cloned()
    }
}

/// Renders a raw Slack message to CommonMark, resolving mentions and channels
/// with `names`.
pub fn to_markdown(text: &str, names: &impl Names) -> String {
    let resolved = resolve_tokens(text, names);
    let decoded = replace_emoji(&decode_entities(&resolved));
    // One pass per delimiter; the second runs over the first's output, which is
    // fine because each pass only touches its own delimiter and leaves code
    // spans untouched.
    let bold = convert_delim(&decoded, '*', "**");
    convert_delim(&bold, '~', "~~")
}

/// Slack's own names for emoji that the standard shortcode list lacks.
const SLACK_ALIASES: [(&str, &str); 3] = [
    ("simple_smile", "slightly_smiling_face"),
    ("thumbsup_all", "+1"),
    ("slack_call", "telephone_receiver"),
];

/// Turns `:shortcode:` (and Slack's `:wave::skin-tone-3:`) into the emoji,
/// outside code spans and blocks. Unknown names, such as a workspace's custom
/// emoji, are left as typed.
fn replace_emoji(text: &str) -> String {
    text.split('`')
        .enumerate()
        .map(|(i, part)| {
            // Odd pieces sit between backticks: code, shown as typed.
            if i % 2 == 1 {
                part.to_string()
            } else {
                replace_shortcodes(part)
            }
        })
        .collect::<Vec<_>>()
        .join("`")
}

fn replace_shortcodes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(':') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let name_len = after
            .find(|c: char| !is_shortcode_char(c))
            .unwrap_or(after.len());
        let closed = after[name_len..].starts_with(':');
        let emoji = (closed && name_len > 0)
            .then(|| emoji_for(&after[..name_len]))
            .flatten();
        let Some(emoji) = emoji else {
            // Not an emoji: keep the colon, and look again from the next character.
            out.push(':');
            rest = after;
            continue;
        };
        rest = &after[name_len + 1..];
        let (emoji, skip) = with_skin_tone(emoji, rest);
        out.push_str(emoji.as_str());
        rest = &rest[skip..];
    }
    out.push_str(rest);
    out
}

pub(crate) fn is_shortcode_char(c: char) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '+' | '-' | '\'')
}

fn emoji_for(name: &str) -> Option<&'static emojis::Emoji> {
    let name = SLACK_ALIASES
        .iter()
        .find(|(alias, _)| *alias == name)
        .map_or(name, |(_, standard)| standard);
    emojis::get_by_shortcode(name)
}

/// Applies a `:skin-tone-N:` right after an emoji; returns the emoji and how
/// much of `rest` the suffix took.
fn with_skin_tone(emoji: &'static emojis::Emoji, rest: &str) -> (&'static emojis::Emoji, usize) {
    use emojis::SkinTone;
    let tones = [
        ("skin-tone-2:", SkinTone::Light),
        ("skin-tone-3:", SkinTone::MediumLight),
        ("skin-tone-4:", SkinTone::Medium),
        ("skin-tone-5:", SkinTone::MediumDark),
        ("skin-tone-6:", SkinTone::Dark),
    ];
    let Some(suffix) = rest.strip_prefix(':') else {
        return (emoji, 0);
    };
    tones
        .iter()
        .find(|(code, _)| suffix.starts_with(code))
        .map(|(code, tone)| {
            let toned = emoji.with_skin_tone(*tone).unwrap_or(emoji);
            (toned, 1 + code.len())
        })
        .unwrap_or((emoji, 0))
}

/// Replaces every `<…>` token with readable text, leaving other characters as is.
fn resolve_tokens(text: &str, names: &impl Names) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('>') {
            // Entity-escaped `>` arrives as `&gt;`, so a raw `>` always closes a token.
            Some(end) => {
                out.push_str(&resolve_token(&after[..end], names));
                rest = &after[end + 1..];
            }
            None => {
                out.push('<');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Turns one token's body (what sat between `<` and `>`) into readable text.
fn resolve_token(token: &str, names: &impl Names) -> String {
    let (left, label) = match token.split_once('|') {
        Some((l, r)) => (l, Some(r)),
        None => (token, None),
    };
    match left.chars().next() {
        Some('@') => {
            let id = &left[1..];
            let name = label
                .map(str::to_string)
                .or_else(|| names.user(id))
                .unwrap_or_else(|| id.to_string());
            format!("@{}", name.trim_start_matches('@'))
        }
        Some('#') => {
            let id = &left[1..];
            let name = label
                .map(str::to_string)
                .or_else(|| names.channel(id))
                .unwrap_or_else(|| id.to_string());
            format!("#{}", name.trim_start_matches('#'))
        }
        Some('!') => resolve_bang(&left[1..], label),
        _ => resolve_link(left, label),
    }
}

/// `<!here>`, `<!subteam^S…|@group>` and other `<!…>` commands.
fn resolve_bang(command: &str, label: Option<&str>) -> String {
    match command {
        "here" | "channel" | "everyone" => format!("@{command}"),
        // A subteam label already carries its `@`.
        _ if command.starts_with("subteam^") => label.unwrap_or("@group").to_string(),
        // `<!date^…|fallback>` and anything unknown: the human fallback if any.
        _ => label
            .map(str::to_string)
            .unwrap_or_else(|| format!("<!{command}>")),
    }
}

/// A link, as CommonMark so it stays clickable. `mailto:` is stripped for display.
fn resolve_link(url: &str, label: Option<&str>) -> String {
    let href = url.strip_prefix("mailto:").unwrap_or(url);
    let display = label
        .map(|l| l.strip_prefix("mailto:").unwrap_or(l))
        .unwrap_or(href);
    format!("[{display}]({href})")
}

/// Decodes the three entities Slack escapes. `&amp;` is done last so a literal
/// `&lt;` (sent as `&amp;lt;`) is not mistaken for an escaped `<`.
fn decode_entities(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

/// Converts a single-character Slack emphasis delimiter to its CommonMark form,
/// leaving code spans (runs of back-ticks) untouched.
fn convert_delim(text: &str, delim: char, replacement: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len() + 4);
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '`' {
            let run = run_len(&chars, i);
            match closing_run(&chars, i + run, run) {
                Some(close) => {
                    out.extend(&chars[i..close + run]);
                    i = close + run;
                }
                None => {
                    (0..run).for_each(|_| out.push('`'));
                    i += run;
                }
            }
            continue;
        }
        if c == delim
            && let Some(close) = emphasis_close(&chars, i, delim)
        {
            out.push_str(replacement);
            out.extend(&chars[i + 1..close]);
            out.push_str(replacement);
            i = close + 1;
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

/// Length of the run of back-ticks starting at `start`.
fn run_len(chars: &[char], start: usize) -> usize {
    chars[start..].iter().take_while(|&&c| c == '`').count()
}

/// Index where a back-tick run of exactly `len` begins at or after `from`.
fn closing_run(chars: &[char], from: usize, len: usize) -> Option<usize> {
    let mut i = from;
    while i < chars.len() {
        if chars[i] == '`' {
            let run = run_len(chars, i);
            if run == len {
                return Some(i);
            }
            i += run;
        } else {
            i += 1;
        }
    }
    None
}

/// Index of the delimiter that closes an emphasis span opened at `open`, if the
/// span is well formed: non-empty, on one line, and not padded with spaces.
fn emphasis_close(chars: &[char], open: usize, delim: char) -> Option<usize> {
    let first = *chars.get(open + 1)?;
    if first.is_whitespace() || first == delim {
        return None;
    }
    let mut i = open + 1;
    while i < chars.len() {
        match chars[i] {
            '\n' => return None,
            c if c == delim && !chars[i - 1].is_whitespace() => return Some(i),
            _ => i += 1,
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcodes_become_emoji() {
        let names = NameMap::default();
        assert_eq!(to_markdown("Top :+1: :tada:!", &names), "Top 👍 🎉!");
        assert_eq!(to_markdown(":heart::smile:", &names), "❤️😄");
        assert_eq!(to_markdown(":simple_smile:", &names), "🙂");
    }

    #[test]
    fn skin_tones_apply_to_the_emoji_before() {
        let names = NameMap::default();
        assert_eq!(to_markdown(":wave::skin-tone-3:", &names), "👋🏼");
        assert_eq!(
            to_markdown(":wave::skin-tone-9:", &names),
            "👋:skin-tone-9:"
        );
    }

    #[test]
    fn unknown_names_times_and_code_stay_as_typed() {
        let names = NameMap::default();
        assert_eq!(to_markdown("at 10:30:00", &names), "at 10:30:00");
        assert_eq!(to_markdown(":partyparrot: hi", &names), ":partyparrot: hi");
        assert_eq!(to_markdown("`:tada:` :tada:", &names), "`:tada:` 🎉");
        assert_eq!(to_markdown("a: b :", &names), "a: b :");
    }

    fn names() -> NameMap {
        NameMap {
            users: HashMap::from([
                ("U1".to_string(), "alice".to_string()),
                ("U2".to_string(), "bob".to_string()),
            ]),
            channels: HashMap::from([("C1".to_string(), "general".to_string())]),
        }
    }

    fn md(text: &str) -> String {
        to_markdown(text, &names())
    }

    #[test]
    fn user_mention_uses_resolved_name() {
        assert_eq!(md("hi <@U1>!"), "hi @alice!");
    }

    #[test]
    fn user_mention_prefers_embedded_label() {
        assert_eq!(md("<@U9|carol> hi"), "@carol hi");
    }

    #[test]
    fn unknown_user_mention_falls_back_to_id() {
        assert_eq!(md("<@U9>"), "@U9");
    }

    #[test]
    fn channel_reference_uses_label_then_name() {
        assert_eq!(md("see <#C9|random>"), "see #random");
        assert_eq!(md("see <#C1>"), "see #general");
    }

    #[test]
    fn here_and_group_commands() {
        assert_eq!(md("<!here> ping"), "@here ping");
        assert_eq!(md("<!channel>"), "@channel");
        assert_eq!(md("<!subteam^S1|@team>"), "@team");
    }

    #[test]
    fn link_with_label_becomes_markdown_link() {
        assert_eq!(
            md("<https://x.com/a|the site>"),
            "[the site](https://x.com/a)"
        );
    }

    #[test]
    fn bare_link_keeps_url_as_text_and_href() {
        assert_eq!(md("<https://x.com>"), "[https://x.com](https://x.com)");
    }

    #[test]
    fn mailto_is_stripped_for_display() {
        assert_eq!(md("<mailto:a@b.com|a@b.com>"), "[a@b.com](a@b.com)");
    }

    #[test]
    fn entities_are_decoded_outside_tokens() {
        assert_eq!(md("a &amp; b &lt;3 &gt;"), "a & b <3 >");
    }

    #[test]
    fn literal_angle_entity_is_not_a_token() {
        // User typed a literal "<3"; Slack sent it as "&lt;3".
        assert_eq!(md("&lt;3"), "<3");
    }

    #[test]
    fn bold_becomes_double_star() {
        assert_eq!(md("*bold* word"), "**bold** word");
    }

    #[test]
    fn italic_passes_through() {
        assert_eq!(md("_italic_ word"), "_italic_ word");
    }

    #[test]
    fn strike_becomes_double_tilde() {
        assert_eq!(md("~gone~"), "~~gone~~");
    }

    #[test]
    fn lone_star_with_spaces_is_left_alone() {
        assert_eq!(md("2 * 3 = 6"), "2 * 3 = 6");
    }

    #[test]
    fn emphasis_inside_inline_code_is_untouched() {
        assert_eq!(md("use `a*b*c` here"), "use `a*b*c` here");
    }

    #[test]
    fn fenced_code_block_is_preserved() {
        assert_eq!(md("```\n*not bold*\n```"), "```\n*not bold*\n```");
    }

    #[test]
    fn emphasis_does_not_cross_newlines() {
        assert_eq!(md("a * b\nc * d"), "a * b\nc * d");
    }

    #[test]
    fn combined_message() {
        assert_eq!(
            md("<@U1> shipped *it*: <https://pr/1|#1> &amp; done"),
            "@alice shipped **it**: [#1](https://pr/1) & done"
        );
    }

    #[test]
    fn unclosed_angle_is_kept_literally() {
        assert_eq!(md("a < b"), "a < b");
    }
}
