//! Notebook and cell metadata filtering

use serde_json::Value;
use std::collections::{BTreeMap, HashSet};

use crate::cell_metadata::{is_valid_metadata_key, JUPYTEXT_CELL_METADATA};

/// Default notebook metadata to preserve
pub const DEFAULT_NOTEBOOK_METADATA: &str = "jupytext,kernelspec,kernel_info,orphan,tocdepth";
/// Jupyter metadata namespace key
pub const JUPYTER_METADATA_NAMESPACE: &str = "jupyter";
/// Default root level metadata filter
pub const DEFAULT_ROOT_LEVEL_METADATA: &str = "-all";

/// Metadata filter parsed as a dictionary
#[derive(Debug, Clone, Default)]
pub struct MetadataFilter {
    pub additional: FilterSpec,
    pub excluded: FilterSpec,
}

#[derive(Debug, Clone)]
pub enum FilterSpec {
    All,
    Keys(Vec<String>),
}

impl Default for FilterSpec {
    fn default() -> Self {
        FilterSpec::Keys(Vec::new())
    }
}

impl FilterSpec {
    pub fn is_all(&self) -> bool {
        matches!(self, FilterSpec::All)
    }

    pub fn contains(&self, key: &str) -> bool {
        match self {
            FilterSpec::All => true,
            FilterSpec::Keys(keys) => keys.iter().any(|k| k == key),
        }
    }
}

/// Parse a metadata filter string to a MetadataFilter
pub fn metadata_filter_as_dict(config: &str) -> MetadataFilter {
    let config = config.trim();
    if config.is_empty() {
        return MetadataFilter::default();
    }

    let keys: Vec<&str> = config.split(',').collect();
    let mut additional = Vec::new();
    let mut excluded = Vec::new();
    let mut has_all_additional = false;
    let mut has_all_excluded = false;

    for key in keys {
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        if key.starts_with('-') {
            let k = &key[1..];
            if k == "all" {
                has_all_excluded = true;
            } else if !k.is_empty() {
                excluded.push(k.to_string());
            }
        } else if key.starts_with('+') {
            let k = &key[1..];
            if k == "all" {
                has_all_additional = true;
            } else if !k.is_empty() {
                additional.push(k.to_string());
            }
        } else if key == "all" {
            has_all_additional = true;
        } else {
            additional.push(key.to_string());
        }
    }

    MetadataFilter {
        additional: if has_all_additional {
            FilterSpec::All
        } else {
            FilterSpec::Keys(additional)
        },
        excluded: if has_all_excluded {
            FilterSpec::All
        } else {
            FilterSpec::Keys(excluded)
        },
    }
}

/// Convert a MetadataFilter back to a string
pub fn metadata_filter_as_string(filter: &MetadataFilter) -> String {
    let mut entries = Vec::new();

    match &filter.additional {
        FilterSpec::All => entries.push("all".to_string()),
        FilterSpec::Keys(keys) => {
            for key in keys {
                if !JUPYTEXT_CELL_METADATA.contains(&key.as_str()) {
                    entries.push(key.clone());
                }
            }
        }
    }

    match &filter.excluded {
        FilterSpec::All => entries.push("-all".to_string()),
        FilterSpec::Keys(keys) => {
            for key in keys {
                entries.push(format!("-{}", key));
            }
        }
    }

    entries.join(",")
}

/// Filter metadata according to user and default filters
pub fn filter_metadata(
    metadata: &BTreeMap<String, Value>,
    user_filter: &str,
    default_filter: &str,
) -> BTreeMap<String, Value> {
    let default = metadata_filter_as_dict(default_filter);
    let user = metadata_filter_as_dict(user_filter);

    // If default says exclude all (like notebook metadata)
    if default.excluded.is_all() {
        if user.additional.is_all() {
            // Include all except user excluded
            return subset_metadata(metadata, None, Some(&user.excluded));
        }
        if user.excluded.is_all() {
            // Include only user additional
            return subset_metadata(metadata, Some(&user.additional), None);
        }
        // Include default additional + user additional, minus user excluded
        let combined = match (&default.additional, &user.additional) {
            (FilterSpec::Keys(d), FilterSpec::Keys(u)) => {
                let mut combined: HashSet<String> = d.iter().cloned().collect();
                combined.extend(u.iter().cloned());
                FilterSpec::Keys(combined.into_iter().collect())
            }
            _ => FilterSpec::All,
        };
        return subset_metadata(metadata, Some(&combined), Some(&user.excluded));
    }

    // Default includes most (cell metadata case)
    if user.additional.is_all() {
        return subset_metadata(metadata, None, Some(&user.excluded));
    }
    if user.excluded.is_all() {
        return subset_metadata(metadata, Some(&user.additional), None);
    }

    // Remove empty tags
    let mut metadata = metadata.clone();
    if let Some(Value::Array(tags)) = metadata.get("tags") {
        if tags.is_empty() {
            metadata.remove("tags");
        }
    }

    // Default exclude minus user include, plus user exclude
    let effective_exclude = match (&default.excluded, &user.additional) {
        (FilterSpec::Keys(de), FilterSpec::Keys(ua)) => {
            let ua_set: HashSet<&str> = ua.iter().map(|s| s.as_str()).collect();
            let mut result: Vec<String> = de
                .iter()
                .filter(|k| !ua_set.contains(k.as_str()))
                .cloned()
                .collect();
            if let FilterSpec::Keys(ue) = &user.excluded {
                result.extend(ue.iter().cloned());
            }
            FilterSpec::Keys(result)
        }
        _ => user.excluded.clone(),
    };

    subset_metadata(&metadata, None, Some(&effective_exclude))
}

/// Subset metadata based on keep_only or exclude lists
fn subset_metadata(
    metadata: &BTreeMap<String, Value>,
    keep_only: Option<&FilterSpec>,
    exclude: Option<&FilterSpec>,
) -> BTreeMap<String, Value> {
    let mut result = BTreeMap::new();

    let keys: Vec<&String> = metadata
        .keys()
        .filter(|k| is_valid_metadata_key(k))
        .collect();

    if let Some(keep) = keep_only {
        for key in &keys {
            if keep.contains(key) {
                result.insert(key.to_string(), metadata[*key].clone());
            }
        }
    } else {
        for key in &keys {
            result.insert(key.to_string(), metadata[*key].clone());
        }
    }

    if let Some(exc) = exclude {
        match exc {
            FilterSpec::All => {
                result.clear();
            }
            FilterSpec::Keys(exc_keys) => {
                for key in exc_keys {
                    result.remove(key);
                }
            }
        }
    }

    result
}

/// Update metadata filters when reading a notebook
pub fn update_metadata_filters(
    metadata: &mut serde_json::Map<String, Value>,
    has_jupyter_md: bool,
    cell_metadata: &HashSet<String>,
) {
    if !has_jupyter_md {
        // Set metadata filter equal to current metadata in script
        let jupytext = metadata
            .entry("jupytext".to_string())
            .or_insert_with(|| Value::Object(serde_json::Map::new()));
        if let Some(obj) = jupytext.as_object_mut() {
            obj.insert(
                "notebook_metadata_filter".to_string(),
                Value::String("-all".to_string()),
            );
            if !obj.contains_key("cell_metadata_filter") {
                let mut filter_parts: Vec<String> = cell_metadata
                    .iter()
                    .filter(|k| !JUPYTEXT_CELL_METADATA.contains(&k.as_str()))
                    .cloned()
                    .collect();
                filter_parts.sort();
                filter_parts.push("-all".to_string());
                obj.insert(
                    "cell_metadata_filter".to_string(),
                    Value::String(filter_parts.join(",")),
                );
            }
        }
    }
}
