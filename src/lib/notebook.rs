//! Notebook data model (equivalent to nbformat)

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use std::collections::BTreeMap;

/// Custom deserializer for the `source` field which can be a string or array of strings
fn deserialize_source<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    match value {
        Value::String(s) => Ok(s),
        Value::Array(arr) => {
            let parts: Vec<String> = arr
                .into_iter()
                .map(|v| match v {
                    Value::String(s) => s,
                    _ => v.to_string(),
                })
                .collect();
            Ok(parts.join(""))
        }
        Value::Null => Ok(String::new()),
        _ => Ok(value.to_string()),
    }
}

/// Custom serializer for `source` - always writes as a single string
fn serialize_source<S>(source: &str, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(source)
}

/// A Jupyter notebook cell
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cell {
    pub cell_type: CellType,
    #[serde(deserialize_with = "deserialize_source", serialize_with = "serialize_source")]
    pub source: String,
    #[serde(default)]
    pub metadata: BTreeMap<String, Value>,
    /// Only present for code cells
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_count: Option<Value>,
    /// Only present for code cells
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outputs: Option<Vec<Value>>,
    /// Cell ID (nbformat 4.5+)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CellType {
    Code,
    Markdown,
    Raw,
}

impl std::fmt::Display for CellType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CellType::Code => write!(f, "code"),
            CellType::Markdown => write!(f, "markdown"),
            CellType::Raw => write!(f, "raw"),
        }
    }
}

impl CellType {
    pub fn from_str(s: &str) -> Option<CellType> {
        match s {
            "code" => Some(CellType::Code),
            "markdown" | "md" => Some(CellType::Markdown),
            "raw" => Some(CellType::Raw),
            _ => None,
        }
    }
}

/// A Jupyter notebook
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notebook {
    pub nbformat: u32,
    pub nbformat_minor: u32,
    pub metadata: BTreeMap<String, Value>,
    pub cells: Vec<Cell>,
}

impl Notebook {
    pub fn new() -> Self {
        Notebook {
            nbformat: 4,
            nbformat_minor: 5,
            metadata: BTreeMap::new(),
            cells: Vec::new(),
        }
    }

    pub fn new_with_metadata(metadata: BTreeMap<String, Value>) -> Self {
        Notebook {
            nbformat: 4,
            nbformat_minor: 5,
            metadata,
            cells: Vec::new(),
        }
    }
}

impl Default for Notebook {
    fn default() -> Self {
        Self::new()
    }
}

impl Cell {
    pub fn new_code(source: &str) -> Self {
        Cell {
            cell_type: CellType::Code,
            source: source.to_string(),
            metadata: BTreeMap::new(),
            execution_count: Some(Value::Null),
            outputs: Some(Vec::new()),
            id: None,
        }
    }

    pub fn new_markdown(source: &str) -> Self {
        Cell {
            cell_type: CellType::Markdown,
            source: source.to_string(),
            metadata: BTreeMap::new(),
            execution_count: None,
            outputs: None,
            id: None,
        }
    }

    pub fn new_raw(source: &str) -> Self {
        Cell {
            cell_type: CellType::Raw,
            source: source.to_string(),
            metadata: BTreeMap::new(),
            execution_count: None,
            outputs: None,
            id: None,
        }
    }

    pub fn new_with_type(cell_type: CellType, source: &str) -> Self {
        match cell_type {
            CellType::Code => Self::new_code(source),
            CellType::Markdown => Self::new_markdown(source),
            CellType::Raw => Self::new_raw(source),
        }
    }
}

/// Parse a .ipynb file from JSON
pub fn reads_ipynb(text: &str) -> Result<Notebook, serde_json::Error> {
    serde_json::from_str(text)
}

/// Write a notebook as .ipynb JSON
pub fn writes_ipynb(notebook: &Notebook) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(notebook)
}

/// Get nested metadata value using dot notation: "jupytext.formats"
pub fn get_metadata_nested<'a>(
    metadata: &'a BTreeMap<String, Value>,
    path: &str,
) -> Option<&'a Value> {
    let parts: Vec<&str> = path.split('.').collect();
    let mut current: &Value = metadata.get(parts[0])?;
    for part in &parts[1..] {
        current = current.get(part)?;
    }
    Some(current)
}

/// Set nested metadata value using dot notation
pub fn set_metadata_nested(
    metadata: &mut BTreeMap<String, Value>,
    path: &str,
    value: Value,
) {
    let parts: Vec<&str> = path.split('.').collect();
    if parts.len() == 1 {
        metadata.insert(parts[0].to_string(), value);
        return;
    }

    let entry = metadata
        .entry(parts[0].to_string())
        .or_insert_with(|| Value::Object(serde_json::Map::new()));

    let mut current = entry;
    for part in &parts[1..parts.len() - 1] {
        current = current
            .as_object_mut()
            .unwrap()
            .entry(part.to_string())
            .or_insert_with(|| Value::Object(serde_json::Map::new()));
    }

    if let Some(obj) = current.as_object_mut() {
        obj.insert(parts.last().unwrap().to_string(), value);
    }
}

/// Helper: get a string value from metadata
pub fn metadata_string(metadata: &BTreeMap<String, Value>, key: &str) -> Option<String> {
    metadata.get(key).and_then(|v| v.as_str()).map(|s| s.to_string())
}

/// Helper: get nested string from metadata using path notation
pub fn metadata_nested_string(metadata: &BTreeMap<String, Value>, path: &str) -> Option<String> {
    get_metadata_nested(metadata, path)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}
