//! TMP's input stage: the processing array (`PopulateTextProcessingArray`)
//! and its rich-text tags (`ValidateHtmlTag`).
//!
//! Tag rules kept from the game's code: a tag starts at `<` and ends at
//! the first `>`, with no `<` between and fewer than 128 characters; its
//! name is matched by a hash TMP computes over the characters, and its
//! table lists each name all lower case or all upper case, so `<b>` and
//! `<B>` are tags but `<Size=…>` is text. A value follows `=`: a number
//! with a unit (`em` → font units, `%`, else pixels) or a colour. An
//! unknown name is not a tag: its characters are shown.

/// Where the text came from (`TMP_Text.m_inputSource`): a text stored in
/// the scene is `TextInputBox` (escapes like `\n` are parsed); one set
/// from code (`text = …`) is `TextString` (they are not).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Stored,
    Code,
}

/// A size value's unit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Unit {
    Pixels,
    FontUnits,
    Percentage,
}

/// A parsed tag.
#[derive(Clone, Debug, PartialEq)]
pub enum Tag {
    Bold,
    EndBold,
    /// With an angle, or the font's italic style.
    Italic(Option<i32>),
    EndItalic,
    /// Value, unit, and whether it is relative (`+`/`-`).
    Size(f32, Unit, bool),
    EndSize,
    Color([u8; 4]),
    EndColor,
    /// A tag TMP knows that this port does not apply (counted by the
    /// callers): its name.
    Unported(String),
}

/// TMP's names this port knows but does not apply.
const UNPORTED: [&str; 34] = [
    "u",
    "/u",
    "s",
    "/s",
    "sup",
    "/sup",
    "sub",
    "/sub",
    "align",
    "/align",
    "alpha",
    "cspace",
    "/cspace",
    "font",
    "/font",
    "indent",
    "/indent",
    "line-height",
    "/line-height",
    "link",
    "/link",
    "lowercase",
    "/lowercase",
    "uppercase",
    "/uppercase",
    "smallcaps",
    "/smallcaps",
    "mark",
    "/mark",
    "mspace",
    "/mspace",
    "sprite",
    "voffset",
    "/voffset",
];

fn named_color(name: &str) -> Option<[u8; 4]> {
    Some(match name {
        "red" => [255, 0, 0, 255],
        "white" => [255, 255, 255, 255],
        "black" => [0, 0, 0, 255],
        "green" => [0, 255, 0, 255],
        "blue" => [0, 0, 255, 255],
        "orange" => [255, 128, 0, 255],
        "purple" => [160, 32, 240, 255],
        "yellow" => [255, 255, 0, 255],
        _ => return None,
    })
}

fn hex(c: char) -> Option<u8> {
    c.to_digit(16).map(|d| d as u8)
}

/// `#RGB`, `#RGBA`, `#RRGGBB`, `#RRGGBBAA`.
fn hex_color(s: &str) -> Option<[u8; 4]> {
    let h: Vec<u8> = s
        .strip_prefix('#')?
        .chars()
        .map(hex)
        .collect::<Option<_>>()?;
    Some(match h.len() {
        3 => [h[0] * 17, h[1] * 17, h[2] * 17, 255],
        4 => [h[0] * 17, h[1] * 17, h[2] * 17, h[3] * 17],
        6 => [h[0] * 16 + h[1], h[2] * 16 + h[3], h[4] * 16 + h[5], 255],
        8 => [
            h[0] * 16 + h[1],
            h[2] * 16 + h[3],
            h[4] * 16 + h[5],
            h[6] * 16 + h[7],
        ],
        _ => return None,
    })
}

/// `ConvertToFloat` on a value with an optional unit.
fn number(v: &str) -> Option<(f32, Unit, bool)> {
    let (num, unit) = if let Some(n) = v.strip_suffix('%') {
        (n, Unit::Percentage)
    } else if let Some(n) = v.strip_suffix("em") {
        (n, Unit::FontUnits)
    } else if let Some(n) = v.strip_suffix("px") {
        (n, Unit::Pixels)
    } else {
        (v, Unit::Pixels)
    };
    let relative = num.starts_with('+') || num.starts_with('-');
    let x: f32 = num.parse().ok()?;
    Some((x, unit, relative))
}

/// A tag starting after the `<` at `start`: the tag and the index of its
/// `>`, or `None` if TMP would show these characters as text.
pub fn parse_tag(chars: &[u32], start: usize) -> Option<(Tag, usize)> {
    let mut end = None;
    for (k, &c) in chars.iter().enumerate().skip(start) {
        if k - start >= 128 || c == 0 || c == u32::from('<') {
            return None;
        }
        if c == u32::from('>') {
            end = Some(k);
            break;
        }
    }
    let end = end?;
    let body: String = chars[start..end]
        .iter()
        .map(|&c| char::from_u32(c).unwrap_or('\u{FFFD}'))
        .collect();
    if let Some(c) = hex_color(&body) {
        return Some((Tag::Color(c), end));
    }
    // The name runs to the first '=' or ' '; its value to the next ' ';
    // other attributes follow, `name=value`, separated by spaces.
    let name_end = body.find(['=', ' ']).unwrap_or(body.len());
    let name = &body[..name_end];
    let value = body[name_end..]
        .strip_prefix('=')
        .map(|v| v.split(' ').next().unwrap_or("").trim_matches('"'));
    let attribute = |a: &str| {
        body.split(' ')
            .skip(1)
            .find_map(|kv| kv.strip_prefix(a).and_then(|r| r.strip_prefix('=')))
    };
    // TMP's hash table: all lower case or all upper case.
    let lower = name.to_ascii_lowercase();
    if name != lower && name != name.to_ascii_uppercase() {
        return None;
    }
    let tag = match (lower.as_str(), value) {
        ("b", None) => Tag::Bold,
        ("/b", None) => Tag::EndBold,
        ("i", _) => match attribute("angle") {
            Some(v) => {
                let a = v.parse::<f32>().ok()? as i32;
                if !(-180..=180).contains(&a) {
                    return None;
                }
                Tag::Italic(Some(a))
            }
            None => Tag::Italic(None),
        },
        ("/i", _) => Tag::EndItalic,
        ("size", Some(v)) => {
            let (x, unit, rel) = number(v)?;
            Tag::Size(x, unit, rel)
        }
        ("/size", _) => Tag::EndSize,
        ("color", Some(v)) => Tag::Color(hex_color(v).or_else(|| named_color(v))?),
        ("/color", _) => Tag::EndColor,
        (n, _) if UNPORTED.contains(&n) => Tag::Unported(n.to_string()),
        _ => return None,
    };
    Some((tag, end))
}

/// The processing array: the text's characters with escapes (stored
/// texts) and `<br>`, `<nbsp>`, `<zwsp>` replaced.
pub fn processing_array(text: &str, source: Source, parse_ctrl: bool, rich_text: bool) -> Vec<u32> {
    let chars: Vec<u32> = text.chars().map(u32::from).collect();
    let mut out = Vec::with_capacity(chars.len());
    let mut i = 0;
    let at = |k: usize| chars.get(k).copied().unwrap_or(0);
    while i < chars.len() {
        let c = chars[i];
        if c == 0 {
            break;
        }
        if source == Source::Stored && c == u32::from('\\') && i + 1 < chars.len() {
            let next = at(i + 1);
            let escaped = match char::from_u32(next) {
                Some('\\') if parse_ctrl && chars.len() > i + 2 => {
                    out.push(next);
                    out.push(at(i + 2));
                    i += 3;
                    continue;
                }
                Some('n') if parse_ctrl => Some(10),
                Some('r') if parse_ctrl => Some(13),
                Some('t') if parse_ctrl => Some(9),
                Some('v') if parse_ctrl => Some(11),
                _ => None,
            };
            if let Some(e) = escaped {
                out.push(e);
                i += 2;
                continue;
            }
            let hex_at = |from: usize, n: usize| -> Option<u32> {
                let s: String = chars
                    .get(from..from + n)?
                    .iter()
                    .filter_map(|&c| char::from_u32(c))
                    .collect();
                u32::from_str_radix(&s, 16).ok()
            };
            if next == u32::from('u') && chars.len() > i + 5 {
                if let Some(u) = hex_at(i + 2, 4) {
                    out.push(u);
                    i += 6;
                    continue;
                }
            }
            if next == u32::from('U') && chars.len() > i + 9 {
                if let Some(u) = hex_at(i + 2, 8) {
                    out.push(u);
                    i += 10;
                    continue;
                }
            }
        }
        if c == u32::from('<') && rich_text {
            let rest: String = chars[i + 1..]
                .iter()
                .take(6)
                .filter_map(|&c| char::from_u32(c))
                .collect();
            let replaced = [
                ("br>", 10u32, 4usize),
                ("BR>", 10, 4),
                ("nbsp>", 160, 6),
                ("NBSP>", 160, 6),
                ("zwsp>", 8203, 6),
                ("ZWSP>", 8203, 6),
            ]
            .into_iter()
            .find(|(t, _, _)| rest.starts_with(t));
            if let Some((_, u, len)) = replaced {
                out.push(u);
                i += len;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arr(s: &str) -> Vec<u32> {
        s.chars().map(u32::from).collect()
    }

    #[test]
    fn tags_parse_as_tmp_reads_them() {
        let t = arr(
            "<b>x</B><size=120%><SIZE=+5><size=1.5em><size=20px><color=#FF000080><#0f0><color=red><i=15><i angle=15>",
        );
        let mut i = 0;
        let mut got = Vec::new();
        while i < t.len() {
            if t[i] == u32::from('<') {
                if let Some((tag, end)) = parse_tag(&t, i + 1) {
                    got.push(tag);
                    i = end + 1;
                    continue;
                }
            }
            i += 1;
        }
        assert_eq!(
            got,
            [
                Tag::Bold,
                Tag::EndBold,
                Tag::Size(120.0, Unit::Percentage, false),
                Tag::Size(5.0, Unit::Pixels, true),
                Tag::Size(1.5, Unit::FontUnits, false),
                Tag::Size(20.0, Unit::Pixels, false),
                Tag::Color([255, 0, 0, 128]),
                Tag::Color([0, 255, 0, 255]),
                Tag::Color([255, 0, 0, 255]),
                Tag::Italic(None),
                Tag::Italic(Some(15)),
            ]
        );
        // Mixed case, unknown names, a '<' inside, no '>': text.
        for s in ["<Size=10>", "<delay=2>", "<b<b>", "<b", "<color=nope>"] {
            assert!(parse_tag(&arr(s), 1).is_none(), "{s}");
        }
        assert_eq!(
            parse_tag(&arr("<u>"), 1),
            Some((Tag::Unported("u".into()), 2))
        );
    }

    #[test]
    fn escapes_only_for_stored_texts() {
        assert_eq!(
            processing_array("a\\nb", Source::Stored, true, true),
            arr("a\nb")
        );
        assert_eq!(
            processing_array("a\\nb", Source::Code, true, true),
            arr("a\\nb")
        );
        assert_eq!(
            processing_array("a\\nb", Source::Stored, false, true),
            arr("a\\nb")
        );
        assert_eq!(
            processing_array("\\u00e9", Source::Stored, true, true),
            arr("é")
        );
        assert_eq!(
            processing_array("a<br>b<nbsp>", Source::Code, true, true),
            [97, 10, 98, 160]
        );
        assert_eq!(
            processing_array("a<br>", Source::Code, true, false),
            arr("a<br>")
        );
    }
}
