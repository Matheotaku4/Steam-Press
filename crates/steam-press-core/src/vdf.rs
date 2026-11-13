//! Valve KeyValues (VDF) parsing and serialization.

use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Str(String),
    Object(Vec<(String, Value)>),
}

impl Value {
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Object(entries) => entries
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(key))
                .map(|(_, value)| value),
            Value::Str(_) => None,
        }
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut Value> {
        match self {
            Value::Object(entries) => entries
                .iter_mut()
                .find(|(name, _)| name.eq_ignore_ascii_case(key))
                .map(|(_, value)| value),
            Value::Str(_) => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(value) => Some(value.as_str()),
            Value::Object(_) => None,
        }
    }

    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(Value::as_str)
    }

    pub fn as_object(&self) -> Option<&[(String, Value)]> {
        match self {
            Value::Object(entries) => Some(entries),
            Value::Str(_) => None,
        }
    }

    /// Inserts or replaces a key. Returns whether an existing key was replaced.
    pub fn set(&mut self, key: &str, value: Value) -> bool {
        match self {
            Value::Object(entries) => {
                match entries
                    .iter_mut()
                    .find(|(name, _)| name.eq_ignore_ascii_case(key))
                {
                    Some(entry) => {
                        entry.1 = value;
                        true
                    }
                    None => {
                        entries.push((key.to_string(), value));
                        false
                    }
                }
            }
            Value::Str(_) => false,
        }
    }

    pub fn to_vdf_string(&self) -> String {
        let mut output = String::new();
        match self {
            Value::Object(entries) => {
                for (key, value) in entries {
                    write_entry(&mut output, key, value, 0);
                }
            }
            Value::Str(value) => {
                output.push('"');
                output.push_str(&escape(value));
                output.push('"');
            }
        }
        output
    }
}

fn escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn write_entry(output: &mut String, key: &str, value: &Value, indent: usize) {
    for _ in 0..indent {
        output.push('\t');
    }
    output.push('"');
    output.push_str(&escape(key));
    output.push('"');
    output.push_str("\t\t");
    match value {
        Value::Str(text) => {
            output.push('"');
            output.push_str(&escape(text));
            output.push_str("\"\n");
        }
        Value::Object(entries) => {
            output.push('\n');
            for _ in 0..indent {
                output.push('\t');
            }
            output.push_str("{\n");
            for (child_key, child_value) in entries {
                write_entry(output, child_key, child_value, indent + 1);
            }
            for _ in 0..indent {
                output.push('\t');
            }
            output.push_str("}\n");
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.to_vdf_string())
    }
}

struct Parser<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            bytes: input.as_bytes(),
            position: 0,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.position).copied()
    }

    fn skip_trivia(&mut self) {
        loop {
            while let Some(byte) = self.peek() {
                if byte.is_ascii_whitespace() {
                    self.position += 1;
                } else {
                    break;
                }
            }
            if self.peek() == Some(b'/') && self.bytes.get(self.position + 1) == Some(&b'/') {
                while let Some(byte) = self.peek() {
                    self.position += 1;
                    if byte == b'\n' {
                        break;
                    }
                }
                continue;
            }
            break;
        }
    }

    fn parse_value(&mut self) -> Result<Value, String> {
        self.skip_trivia();
        if self.peek() == Some(b'{') {
            self.position += 1;
            self.parse_object()
        } else {
            Ok(Value::Str(self.parse_token()?))
        }
    }

    fn parse_token(&mut self) -> Result<String, String> {
        self.skip_trivia();
        if self.peek() == Some(b'"') {
            self.position += 1;
            let mut output = Vec::new();
            loop {
                let Some(byte) = self.peek() else {
                    return Err("unterminated VDF string".to_string());
                };
                self.position += 1;
                match byte {
                    b'"' => break,
                    b'\\' => {
                        let Some(escaped) = self.peek() else {
                            return Err("incomplete VDF escape".to_string());
                        };
                        output.push(escaped);
                        self.position += 1;
                    }
                    _ => output.push(byte),
                }
            }
            String::from_utf8(output).map_err(|_| "VDF string is not UTF-8".to_string())
        } else {
            let start = self.position;
            while let Some(byte) = self.peek() {
                if byte.is_ascii_whitespace() || byte == b'{' || byte == b'}' {
                    break;
                }
                self.position += 1;
            }
            if start == self.position {
                return Err(format!("expected a VDF token at byte {}", self.position));
            }
            Ok(String::from_utf8_lossy(&self.bytes[start..self.position]).into_owned())
        }
    }

    fn parse_object(&mut self) -> Result<Value, String> {
        let mut entries: Vec<(String, Value)> = Vec::new();
        loop {
            self.skip_trivia();
            match self.peek() {
                Some(b'}') => {
                    self.position += 1;
                    return Ok(Value::Object(entries));
                }
                None => return Err("unterminated VDF object".to_string()),
                _ => {}
            }
            let key = self.parse_token()?;
            let value = self.parse_value()?;
            match entries
                .iter_mut()
                .find(|(existing, _)| existing.eq_ignore_ascii_case(&key))
            {
                Some(entry) => entry.1 = value,
                None => entries.push((key, value)),
            }
        }
    }
}

pub fn parse(input: &str) -> Result<Value, String> {
    let mut parser = Parser::new(input);
    let mut entries: Vec<(String, Value)> = Vec::new();

    loop {
        parser.skip_trivia();
        if parser.position >= parser.bytes.len() {
            break;
        }
        if parser.peek() == Some(b'}') {
            return Err(format!(
                "unexpected closing brace at byte {}",
                parser.position
            ));
        }
        let key = parser.parse_token()?;
        let value = parser.parse_value()?;
        match entries
            .iter_mut()
            .find(|(existing, _)| existing.eq_ignore_ascii_case(&key))
        {
            Some(entry) => entry.1 = value,
            None => entries.push((key, value)),
        }
    }

    Ok(Value::Object(entries))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested_objects_and_escapes() {
        let value = parse("\"AppState\"\n{\n\t\"name\"\t\t\"Say \\\"Hi\\\"\"\n\t\"depots\"\n\t{\n\t\t\"10\"\n\t\t{\n\t\t\t\"manifest\"\t\t\"42\"\n\t\t}\n\t}\n}").unwrap();
        let state = value.get("AppState").unwrap();
        assert_eq!(state.get_str("name"), Some("Say \"Hi\""));
        assert_eq!(
            state
                .get("depots")
                .and_then(|depots| depots.get("10"))
                .and_then(|depot| depot.get_str("manifest")),
            Some("42")
        );
    }

    #[test]
    fn serializes_and_reparses() {
        let value = Value::Object(vec![(
            "AppState".into(),
            Value::Object(vec![("appid".into(), Value::Str("42".into()))]),
        )]);
        let text = value.to_vdf_string();
        assert!(text.contains("\"appid\"\t\t\"42\""));
        assert_eq!(parse(&text).unwrap(), value);
    }

    #[test]
    fn rejects_unterminated_objects() {
        assert!(parse("\"AppState\" { \"appid\" \"42\"").is_err());
    }
}
