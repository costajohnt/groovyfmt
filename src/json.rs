//! A JSON reader for the messages `--lsp` receives, kept by hand like the writer in
//! `report.rs` so the binary stays free of dependencies beyond the grammar and TOML. It
//! takes RFC 8259 JSON and rejects anything else; nesting is capped so a hostile message
//! can't overflow the stack.

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    /// A number as written; checked against the JSON number grammar, so it is safe to echo.
    Num(String),
    Str(String),
    Arr(Vec<Value>),
    /// Members in order; a repeated key keeps its first value for `get`.
    Obj(Vec<(String, Value)>),
}

/// Deeper than any LSP message; deeper input is an error, not a stack overflow.
const MAX_DEPTH: usize = 64;

impl Value {
    /// A member of an object; `Null` for anything else or a missing key, so lookups chain.
    pub fn get(&self, key: &str) -> &Value {
        match self {
            Value::Obj(m) => m.iter().find(|(k, _)| k == key).map_or(&NULL, |(_, v)| v),
            _ => &NULL,
        }
    }

    pub fn str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    /// A non-negative integer that fits in u32 (LSP positions are uinteger).
    pub fn u32(&self) -> Option<u32> {
        match self {
            Value::Num(n) => n.parse().ok(),
            _ => None,
        }
    }

    pub fn arr(&self) -> &[Value] {
        match self {
            Value::Arr(a) => a,
            _ => &[],
        }
    }

    pub fn is_null(&self) -> bool {
        *self == Value::Null
    }
}

static NULL: Value = Value::Null;

pub fn parse(s: &str) -> Result<Value, String> {
    let mut p = Parser {
        b: s.as_bytes(),
        i: 0,
    };
    let v = p.value(0)?;
    p.ws();
    if p.i != p.b.len() {
        return Err(p.err("trailing characters"));
    }
    Ok(v)
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn err(&self, what: &str) -> String {
        format!("{what} at byte {}", self.i)
    }

    fn ws(&mut self) {
        while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }

    fn eat(&mut self, c: u8) -> bool {
        self.ws();
        if self.peek() == Some(c) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn lit(&mut self, word: &str, v: Value) -> Result<Value, String> {
        if self.b[self.i..].starts_with(word.as_bytes()) {
            self.i += word.len();
            Ok(v)
        } else {
            Err(self.err("unexpected character"))
        }
    }

    fn value(&mut self, depth: usize) -> Result<Value, String> {
        if depth > MAX_DEPTH {
            return Err(self.err("nesting too deep"));
        }
        self.ws();
        match self.peek() {
            Some(b'{') => {
                self.i += 1;
                let mut m = Vec::new();
                if self.eat(b'}') {
                    return Ok(Value::Obj(m));
                }
                loop {
                    self.ws();
                    if self.peek() != Some(b'"') {
                        return Err(self.err("expected a key"));
                    }
                    let k = self.string()?;
                    if !self.eat(b':') {
                        return Err(self.err("expected `:`"));
                    }
                    m.push((k, self.value(depth + 1)?));
                    if self.eat(b'}') {
                        return Ok(Value::Obj(m));
                    }
                    if !self.eat(b',') {
                        return Err(self.err("expected `,` or `}`"));
                    }
                }
            }
            Some(b'[') => {
                self.i += 1;
                let mut a = Vec::new();
                if self.eat(b']') {
                    return Ok(Value::Arr(a));
                }
                loop {
                    a.push(self.value(depth + 1)?);
                    if self.eat(b']') {
                        return Ok(Value::Arr(a));
                    }
                    if !self.eat(b',') {
                        return Err(self.err("expected `,` or `]`"));
                    }
                }
            }
            Some(b'"') => self.string().map(Value::Str),
            Some(b't') => self.lit("true", Value::Bool(true)),
            Some(b'f') => self.lit("false", Value::Bool(false)),
            Some(b'n') => self.lit("null", Value::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            _ => Err(self.err("unexpected character")),
        }
    }

    fn digits(&mut self) -> usize {
        let start = self.i;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.i += 1;
        }
        self.i - start
    }

    fn number(&mut self) -> Result<Value, String> {
        let start = self.i;
        if self.peek() == Some(b'-') {
            self.i += 1;
        }
        let int_start = self.i;
        let n = self.digits();
        if n == 0 || (n > 1 && self.b[int_start] == b'0') {
            return Err(self.err("bad number"));
        }
        if self.peek() == Some(b'.') {
            self.i += 1;
            if self.digits() == 0 {
                return Err(self.err("bad number"));
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.i += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.i += 1;
            }
            if self.digits() == 0 {
                return Err(self.err("bad number"));
            }
        }
        // only ASCII digits and signs were consumed, so this is a char boundary
        Ok(Value::Num(
            String::from_utf8_lossy(&self.b[start..self.i]).into_owned(),
        ))
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let h = self
            .b
            .get(self.i..self.i + 4)
            .and_then(|h| std::str::from_utf8(h).ok())
            .filter(|h| h.bytes().all(|c| c.is_ascii_hexdigit()))
            .and_then(|h| u32::from_str_radix(h, 16).ok())
            .ok_or_else(|| self.err("bad \\u escape"))?;
        self.i += 4;
        Ok(h)
    }

    /// A string at `"`. The input is a `&str`, so runs between escapes are valid UTF-8 and
    /// end on a char boundary (`"` and `\` never occur inside a multi-byte sequence).
    fn string(&mut self) -> Result<String, String> {
        self.i += 1;
        let mut out = String::new();
        loop {
            let start = self.i;
            while self
                .peek()
                .is_some_and(|c| c != b'"' && c != b'\\' && c >= 0x20)
            {
                self.i += 1;
            }
            out.push_str(
                std::str::from_utf8(&self.b[start..self.i]).map_err(|_| self.err("bad UTF-8"))?,
            );
            match self.peek() {
                Some(b'"') => {
                    self.i += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.i += 1;
                    let c = self.peek().ok_or_else(|| self.err("unterminated string"))?;
                    self.i += 1;
                    out.push(match c {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => {
                            let hi = self.hex4()?;
                            let cp = if (0xD800..0xDC00).contains(&hi) {
                                // a high surrogate needs its low half; a lone one is not text
                                // we could hand back unchanged, so the message is refused
                                if !self.b[self.i..].starts_with(b"\\u") {
                                    return Err(self.err("lone surrogate"));
                                }
                                self.i += 2;
                                let lo = self.hex4()?;
                                if !(0xDC00..0xE000).contains(&lo) {
                                    return Err(self.err("lone surrogate"));
                                }
                                0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)
                            } else {
                                hi
                            };
                            char::from_u32(cp).ok_or_else(|| self.err("lone surrogate"))?
                        }
                        _ => return Err(self.err("bad escape")),
                    });
                }
                Some(_) => return Err(self.err("control character in string")),
                None => return Err(self.err("unterminated string")),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values() {
        let v =
            parse(r#" {"a": [1, -2.5e3, true, false, null], "b": {"c": "d"}, "a": 0} "#).unwrap();
        assert_eq!(
            v.get("a").arr(),
            [
                Value::Num("1".into()),
                Value::Num("-2.5e3".into()),
                Value::Bool(true),
                Value::Bool(false),
                Value::Null
            ]
        );
        assert_eq!(v.get("b").get("c").str(), Some("d"));
        assert!(v.get("x").get("y").is_null());
        assert_eq!(parse("42").unwrap().u32(), Some(42));
        assert_eq!(parse("-1").unwrap().u32(), None);
        assert_eq!(parse("1.5").unwrap().u32(), None);
        assert_eq!(parse("[]").unwrap(), Value::Arr(vec![]));
        assert_eq!(parse("{}").unwrap(), Value::Obj(vec![]));
    }

    #[test]
    fn strings_and_escapes() {
        let s = |j: &str| parse(j).map(|v| v.str().unwrap().to_string());
        assert_eq!(
            s(r#""a\"\\\/\b\f\n\r\t""#).unwrap(),
            "a\"\\/\u{8}\u{c}\n\r\t"
        );
        assert_eq!(s(r#""\u00e9\u4e2d\ud83d\ude00""#).unwrap(), "é中😀");
        assert_eq!(s("\"é中😀\"").unwrap(), "é中😀");
        for bad in [
            r#""\ud83d""#,
            r#""\ud83dx""#,
            r#""\ud83d\u0041""#,
            r#""\ude00""#,
            r#""\x""#,
            r#""\u12""#,
            "\"a\nb\"",
            "\"abc",
            "\"\\",
        ] {
            assert!(parse(bad).is_err(), "{bad:?}");
        }
        // round trip through the writer
        let text = "x\u{0}\u{1b}\u{7f}##[y\"\\é😀\r\n";
        assert_eq!(s(&crate::report::string(text)).unwrap(), text);
    }

    #[test]
    fn rejects_what_is_not_json() {
        for bad in [
            "",
            " ",
            "{",
            "[1,]",
            "{\"a\":1,}",
            "{\"a\" 1}",
            "{a:1}",
            "01",
            "1.",
            "-",
            "1e",
            ".5",
            "+1",
            "tru",
            "nul",
            "[1 2]",
            "{} x",
            "'a'",
            "NaN",
        ] {
            assert!(parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn deep_nesting_is_an_error_not_a_crash() {
        assert!(parse(&"[".repeat(1_000_000)).is_err());
        assert!(parse(&format!("{}{}", "[".repeat(60), "]".repeat(60))).is_ok());
        let deep = "{\"a\":".repeat(100_000);
        assert!(parse(&deep).is_err());
    }
}
