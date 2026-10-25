//! What the daemon reports about `config.toml`, and the change lists clients
//! send it, as JSON.

use std::path::PathBuf;

use serde_json::{Map, Value as Json, json};

use super::load::{Doc, Issue, Loaded, Value, load};
use super::schema;
use super::write::Change;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct State {
    pub path: PathBuf,
    /// The valid values the file sets; a key it does not set has its default.
    pub doc: Doc,
    pub issues: Vec<Issue>,
    /// Set while the file does not parse; `doc` and `issues` are then those
    /// of the last text that did.
    pub broken: Option<Broken>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Broken {
    pub line: Option<usize>,
    pub reason: String,
}

impl State {
    /// `text` is the file as it is now, `good` the last text that parsed.
    pub fn read(path: PathBuf, text: &str, good: Option<&str>) -> State {
        let broken = match load(text) {
            Loaded::Ok { doc, issues } => {
                return State {
                    path,
                    doc,
                    issues,
                    broken: None,
                };
            }
            Loaded::Broken { line, reason, .. } => Broken { line, reason },
        };
        let (doc, issues) = match good.map(load) {
            Some(Loaded::Ok { doc, issues }) => (doc, issues),
            _ => Default::default(),
        };
        State {
            path,
            doc,
            issues,
            broken: Some(broken),
        }
    }

    pub fn to_json(&self) -> Json {
        let values: Map<_, _> = self
            .doc
            .0
            .iter()
            .map(|(key, value)| (key.to_string(), value_json(value)))
            .collect();
        let issues: Vec<_> = self
            .issues
            .iter()
            .map(|issue| json!({"key": issue.key, "line": issue.line, "message": issue.message}))
            .collect();
        json!({
            "path": self.path.to_string_lossy(),
            "values": values,
            "issues": issues,
            "broken": self
                .broken
                .as_ref()
                .map(|broken| json!({"line": broken.line, "reason": broken.reason})),
        })
    }

    /// Values of keys this build does not know are left out.
    pub fn from_json(json: &Json) -> Result<State, String> {
        let mut doc = Doc::default();
        for (key, value) in field(json, "values")?
            .as_object()
            .ok_or("values must be an object")?
        {
            if let Some(key) = schema::find(key) {
                let value =
                    value_from_json(value).ok_or_else(|| format!("bad value for {}", key.path))?;
                doc.0.insert(key.path, value);
            }
        }
        let issues = field(json, "issues")?
            .as_array()
            .ok_or("issues must be an array")?
            .iter()
            .map(|issue| {
                Ok(Issue {
                    key: text(issue, "key")?,
                    line: line(issue)?,
                    message: text(issue, "message")?,
                })
            })
            .collect::<Result<_, String>>()?;
        let broken = match field(json, "broken")? {
            Json::Null => None,
            broken => Some(Broken {
                line: line(broken)?,
                reason: text(broken, "reason")?,
            }),
        };
        Ok(State {
            path: text(json, "path")?.into(),
            doc,
            issues,
            broken,
        })
    }
}

/// `[["record_fps", 120], ["record_dir", null]]`; `null` removes the key.
pub fn patch_to_json(patch: &[(String, Change)]) -> Json {
    patch
        .iter()
        .map(|(key, change)| match change {
            Change::Set(value) => json!([key, value_json(value)]),
            Change::Remove => json!([key, null]),
        })
        .collect()
}

/// Checks the form only; `apply` checks keys and values against the schema.
pub fn patch_from_json(json: &Json) -> Result<Vec<(String, Change)>, String> {
    json.as_array()
        .ok_or("patch must be an array")?
        .iter()
        .map(|entry| match entry.as_array().map(Vec::as_slice) {
            Some([Json::String(key), Json::Null]) => Ok((key.clone(), Change::Remove)),
            Some([Json::String(key), value]) => value_from_json(value)
                .map(|value| (key.clone(), Change::Set(value)))
                .ok_or_else(|| {
                    format!("{key} must be set to true, false, a whole number, a string or null")
                }),
            _ => Err("patch entries must be [key, value] pairs".into()),
        })
        .collect()
}

fn value_json(value: &Value) -> Json {
    match value {
        Value::Bool(value) => json!(value),
        Value::Int(value) => json!(value),
        Value::Text(value) => json!(value),
    }
}

fn value_from_json(json: &Json) -> Option<Value> {
    match json {
        Json::Bool(value) => Some(Value::Bool(*value)),
        Json::Number(number) => number.as_i64().map(Value::Int),
        Json::String(value) => Some(Value::Text(value.clone())),
        _ => None,
    }
}

fn field<'a>(json: &'a Json, name: &str) -> Result<&'a Json, String> {
    json.get(name).ok_or_else(|| format!("missing {name}"))
}

fn text(json: &Json, name: &str) -> Result<String, String> {
    field(json, name)?
        .as_str()
        .map(String::from)
        .ok_or_else(|| format!("{name} must be a string"))
}

fn line(json: &Json) -> Result<Option<usize>, String> {
    match field(json, "line")? {
        Json::Null => Ok(None),
        line => line
            .as_u64()
            .and_then(|line| usize::try_from(line).ok())
            .map(Some)
            .ok_or_else(|| "line must be a number or null".into()),
    }
}
