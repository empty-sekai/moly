//! glb JSON 块用的最小 JSON 读取器（仅判据编译）。
//!
//! 与 `particle::json` / `weather::json` 是同一约定的第三份同构私有件：
//! 那两份的模块头具名了「各自演化比一条跨域的公共依赖便宜，重复在此
//! 具名」——本 crate 零外部依赖，corpus 判据要读 glb 的 JSON 块，照
//! 抄而不是去 pub 别人的。行为逐行同源：对象保留插入序、转义处理、
//! 尾随字节放行。只在本模块的测试里使用，`#[cfg(test)]` 门着，不进
//! lib 构建。

/// 一个已解析的 JSON 值。
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Number(f64),
    Str(String),
    Array(Vec<Value>),
    /// 插入序保留，见模块 doc。
    Object(Vec<(String, Value)>),
}

impl Value {
    /// 对象里 `key` 对应的值，不存在或不是对象则 `None`。
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Object(entries) => entries.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&[(String, Value)]> {
        match self {
            Value::Object(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Number(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }
}

/// 从 `bytes` 解析一个 JSON 值，忽略其后的所有字节。
pub fn parse(bytes: &[u8]) -> Result<Value, String> {
    let mut p = Parser { bytes, pos: 0 };
    p.skip_ws();
    p.parse_value()
}

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let b = self.peek();
        if b.is_some() {
            self.pos += 1;
        }
        b
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }

    fn expect(&mut self, want: u8) -> Result<(), String> {
        match self.bump() {
            Some(b) if b == want => Ok(()),
            Some(b) => Err(format!(
                "expected {:?} at byte {}, found {:?}",
                want as char,
                self.pos - 1,
                b as char
            )),
            None => Err(format!("expected {:?}, found end of input", want as char)),
        }
    }

    fn parse_value(&mut self) -> Result<Value, String> {
        self.skip_ws();
        match self.peek() {
            Some(b'{') => self.parse_object(),
            Some(b'[') => self.parse_array(),
            Some(b'"') => self.parse_string().map(Value::Str),
            Some(b't') => self.parse_literal("true", Value::Bool(true)),
            Some(b'f') => self.parse_literal("false", Value::Bool(false)),
            Some(b'n') => self.parse_literal("null", Value::Null),
            Some(b) if b == b'-' || b.is_ascii_digit() => self.parse_number(),
            Some(b) => Err(format!("unexpected byte {:?} at {}", b as char, self.pos)),
            None => Err("unexpected end of input".to_string()),
        }
    }

    fn parse_literal(&mut self, lit: &str, value: Value) -> Result<Value, String> {
        let bytes = lit.as_bytes();
        if self.bytes[self.pos..].starts_with(bytes) {
            self.pos += bytes.len();
            Ok(value)
        } else {
            Err(format!("expected literal {lit:?} at byte {}", self.pos))
        }
    }

    fn parse_object(&mut self) -> Result<Value, String> {
        self.expect(b'{')?;
        let mut entries = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.bump();
            return Ok(Value::Object(entries));
        }
        loop {
            self.skip_ws();
            let key = self.parse_string()?;
            self.skip_ws();
            self.expect(b':')?;
            let value = self.parse_value()?;
            entries.push((key, value));
            self.skip_ws();
            match self.bump() {
                Some(b',') => continue,
                Some(b'}') => break,
                Some(b) => {
                    return Err(format!(
                        "expected ',' or '}}' at byte {}, found {:?}",
                        self.pos - 1,
                        b as char
                    ))
                }
                None => return Err("unexpected end of input in object".to_string()),
            }
        }
        Ok(Value::Object(entries))
    }

    fn parse_array(&mut self) -> Result<Value, String> {
        self.expect(b'[')?;
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.bump();
            return Ok(Value::Array(items));
        }
        loop {
            let value = self.parse_value()?;
            items.push(value);
            self.skip_ws();
            match self.bump() {
                Some(b',') => continue,
                Some(b']') => break,
                Some(b) => {
                    return Err(format!(
                        "expected ',' or ']' at byte {}, found {:?}",
                        self.pos - 1,
                        b as char
                    ))
                }
                None => return Err("unexpected end of input in array".to_string()),
            }
        }
        Ok(Value::Array(items))
    }

    /// 读一个含转义的字符串。结构字节 `"` (0x22) 与 `\` (0x5C) 都在
    /// 0x80 以下，不可能作为 UTF-8 续字节出现在多字节字符中间——
    /// 这里的字节级扫描因此不会切进字符内部。
    fn parse_string(&mut self) -> Result<String, String> {
        self.expect(b'"')?;
        let mut out = Vec::new();
        loop {
            let start = self.pos;
            while let Some(b) = self.peek() {
                if b == b'"' || b == b'\\' {
                    break;
                }
                self.pos += 1;
            }
            out.extend_from_slice(&self.bytes[start..self.pos]);
            match self.bump() {
                Some(b'"') => {
                    return String::from_utf8(out).map_err(|e| format!("invalid utf-8 in string: {e}"))
                }
                Some(b'\\') => self.parse_escape(&mut out)?,
                Some(_) => unreachable!("loop above only stops at '\"' or '\\\\'"),
                None => return Err("unexpected end of input in string".to_string()),
            }
        }
    }

    fn parse_escape(&mut self, out: &mut Vec<u8>) -> Result<(), String> {
        match self.bump() {
            Some(b'"') => out.push(b'"'),
            Some(b'\\') => out.push(b'\\'),
            Some(b'/') => out.push(b'/'),
            Some(b'b') => out.push(0x08),
            Some(b'f') => out.push(0x0C),
            Some(b'n') => out.push(b'\n'),
            Some(b'r') => out.push(b'\r'),
            Some(b't') => out.push(b'\t'),
            Some(b'u') => {
                let hi = self.parse_hex4()?;
                let scalar = if (0xD800..=0xDBFF).contains(&hi) {
                    if self.bump() != Some(b'\\') || self.bump() != Some(b'u') {
                        return Err("expected low surrogate escape after high surrogate".to_string());
                    }
                    let lo = self.parse_hex4()?;
                    if !(0xDC00..=0xDFFF).contains(&lo) {
                        return Err(format!("invalid low surrogate {lo:#06x}"));
                    }
                    0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)
                } else {
                    hi
                };
                let ch = char::from_u32(scalar)
                    .ok_or_else(|| format!("invalid unicode escape {scalar:#x}"))?;
                let mut buf = [0u8; 4];
                out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
            }
            Some(b) => return Err(format!("invalid escape '\\{}'", b as char)),
            None => return Err("unexpected end of input after '\\'".to_string()),
        }
        Ok(())
    }

    fn parse_hex4(&mut self) -> Result<u32, String> {
        let mut v = 0u32;
        for _ in 0..4 {
            let b = self.bump().ok_or("unexpected end of input in \\u escape")?;
            let digit = match b {
                b'0'..=b'9' => u32::from(b - b'0'),
                b'a'..=b'f' => u32::from(b - b'a') + 10,
                b'A'..=b'F' => u32::from(b - b'A') + 10,
                _ => return Err(format!("invalid hex digit {:?}", b as char)),
            };
            v = v * 16 + digit;
        }
        Ok(v)
    }

    fn parse_number(&mut self) -> Result<Value, String> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.pos += 1;
        }
        if self.peek() == Some(b'.') {
            self.pos += 1;
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }
        let text = std::str::from_utf8(&self.bytes[start..self.pos])
            .map_err(|e| format!("invalid utf-8 in number: {e}"))?;
        text.parse::<f64>()
            .map(Value::Number)
            .map_err(|e| format!("invalid number {text:?}: {e}"))
    }
}
