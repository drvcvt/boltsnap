//! Reads `config.toml` against the schema and says what is wrong with it.

use std::collections::BTreeMap;

use super::schema::{self, KEYS, Key, Kind, TABLES};

/// A checked value as the file has it; defaults are the caller's business.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Bool(bool),
    Int(i64),
    Text(String),
}

/// The valid values in a file, by key path. A key that is missing or invalid
/// has no entry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Doc(BTreeMap<&'static str, Value>);

impl Doc {
    pub fn get(&self, path: &str) -> Option<&Value> {
        self.0.get(path)
    }

    pub fn bool(&self, path: &str) -> Option<bool> {
        match self.get(path)? {
            Value::Bool(value) => Some(*value),
            _ => None,
        }
    }

    pub fn int(&self, path: &str) -> Option<i64> {
        match self.get(path)? {
            Value::Int(value) => Some(*value),
            _ => None,
        }
    }

    pub fn text(&self, path: &str) -> Option<&str> {
        match self.get(path)? {
            Value::Text(value) => Some(value),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    /// A key path such as `replay.fps`, or a table name.
    pub key: String,
    /// 1-based, when the key could be found in the text.
    pub line: Option<usize>,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Loaded {
    Ok {
        doc: Doc,
        issues: Vec<Issue>,
    },
    /// Not valid TOML; nothing in it can be trusted.
    Broken {
        line: Option<usize>,
        /// The parser's full report, with the offending line quoted.
        message: String,
        /// Its first line alone, short enough for a notification.
        reason: String,
    },
}

pub fn load(text: &str) -> Loaded {
    let root = match toml::from_str::<toml::Table>(text) {
        Ok(root) => root,
        Err(error) => {
            return Loaded::Broken {
                line: error.span().map(|span| line_at(text, span.start)),
                message: error.to_string(),
                reason: error.message().lines().next().unwrap_or_default().into(),
            };
        }
    };
    let mut doc = Doc::default();
    let mut issues = Vec::new();
    for table in TABLES {
        if root.get(*table).is_some_and(|value| !value.is_table()) {
            issues.push(Issue {
                key: (*table).into(),
                line: key_line(text, None, table),
                message: format!("{table} must be a settings table"),
            });
        }
    }
    for key in KEYS {
        let (table, name) = schema::split(key.path);
        let value = match table {
            None => root.get(name),
            Some(table) => root
                .get(table)
                .and_then(toml::Value::as_table)
                .and_then(|table| table.get(name)),
        };
        let Some(value) = value else {
            continue;
        };
        match from_toml(value)
            .ok_or_else(|| invalid(key))
            .and_then(|value| check(key, &value).map(|()| value))
        {
            Ok(value) => {
                doc.0.insert(key.path, value);
            }
            Err(message) => issues.push(Issue {
                key: key.path.into(),
                line: key_line(text, table, name),
                message,
            }),
        }
    }
    for (name, value) in &root {
        if TABLES.contains(&name.as_str()) {
            for inner in value.as_table().into_iter().flat_map(|table| table.keys()) {
                let path = format!("{name}.{inner}");
                if schema::find(&path).is_none() {
                    issues.push(unknown(text, Some(name), inner, path));
                }
            }
        } else if schema::find(name).is_none() {
            issues.push(unknown(text, None, name, name.clone()));
        }
    }
    Loaded::Ok { doc, issues }
}

/// Whether `value` is allowed for `key`, with the message to show if not.
pub fn check(key: &Key, value: &Value) -> Result<(), String> {
    let valid = match (key.kind, value) {
        (Kind::Bool, Value::Bool(_)) => true,
        // A negative count is not a count at all, as the replay parser said.
        (Kind::Int { min, .. }, Value::Int(n)) if *n < 0 && min >= 0 => false,
        (Kind::Int { min, max }, Value::Int(n)) => {
            if *n < min || *n > max {
                return Err(key
                    .message
                    .map(String::from)
                    .unwrap_or_else(|| format!("{} must be {min}..{max}", label(key))));
            }
            true
        }
        (Kind::Choice(choices), Value::Text(text)) => choices.contains(&text.as_str()),
        (Kind::Text { empty_ok }, Value::Text(text)) => empty_ok || !text.is_empty(),
        (Kind::Path, Value::Text(_)) => true,
        (Kind::Target, Value::Text(text)) => {
            matches!(text.as_str(), "focused" | "both")
                || text
                    .strip_prefix("output:")
                    .is_some_and(|name| !name.is_empty())
        }
        _ => false,
    };
    if valid { Ok(()) } else { Err(invalid(key)) }
}

fn from_toml(value: &toml::Value) -> Option<Value> {
    match value {
        toml::Value::Boolean(value) => Some(Value::Bool(*value)),
        toml::Value::Integer(value) => Some(Value::Int(*value)),
        toml::Value::String(value) => Some(Value::Text(value.clone())),
        _ => None,
    }
}

/// `replay fps` for `replay.fps`, as the replay parser always said it.
fn label(key: &Key) -> String {
    key.path.replacen('.', " ", 1)
}

fn invalid(key: &Key) -> String {
    if let Some(message) = key.message {
        return message.into();
    }
    let label = label(key);
    match key.kind {
        Kind::Choice(choices) => format!("invalid {label}: expected {}", choices.join(", ")),
        Kind::Target => format!("invalid {label}: expected focused, both or output:<name>"),
        _ => format!("invalid {label}"),
    }
}

fn unknown(text: &str, table: Option<&str>, name: &str, path: String) -> Issue {
    Issue {
        line: key_line(text, table, name),
        message: format!("unknown setting {path}"),
        key: path,
    }
}

fn line_at(text: &str, byte: usize) -> usize {
    let byte = byte.min(text.len());
    text.as_bytes()[..byte]
        .iter()
        .filter(|&&b| b == b'\n')
        .count()
        + 1
}

/// The line of `name = …` in `table` (`None` for the top level). A plain line
/// scan: the config has no dotted keys or inline tables to follow.
fn key_line(text: &str, table: Option<&str>, name: &str) -> Option<usize> {
    let mut current: Option<&str> = None;
    for (index, line) in text.lines().enumerate() {
        let line = line.trim_start();
        if let Some(header) = line.strip_prefix('[') {
            current = header
                .split(']')
                .next()
                .map(str::trim)
                .filter(|header| !header.is_empty());
            continue;
        }
        if current == table
            && line
                .strip_prefix(name)
                .is_some_and(|rest| rest.trim_start().starts_with('='))
        {
            return Some(index + 1);
        }
    }
    None
}
