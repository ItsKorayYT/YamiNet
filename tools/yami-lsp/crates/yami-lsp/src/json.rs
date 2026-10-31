//! Small, strict, bounded JSON implementation for JSON-RPC framing.
//!
//! Keeping this in-tree means the language server has no registry dependencies,
//! no build-time network access and a single, auditable message parser. It is
//! deliberately not a general-purpose JSON library: depth, nesting and input
//! size are bounded and lone UTF-16 surrogates are rejected.

use std::fmt;

/// Maximum nesting depth accepted by the parser. LSP messages are shallow;
/// this only exists to make hostile input cheap to reject.
pub const MAX_DEPTH: usize = 64;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    /// The lexical form is retained so JSON-RPC ids round-trip exactly.
    Number(String),
    String(String),
    Array(Vec<Value>),
    Object(Vec<(String, Value)>),
}

impl Value {
    pub fn string(value: impl Into<String>) -> Self {
        Self::String(value.into())
    }

    pub fn number(value: i64) -> Self {
        Self::Number(value.to_string())
    }

    pub fn array(values: Vec<Value>) -> Self {
        Self::Array(values)
    }

    pub fn object(entries: Vec<(&str, Value)>) -> Self {
        Self::Object(
            entries
                .into_iter()
                .map(|(key, value)| (key.to_string(), value))
                .collect(),
        )
    }

    /// Look up an object member. Duplicate keys resolve to the last occurrence,
    /// matching common JSON behaviour.
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Self::Object(entries) => entries
                .iter()
                .rev()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(value) => Some(value),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(value) => Some(*value),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Number(raw) => raw.parse().ok(),
            _ => None,
        }
    }

    pub fn as_u32(&self) -> Option<u32> {
        self.as_i64().and_then(|value| u32::try_from(value).ok())
    }

    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Self::Array(values) => Some(values),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&[(String, Value)]> {
        match self {
            Self::Object(entries) => Some(entries),
            _ => None,
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    /// Serialize to a compact JSON string. Unicode is emitted directly; only
    /// JSON metacharacters and control characters are escaped.
    pub fn to_json_string(&self) -> String {
        let mut out = String::new();
        self.write_json(&mut out);
        out
    }

    fn write_json(&self, out: &mut String) {
        match self {
            Self::Null => out.push_str("null"),
            Self::Bool(true) => out.push_str("true"),
            Self::Bool(false) => out.push_str("false"),
            Self::Number(raw) => out.push_str(raw),
            Self::String(value) => write_string(value, out),
            Self::Array(values) => {
                out.push('[');
                for (index, value) in values.iter().enumerate() {
                    if index != 0 {
                        out.push(',');
                    }
                    value.write_json(out);
                }
                out.push(']');
            }
            Self::Object(entries) => {
                out.push('{');
                for (index, (key, value)) in entries.iter().enumerate() {
                    if index != 0 {
                        out.push(',');
                    }
                    write_string(key, out);
                    out.push(':');
                    value.write_json(out);
                }
                out.push('}');
            }
        }
    }
}

fn write_string(value: &str, out: &mut String) {
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if ch < ' ' => {
                use std::fmt::Write as _;
                // Writing to a String cannot fail.
                let _ = write!(out, "\\u{:04x}", u32::from(ch));
            }
            ch => out.push(ch),
        }
    }
    out.push('"');
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JsonError {
    /// Zero-based byte offset into the original document.
    pub offset: usize,
    pub message: String,
}

impl fmt::Display for JsonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (at byte {})", self.message, self.offset + 1)
    }
}

impl std::error::Error for JsonError {}

/// Parse one complete JSON value. Trailing non-whitespace is rejected.
pub fn parse(source: &str) -> Result<Value, JsonError> {
    let mut parser = Parser {
        source,
        bytes: source.as_bytes(),
        offset: 0,
        depth: 0,
    };
    parser.skip_whitespace();
    let value = parser.parse_value()?;
    parser.skip_whitespace();
    if parser.offset != source.len() {
        return Err(parser.error("trailing characters after JSON value"));
    }
    Ok(value)
}

struct Parser<'a> {
    source: &'a str,
    bytes: &'a [u8],
    offset: usize,
    depth: usize,
}

impl Parser<'_> {
    fn error(&self, message: impl Into<String>) -> JsonError {
        JsonError {
            offset: self.offset,
            message: message.into(),
        }
    }

    fn skip_whitespace(&mut self) {
        while let Some(byte) = self.bytes.get(self.offset) {
            if matches!(byte, b' ' | b'\t' | b'\n' | b'\r') {
                self.offset += 1;
            } else {
                break;
            }
        }
    }

    fn parse_value(&mut self) -> Result<Value, JsonError> {
        let Some(byte) = self.bytes.get(self.offset) else {
            return Err(self.error("unexpected end of JSON input"));
        };
        match byte {
            b'n' => {
                self.expect_literal("null")?;
                Ok(Value::Null)
            }
            b't' => {
                self.expect_literal("true")?;
                Ok(Value::Bool(true))
            }
            b'f' => {
                self.expect_literal("false")?;
                Ok(Value::Bool(false))
            }
            b'"' => Ok(Value::String(self.parse_string()?)),
            b'[' => self.parse_array(),
            b'{' => self.parse_object(),
            b'-' | b'0'..=b'9' => self.parse_number(),
            _ => Err(self.error("unexpected character while parsing a JSON value")),
        }
    }

    fn expect_literal(&mut self, literal: &str) -> Result<(), JsonError> {
        if self.source.get(self.offset..self.offset + literal.len()) == Some(literal) {
            self.offset += literal.len();
            Ok(())
        } else {
            Err(self.error(format!("expected `{literal}`")))
        }
    }

    fn enter(&mut self) -> Result<(), JsonError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(self.error(format!("JSON nesting exceeds {MAX_DEPTH} levels")));
        }
        Ok(())
    }

    fn parse_array(&mut self) -> Result<Value, JsonError> {
        self.enter()?;
        self.offset += 1; // '['
        let mut values = Vec::new();
        self.skip_whitespace();
        if self.bytes.get(self.offset) == Some(&b']') {
            self.offset += 1;
            self.depth -= 1;
            return Ok(Value::Array(values));
        }
        loop {
            self.skip_whitespace();
            values.push(self.parse_value()?);
            self.skip_whitespace();
            match self.bytes.get(self.offset) {
                Some(b',') => self.offset += 1,
                Some(b']') => {
                    self.offset += 1;
                    self.depth -= 1;
                    return Ok(Value::Array(values));
                }
                Some(_) => return Err(self.error("expected `,` or `]` in JSON array")),
                None => return Err(self.error("unterminated JSON array")),
            }
        }
    }

    fn parse_object(&mut self) -> Result<Value, JsonError> {
        self.enter()?;
        self.offset += 1; // '{'
        let mut entries = Vec::new();
        self.skip_whitespace();
        if self.bytes.get(self.offset) == Some(&b'}') {
            self.offset += 1;
            self.depth -= 1;
            return Ok(Value::Object(entries));
        }
        loop {
            self.skip_whitespace();
            if self.bytes.get(self.offset) != Some(&b'"') {
                return Err(self.error("expected a quoted JSON object key"));
            }
            let key = self.parse_string()?;
            self.skip_whitespace();
            if self.bytes.get(self.offset) != Some(&b':') {
                return Err(self.error("expected `:` after JSON object key"));
            }
            self.offset += 1;
            self.skip_whitespace();
            let value = self.parse_value()?;
            entries.push((key, value));
            self.skip_whitespace();
            match self.bytes.get(self.offset) {
                Some(b',') => self.offset += 1,
                Some(b'}') => {
                    self.offset += 1;
                    self.depth -= 1;
                    return Ok(Value::Object(entries));
                }
                Some(_) => return Err(self.error("expected `,` or `}` in JSON object")),
                None => return Err(self.error("unterminated JSON object")),
            }
        }
    }

    fn parse_string(&mut self) -> Result<String, JsonError> {
        self.offset += 1; // opening quote
        let mut out = String::new();
        loop {
            let Some(byte) = self.bytes.get(self.offset) else {
                return Err(self.error("unterminated JSON string"));
            };
            match byte {
                b'"' => {
                    self.offset += 1;
                    return Ok(out);
                }
                b'\\' => {
                    self.offset += 1;
                    self.parse_escape(&mut out)?;
                }
                0x00..=0x1f => {
                    return Err(self.error("unescaped control character in JSON string"));
                }
                _ => {
                    // `source` is valid UTF-8 and offset only moves across
                    // complete characters, so this is always Some.
                    let Some(ch) = self.source[self.offset..].chars().next() else {
                        return Err(self.error("invalid UTF-8 in JSON string"));
                    };
                    out.push(ch);
                    self.offset += ch.len_utf8();
                }
            }
        }
    }

    fn parse_escape(&mut self, out: &mut String) -> Result<(), JsonError> {
        let Some(byte) = self.bytes.get(self.offset) else {
            return Err(self.error("unterminated JSON escape"));
        };
        match byte {
            b'"' => {
                out.push('"');
                self.offset += 1;
            }
            b'\\' => {
                out.push('\\');
                self.offset += 1;
            }
            b'/' => {
                out.push('/');
                self.offset += 1;
            }
            b'b' => {
                out.push('\u{08}');
                self.offset += 1;
            }
            b'f' => {
                out.push('\u{0c}');
                self.offset += 1;
            }
            b'n' => {
                out.push('\n');
                self.offset += 1;
            }
            b'r' => {
                out.push('\r');
                self.offset += 1;
            }
            b't' => {
                out.push('\t');
                self.offset += 1;
            }
            b'u' => {
                self.offset += 1;
                let first = self.parse_hex_quad()?;
                let scalar = if (0xd800..=0xdbff).contains(&first) {
                    if self.bytes.get(self.offset) != Some(&b'\\')
                        || self.bytes.get(self.offset + 1) != Some(&b'u')
                    {
                        return Err(
                            self.error("high surrogate must be followed by a low surrogate")
                        );
                    }
                    self.offset += 2;
                    let second = self.parse_hex_quad()?;
                    if !(0xdc00..=0xdfff).contains(&second) {
                        return Err(self.error("invalid low surrogate in JSON escape"));
                    }
                    0x10000 + ((u32::from(first) - 0xd800) << 10) + (u32::from(second) - 0xdc00)
                } else if (0xdc00..=0xdfff).contains(&first) {
                    return Err(self.error("lone low surrogate in JSON escape"));
                } else {
                    u32::from(first)
                };
                let Some(ch) = char::from_u32(scalar) else {
                    return Err(self.error("invalid Unicode scalar in JSON escape"));
                };
                out.push(ch);
            }
            _ => return Err(self.error("invalid JSON escape sequence")),
        }
        Ok(())
    }

    fn parse_hex_quad(&mut self) -> Result<u16, JsonError> {
        let end = self.offset.saturating_add(4);
        let Some(digits) = self.source.get(self.offset..end) else {
            return Err(self.error("incomplete Unicode escape"));
        };
        if !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(self.error("invalid Unicode escape"));
        }
        let value =
            u16::from_str_radix(digits, 16).map_err(|_| self.error("invalid Unicode escape"))?;
        self.offset = end;
        Ok(value)
    }

    fn parse_number(&mut self) -> Result<Value, JsonError> {
        let start = self.offset;
        if self.bytes.get(self.offset) == Some(&b'-') {
            self.offset += 1;
        }
        match self.bytes.get(self.offset) {
            Some(b'0') => self.offset += 1,
            Some(b'1'..=b'9') => {
                self.offset += 1;
                while matches!(self.bytes.get(self.offset), Some(b'0'..=b'9')) {
                    self.offset += 1;
                }
            }
            _ => return Err(self.error("invalid JSON number")),
        }
        if self.bytes.get(self.offset) == Some(&b'.') {
            self.offset += 1;
            if !matches!(self.bytes.get(self.offset), Some(b'0'..=b'9')) {
                return Err(self.error("JSON fraction requires digits"));
            }
            while matches!(self.bytes.get(self.offset), Some(b'0'..=b'9')) {
                self.offset += 1;
            }
        }
        if matches!(self.bytes.get(self.offset), Some(b'e' | b'E')) {
            self.offset += 1;
            if matches!(self.bytes.get(self.offset), Some(b'+' | b'-')) {
                self.offset += 1;
            }
            if !matches!(self.bytes.get(self.offset), Some(b'0'..=b'9')) {
                return Err(self.error("JSON exponent requires digits"));
            }
            while matches!(self.bytes.get(self.offset), Some(b'0'..=b'9')) {
                self.offset += 1;
            }
        }
        Ok(Value::Number(self.source[start..self.offset].to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rpc_shaped_messages_and_unicode_escapes() {
        let value = parse(
            r#"{"jsonrpc":"2.0","id":"a\u00e9","method":"x","params":{"text":"\ud83d\ude00","n":-12.5e+2,"ok":true,"none":null}}"#,
        )
        .unwrap();
        assert_eq!(value.get("id").and_then(Value::as_str), Some("aé"));
        assert_eq!(
            value
                .get("params")
                .and_then(|params| params.get("text"))
                .and_then(Value::as_str),
            Some("😀")
        );
        assert_eq!(
            value
                .get("params")
                .and_then(|params| params.get("n"))
                .map(Value::to_json_string),
            Some("-12.5e+2".to_string())
        );
        let encoded = value.to_json_string();
        assert_eq!(parse(&encoded).unwrap(), value);
    }

    #[test]
    fn serializes_control_characters_and_quotes() {
        let value = Value::object(vec![(
            "message",
            Value::string("quote \" slash \\ newline\n tab\t \u{1} café 😀"),
        )]);
        let encoded = value.to_json_string();
        assert!(encoded.contains("\\\""));
        assert!(encoded.contains("\\\\"));
        assert!(encoded.contains("\\n"));
        assert!(encoded.contains("\\u0001"));
        assert!(encoded.contains("café 😀"));
        assert_eq!(parse(&encoded).unwrap(), value);
    }

    #[test]
    fn rejects_malformed_documents() {
        for source in [
            "",
            "{",
            "{\"a\":}",
            "{\"a\":1,}",
            "[1,]",
            "01",
            "1.",
            "\"unterminated",
            "\"\\q\"",
            "\"\\u12\"",
            "\"\\ud83d\"",
            "\"\\udc00\"",
            "{} trailing",
            "{\"a\" 1}",
            "nul",
        ] {
            assert!(parse(source).is_err(), "{source:?}");
        }
        assert!(parse(&format!(
            "{}{}",
            "[".repeat(MAX_DEPTH + 1),
            "]".repeat(MAX_DEPTH + 1)
        ))
        .is_err());
    }

    #[test]
    fn object_lookup_uses_last_duplicate() {
        let value = parse(r#"{"a":1,"a":2}"#).unwrap();
        assert_eq!(value.get("a").and_then(Value::as_i64), Some(2));
    }
}
