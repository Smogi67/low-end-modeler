//! Just enough JSON to read GitHub's answer about the newest release. Strict: anything
//! malformed is refused rather than guessed at.

/// A JSON value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Number(f64),
    Text(String),
    List(Vec<Value>),
    /// Keys in the order they came.
    Object(Vec<(String, Value)>),
}

impl Value {
    /// A field of an object.
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Object(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn text(&self) -> Option<&str> {
        match self {
            Value::Text(text) => Some(text),
            _ => None,
        }
    }

    pub fn number(&self) -> Option<f64> {
        match self {
            Value::Number(n) => Some(*n),
            _ => None,
        }
    }

    pub fn list(&self) -> Option<&[Value]> {
        match self {
            Value::List(items) => Some(items),
            _ => None,
        }
    }
}

/// Deeper nesting than this is refused; GitHub's answers are a few levels deep.
const MOST_DEPTH: usize = 64;

pub fn parse(text: &str) -> Result<Value, String> {
    let mut reader = Reader {
        bytes: text.as_bytes(),
        at: 0,
    };
    let value = reader.value(0)?;
    reader.space();
    if reader.at != reader.bytes.len() {
        return Err(reader.error("more after the end"));
    }
    Ok(value)
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn error(&self, what: &str) -> String {
        format!(
            "GitHub's answer is not valid JSON ({what} at byte {})",
            self.at
        )
    }

    fn space(&mut self) {
        while self
            .bytes
            .get(self.at)
            .is_some_and(|b| matches!(b, b' ' | b'\t' | b'\n' | b'\r'))
        {
            self.at += 1;
        }
    }

    fn expect(&mut self, word: &str) -> Result<(), String> {
        if self.bytes[self.at..].starts_with(word.as_bytes()) {
            self.at += word.len();
            Ok(())
        } else {
            Err(self.error("an unknown word"))
        }
    }

    fn value(&mut self, depth: usize) -> Result<Value, String> {
        if depth > MOST_DEPTH {
            return Err(self.error("too deep"));
        }
        self.space();
        match self.bytes.get(self.at) {
            Some(b'{') => {
                self.at += 1;
                let mut fields = Vec::new();
                self.space();
                if self.bytes.get(self.at) == Some(&b'}') {
                    self.at += 1;
                    return Ok(Value::Object(fields));
                }
                loop {
                    self.space();
                    let key = self.string()?;
                    self.space();
                    if self.bytes.get(self.at) != Some(&b':') {
                        return Err(self.error("a missing colon"));
                    }
                    self.at += 1;
                    let value = self.value(depth + 1)?;
                    fields.push((key, value));
                    self.space();
                    match self.bytes.get(self.at) {
                        Some(b',') => self.at += 1,
                        Some(b'}') => {
                            self.at += 1;
                            return Ok(Value::Object(fields));
                        }
                        _ => return Err(self.error("a missing comma")),
                    }
                }
            }
            Some(b'[') => {
                self.at += 1;
                let mut items = Vec::new();
                self.space();
                if self.bytes.get(self.at) == Some(&b']') {
                    self.at += 1;
                    return Ok(Value::List(items));
                }
                loop {
                    items.push(self.value(depth + 1)?);
                    self.space();
                    match self.bytes.get(self.at) {
                        Some(b',') => self.at += 1,
                        Some(b']') => {
                            self.at += 1;
                            return Ok(Value::List(items));
                        }
                        _ => return Err(self.error("a missing comma")),
                    }
                }
            }
            Some(b'"') => Ok(Value::Text(self.string()?)),
            Some(b't') => self.expect("true").map(|_| Value::Bool(true)),
            Some(b'f') => self.expect("false").map(|_| Value::Bool(false)),
            Some(b'n') => self.expect("null").map(|_| Value::Null),
            Some(b'-' | b'0'..=b'9') => {
                let start = self.at;
                while self
                    .bytes
                    .get(self.at)
                    .is_some_and(|b| matches!(b, b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9'))
                {
                    self.at += 1;
                }
                let text = std::str::from_utf8(&self.bytes[start..self.at])
                    .map_err(|_| self.error("a bad number"))?;
                text.parse()
                    .map(Value::Number)
                    .map_err(|_| self.error("a bad number"))
            }
            _ => Err(self.error("an unexpected character")),
        }
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let digits = self
            .bytes
            .get(self.at..self.at + 4)
            .ok_or_else(|| self.error("a short \\u escape"))?;
        let text = std::str::from_utf8(digits).map_err(|_| self.error("a bad \\u escape"))?;
        let code = u32::from_str_radix(text, 16).map_err(|_| self.error("a bad \\u escape"))?;
        self.at += 4;
        Ok(code)
    }

    fn string(&mut self) -> Result<String, String> {
        if self.bytes.get(self.at) != Some(&b'"') {
            return Err(self.error("a missing quote"));
        }
        self.at += 1;
        let mut out: Vec<u8> = Vec::new();
        loop {
            let Some(&byte) = self.bytes.get(self.at) else {
                return Err(self.error("an unfinished string"));
            };
            self.at += 1;
            match byte {
                b'"' => break,
                b'\\' => {
                    let Some(&kind) = self.bytes.get(self.at) else {
                        return Err(self.error("an unfinished escape"));
                    };
                    self.at += 1;
                    let simple = match kind {
                        b'"' => Some('"'),
                        b'\\' => Some('\\'),
                        b'/' => Some('/'),
                        b'b' => Some('\u{8}'),
                        b'f' => Some('\u{c}'),
                        b'n' => Some('\n'),
                        b'r' => Some('\r'),
                        b't' => Some('\t'),
                        b'u' => None,
                        _ => return Err(self.error("an unknown escape")),
                    };
                    let c = match simple {
                        Some(c) => c,
                        None => {
                            let first = self.hex4()?;
                            let code = if (0xD800..0xDC00).contains(&first)
                                && self.bytes[self.at..].starts_with(b"\\u")
                            {
                                self.at += 2;
                                let second = self.hex4()?;
                                if !(0xDC00..0xE000).contains(&second) {
                                    return Err(self.error("a broken surrogate pair"));
                                }
                                0x10000 + ((first - 0xD800) << 10) + (second - 0xDC00)
                            } else {
                                first
                            };
                            char::from_u32(code).unwrap_or('\u{FFFD}')
                        }
                    };
                    let mut buffer = [0u8; 4];
                    out.extend_from_slice(c.encode_utf8(&mut buffer).as_bytes());
                }
                0..0x20 => return Err(self.error("a control character in a string")),
                _ => out.push(byte),
            }
        }
        String::from_utf8(out).map_err(|_| self.error("text that is not UTF-8"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_objects_lists_and_escapes() {
        let value = parse(
            r#" {"a": [1, -2.5e1, true, false, null], "b": {"c": "x\ny\u00e9\ud83d\ude00\"/"}, "d": []} "#,
        )
        .unwrap();
        let a = value.get("a").unwrap().list().unwrap();
        assert_eq!(a[0].number(), Some(1.0));
        assert_eq!(a[1].number(), Some(-25.0));
        assert_eq!(a[2], Value::Bool(true));
        assert_eq!(a[4], Value::Null);
        assert_eq!(
            value.get("b").unwrap().get("c").unwrap().text(),
            Some("x\nyé😀\"/")
        );
        assert_eq!(value.get("d").unwrap().list().unwrap().len(), 0);
        assert!(value.get("missing").is_none());
    }

    #[test]
    fn refuses_what_is_broken() {
        for bad in [
            "",
            "{",
            "{\"a\" 1}",
            "[1 2]",
            "\"open",
            "{\"a\":1}x",
            "tru",
            "\"\\q\"",
            "\"a\nb\"",
            &"[".repeat(100),
        ] {
            assert!(parse(bad).is_err(), "accepted {bad:?}");
        }
    }
}
