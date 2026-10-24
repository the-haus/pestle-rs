//! What an extraction returns: plain values, with objects made cheaply.
//!
//! An object of a spec's fields is a [`Value::Record`]: the field names,
//! shared by every object of that shape, and its values in the same order
//! (one allocation, no hashing, keys in the spec's order). Values parsed as
//! JSON or shaped by a grammar are kept as `serde_json` values. The whole
//! serializes with serde (to JavaScript, to JSON text), and
//! [`Value::to_json`] makes a `serde_json::Value` of it.

use std::sync::Arc;

use serde::ser::{SerializeMap, SerializeSeq};
use serde::{Serialize, Serializer};

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Value>),
    /// An object: its keys (shared by all objects of its shape) and values, in order.
    Record(Arc<[String]>, Box<[Value]>),
    /// A value parsed as JSON or shaped by a grammar.
    Json(serde_json::Value),
}

impl Value {
    /// The same as a `serde_json::Value`.
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            Value::Null => serde_json::Value::Null,
            Value::Bool(b) => serde_json::Value::Bool(*b),
            Value::Number(n) => {
                serde_json::Number::from_f64(*n).map_or(serde_json::Value::Null, serde_json::Value::Number)
            }
            Value::String(s) => serde_json::Value::String(s.clone()),
            Value::Array(items) => serde_json::Value::Array(items.iter().map(Value::to_json).collect()),
            Value::Record(keys, values) => serde_json::Value::Object(
                keys.iter()
                    .zip(values.iter())
                    .map(|(k, v)| (k.clone(), v.to_json()))
                    .collect(),
            ),
            Value::Json(v) => v.clone(),
        }
    }

    /// An object's value under `key`.
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Record(keys, values) => keys.iter().position(|k| k == key).map(|i| &values[i]),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(items) => Some(items),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            Value::Json(serde_json::Value::String(s)) => Some(s),
            _ => None,
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null | Value::Json(serde_json::Value::Null))
    }
}

impl From<serde_json::Value> for Value {
    fn from(v: serde_json::Value) -> Value {
        match v {
            serde_json::Value::Null => Value::Null,
            serde_json::Value::Bool(b) => Value::Bool(b),
            serde_json::Value::String(s) => Value::String(s),
            other => Value::Json(other),
        }
    }
}

impl PartialEq<serde_json::Value> for Value {
    fn eq(&self, other: &serde_json::Value) -> bool {
        self.to_json() == *other
    }
}

impl Serialize for Value {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Value::Null => s.serialize_unit(),
            Value::Bool(b) => s.serialize_bool(*b),
            Value::Number(n) if n.is_finite() => s.serialize_f64(*n),
            Value::Number(_) => s.serialize_unit(),
            Value::String(v) => s.serialize_str(v),
            Value::Array(items) => {
                let mut seq = s.serialize_seq(Some(items.len()))?;
                for item in items {
                    seq.serialize_element(item)?;
                }
                seq.end()
            }
            Value::Record(keys, values) => {
                let mut map = s.serialize_map(Some(values.len()))?;
                for (k, v) in keys.iter().zip(values.iter()) {
                    map.serialize_entry(k, v)?;
                }
                map.end()
            }
            Value::Json(v) => v.serialize(s),
        }
    }
}
