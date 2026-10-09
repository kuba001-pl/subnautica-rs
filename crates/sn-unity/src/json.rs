//! A small JSON reader for the game's JSON files (the Addressables catalog,
//! the loot distribution). Accepts `//` and `/* */` comments, which the
//! game's hand-edited files contain.

use crate::{Error, ErrorKind, Result};

fn invalid(offset: usize, what: impl Into<String>) -> Error {
    Error {
        offset,
        kind: ErrorKind::Invalid(what.into()),
    }
}

/// A parsed JSON value (only what the catalog needs).
#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

impl Json {
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::String(s) => Some(s),
            _ => None,
        }
    }
}

const MAX_DEPTH: usize = 64;

struct JsonParser<'a> {
    data: &'a [u8],
    pos: usize,
}

impl JsonParser<'_> {
    /// Skips white space and comments. An unterminated `/*` comment runs
    /// to the end (the next read reports the error).
    fn skip_ws(&mut self) {
        loop {
            let rest = self.data.get(self.pos..).unwrap_or_default();
            if rest.first().is_some_and(|b| b.is_ascii_whitespace()) {
                self.pos += 1;
            } else if rest.starts_with(b"//") {
                let line = rest.iter().position(|&b| b == b'\n').unwrap_or(rest.len());
                self.pos += line;
            } else if rest.starts_with(b"/*") {
                let end = rest
                    .windows(2)
                    .skip(2)
                    .position(|w| w == b"*/")
                    .map_or(rest.len(), |i| i + 4);
                self.pos += end;
            } else {
                return;
            }
        }
    }

    fn expect(&mut self, byte: u8) -> Result<()> {
        self.skip_ws();
        if self.data.get(self.pos) == Some(&byte) {
            self.pos += 1;
            Ok(())
        } else {
            Err(invalid(
                self.pos,
                format!("JSON: expected {:?}", byte as char),
            ))
        }
    }

    fn value(&mut self, depth: usize) -> Result<Json> {
        if depth > MAX_DEPTH {
            return Err(invalid(self.pos, "JSON nested too deeply"));
        }
        self.skip_ws();
        match self.data.get(self.pos) {
            Some(b'{') => {
                self.pos += 1;
                let mut fields = Vec::new();
                self.skip_ws();
                if self.data.get(self.pos) == Some(&b'}') {
                    self.pos += 1;
                    return Ok(Json::Object(fields));
                }
                loop {
                    self.skip_ws();
                    let key = self.string()?;
                    self.expect(b':')?;
                    fields.push((key, self.value(depth + 1)?));
                    self.skip_ws();
                    match self.data.get(self.pos) {
                        Some(b',') => self.pos += 1,
                        Some(b'}') => {
                            self.pos += 1;
                            return Ok(Json::Object(fields));
                        }
                        _ => return Err(invalid(self.pos, "JSON: expected ',' or '}'")),
                    }
                }
            }
            Some(b'[') => {
                self.pos += 1;
                let mut items = Vec::new();
                self.skip_ws();
                if self.data.get(self.pos) == Some(&b']') {
                    self.pos += 1;
                    return Ok(Json::Array(items));
                }
                loop {
                    items.push(self.value(depth + 1)?);
                    self.skip_ws();
                    match self.data.get(self.pos) {
                        Some(b',') => self.pos += 1,
                        Some(b']') => {
                            self.pos += 1;
                            return Ok(Json::Array(items));
                        }
                        _ => return Err(invalid(self.pos, "JSON: expected ',' or ']'")),
                    }
                }
            }
            Some(b'"') => Ok(Json::String(self.string()?)),
            Some(b't') => self.literal("true", Json::Bool(true)),
            Some(b'f') => self.literal("false", Json::Bool(false)),
            Some(b'n') => self.literal("null", Json::Null),
            Some(b'-' | b'0'..=b'9') => {
                let start = self.pos;
                while self
                    .data
                    .get(self.pos)
                    .is_some_and(|b| b"+-.eE0123456789".contains(b))
                {
                    self.pos += 1;
                }
                std::str::from_utf8(&self.data[start..self.pos])
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .map(Json::Number)
                    .ok_or_else(|| invalid(start, "JSON: bad number"))
            }
            _ => Err(invalid(self.pos, "JSON: unexpected character")),
        }
    }

    fn literal(&mut self, word: &str, value: Json) -> Result<Json> {
        if self.data[self.pos..].starts_with(word.as_bytes()) {
            self.pos += word.len();
            Ok(value)
        } else {
            Err(invalid(self.pos, "JSON: bad literal"))
        }
    }

    fn hex4(&mut self) -> Result<u32> {
        let digits = self
            .data
            .get(self.pos..self.pos + 4)
            .and_then(|d| std::str::from_utf8(d).ok())
            .and_then(|d| u32::from_str_radix(d, 16).ok())
            .ok_or_else(|| invalid(self.pos, "JSON: bad \\u escape"))?;
        self.pos += 4;
        Ok(digits)
    }

    fn string(&mut self) -> Result<String> {
        if self.data.get(self.pos) != Some(&b'"') {
            return Err(invalid(self.pos, "JSON: expected a string"));
        }
        self.pos += 1;
        let mut out = Vec::new();
        loop {
            let Some(&b) = self.data.get(self.pos) else {
                return Err(invalid(self.pos, "JSON: unterminated string"));
            };
            self.pos += 1;
            match b {
                b'"' => break,
                b'\\' => {
                    let Some(&e) = self.data.get(self.pos) else {
                        return Err(invalid(self.pos, "JSON: unterminated escape"));
                    };
                    self.pos += 1;
                    let c = match e {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => {
                            let mut code = self.hex4()?;
                            if (0xd800..0xdc00).contains(&code)
                                && self.data.get(self.pos..self.pos + 2) == Some(b"\\u")
                            {
                                self.pos += 2;
                                let low = self.hex4()?;
                                code = 0x10000
                                    + ((code - 0xd800) << 10)
                                    + (low.wrapping_sub(0xdc00) & 0x3ff);
                            }
                            char::from_u32(code).unwrap_or('\u{fffd}')
                        }
                        _ => return Err(invalid(self.pos, "JSON: bad escape")),
                    };
                    let mut buf = [0u8; 4];
                    out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                }
                _ => out.push(b),
            }
        }
        String::from_utf8(out).map_err(|_| invalid(self.pos, "JSON: string is not UTF-8"))
    }
}

pub fn parse_json(data: &[u8]) -> Result<Json> {
    let mut p = JsonParser { data, pos: 0 };
    let value = p.value(0)?;
    p.skip_ws();
    if p.pos != data.len() {
        return Err(invalid(p.pos, "JSON: trailing data"));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_and_errors() {
        let v = parse_json(br#" {"a": [1, "x\"yA"], "b": {}} "#).unwrap();
        assert_eq!(
            v.get("a"),
            Some(&Json::Array(vec![
                Json::Number(1.0),
                Json::String("x\"yA".into())
            ]))
        );
        assert!(parse_json(b"[1,").is_err());
        assert!(parse_json(b"[1 /* open").is_err());
        assert!(parse_json(&[b'['; 100]).is_err());
    }

    #[test]
    fn comments_are_white_space() {
        let text =
            b"// header\n{\n  // ShipSpecial\n  \"biome\" : 900, /* x */ \"n\": -1.5e1 // end\n}\n";
        let v = parse_json(text).unwrap();
        assert_eq!(v.get("biome"), Some(&Json::Number(900.0)));
        assert_eq!(v.get("n"), Some(&Json::Number(-15.0)));
        // "/*/" is not a closed comment.
        assert!(parse_json(b"/*/ 1").is_err());
    }

    #[test]
    fn corrupt_json_never_panics() {
        let text = b"// c\n{\"a\": [1, 2.5, \"s\\u00e9\", true, null, /* c */ {\"b\": false}]}";
        for i in 0..text.len() {
            let _ = parse_json(&text[..i]);
            for flip in [b'/', b'*', b'"', b'\\', 0xff] {
                let mut bad = text.to_vec();
                bad[i] = flip;
                let _ = parse_json(&bad);
            }
        }
        assert!(parse_json(text).is_ok());
    }
}
