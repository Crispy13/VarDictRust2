use serde::Serialize;
use std::fmt;
use std::io;
use std::path::Path;

use super::snapshot::load_golden;

/// A single field-level difference between golden and actual.
#[derive(Debug, Clone)]
pub struct DiffEntry {
    pub path: String,
    pub java_value: String,
    pub rust_value: String,
}

/// Errors that can occur during parity assertion.
#[derive(Debug)]
pub enum ParityError {
    Io(io::Error),
    Json(serde_json::Error),
    MissingGoldenData,
    Mismatch {
        byte_position: usize,
        actual_snippet: String,
        golden_snippet: String,
    },
}

impl fmt::Display for ParityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "I/O error: {error}"),
            Self::Json(error) => write!(f, "JSON error: {error}"),
            Self::MissingGoldenData => write!(f, "golden data file is missing or empty"),
            Self::Mismatch {
                byte_position,
                actual_snippet,
                golden_snippet,
            } => write!(
                f,
                "parity mismatch at byte {byte_position}:\n  actual:  {actual_snippet}\n  golden:  {golden_snippet}"
            ),
        }
    }
}

impl std::error::Error for ParityError {}

impl From<io::Error> for ParityError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for ParityError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

/// Byte-level JSON string comparison between golden file and actual data.
/// Returns Ok(()) on exact match, Mismatch with diagnostic snippet otherwise.
pub fn assert_module_parity<T: Serialize>(
    golden_path: &Path,
    actual: &T,
) -> Result<(), ParityError> {
    let golden = load_golden(golden_path)?;
    let actual_json = serde_json::to_string(actual)?;

    if golden == actual_json {
        return Ok(());
    }

    let byte_position = golden
        .bytes()
        .zip(actual_json.bytes())
        .position(|(golden_byte, actual_byte)| golden_byte != actual_byte)
        .unwrap_or(golden.len().min(actual_json.len()));

    let snippet_start = byte_position.saturating_sub(20);
    let snippet_end_golden = (byte_position + 40).min(golden.len());
    let snippet_end_actual = (byte_position + 40).min(actual_json.len());

    Err(ParityError::Mismatch {
        byte_position,
        actual_snippet: actual_json[snippet_start..snippet_end_actual].to_string(),
        golden_snippet: golden[snippet_start..snippet_end_golden].to_string(),
    })
}

/// Tree-walk diff: returns all field-level differences as DiffEntry values.
pub fn structured_diff<T: Serialize>(
    golden_path: &Path,
    actual: &T,
) -> Result<Vec<DiffEntry>, ParityError> {
    let golden_str = load_golden(golden_path)?;
    let golden_val: serde_json::Value = serde_json::from_str(&golden_str)?;
    let actual_val = serde_json::to_value(actual)?;

    let mut diffs = Vec::new();
    diff_values("$", &golden_val, &actual_val, &mut diffs);
    Ok(diffs)
}

fn diff_values(
    path: &str,
    golden: &serde_json::Value,
    actual: &serde_json::Value,
    diffs: &mut Vec<DiffEntry>,
) {
    use serde_json::Value;

    match (golden, actual) {
        (Value::Object(golden_map), Value::Object(actual_map)) => {
            for (key, golden_value) in golden_map {
                let child_path = format!("{path}.{key}");
                match actual_map.get(key) {
                    Some(actual_value) => {
                        diff_values(&child_path, golden_value, actual_value, diffs)
                    }
                    None => diffs.push(DiffEntry {
                        path: child_path,
                        java_value: golden_value.to_string(),
                        rust_value: "<missing>".to_string(),
                    }),
                }
            }
            for key in actual_map.keys() {
                if !golden_map.contains_key(key) {
                    diffs.push(DiffEntry {
                        path: format!("{path}.{key}"),
                        java_value: "<missing>".to_string(),
                        rust_value: actual_map[key].to_string(),
                    });
                }
            }
        }
        (Value::Array(golden_values), Value::Array(actual_values)) => {
            let max_len = golden_values.len().max(actual_values.len());
            for index in 0..max_len {
                let child_path = format!("{path}[{index}]");
                match (golden_values.get(index), actual_values.get(index)) {
                    (Some(golden_value), Some(actual_value)) => {
                        diff_values(&child_path, golden_value, actual_value, diffs)
                    }
                    (Some(golden_value), None) => diffs.push(DiffEntry {
                        path: child_path,
                        java_value: golden_value.to_string(),
                        rust_value: "<missing>".to_string(),
                    }),
                    (None, Some(actual_value)) => diffs.push(DiffEntry {
                        path: child_path,
                        java_value: "<missing>".to_string(),
                        rust_value: actual_value.to_string(),
                    }),
                    (None, None) => unreachable!(),
                }
            }
        }
        _ => {
            if golden != actual {
                diffs.push(DiffEntry {
                    path: path.to_string(),
                    java_value: golden.to_string(),
                    rust_value: actual.to_string(),
                });
            }
        }
    }
}
