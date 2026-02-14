//! Core PelagoDB types: NodeId, EdgeId, Value, PropertyType

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Node ID: 9 bytes encoding site + sequence
///
/// The format is designed for:
/// - **Site extraction**: First byte identifies originating site for routing
/// - **No coordination**: Each site allocates IDs independently
/// - **Compact storage**: 9 bytes vs 16 for UUID
/// - **Sort order**: Big-endian encoding enables range scans by site
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodeId {
    pub site: u8,
    pub seq: u64,
}

impl NodeId {
    pub fn new(site: u8, seq: u64) -> Self {
        Self { site, seq }
    }

    /// Serialize to 9 bytes for FDB key
    pub fn to_bytes(&self) -> [u8; 9] {
        let mut buf = [0u8; 9];
        buf[0] = self.site;
        buf[1..9].copy_from_slice(&self.seq.to_be_bytes());
        buf
    }

    /// Deserialize from 9 bytes
    pub fn from_bytes(bytes: &[u8; 9]) -> Self {
        Self {
            site: bytes[0],
            seq: u64::from_be_bytes(bytes[1..9].try_into().unwrap()),
        }
    }

    /// Try to deserialize from a byte slice (must be exactly 9 bytes)
    pub fn try_from_slice(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != 9 {
            return None;
        }
        let mut arr = [0u8; 9];
        arr.copy_from_slice(bytes);
        Some(Self::from_bytes(&arr))
    }

    /// Extract originating site
    pub fn origin_site(&self) -> u8 {
        self.site
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}_{}", self.site, self.seq)
    }
}

impl FromStr for NodeId {
    type Err = ParseIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split('_').collect();
        if parts.len() != 2 {
            return Err(ParseIdError::InvalidFormat);
        }
        Ok(Self {
            site: parts[0].parse().map_err(|_| ParseIdError::InvalidSite)?,
            seq: parts[1].parse().map_err(|_| ParseIdError::InvalidSeq)?,
        })
    }
}

/// Edge ID: same 9-byte format as NodeId
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EdgeId {
    pub site: u8,
    pub seq: u64,
}

impl EdgeId {
    pub fn new(site: u8, seq: u64) -> Self {
        Self { site, seq }
    }

    pub fn to_bytes(&self) -> [u8; 9] {
        let mut buf = [0u8; 9];
        buf[0] = self.site;
        buf[1..9].copy_from_slice(&self.seq.to_be_bytes());
        buf
    }

    pub fn from_bytes(bytes: &[u8; 9]) -> Self {
        Self {
            site: bytes[0],
            seq: u64::from_be_bytes(bytes[1..9].try_into().unwrap()),
        }
    }

    /// Try to deserialize from a byte slice (must be exactly 9 bytes)
    pub fn try_from_slice(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != 9 {
            return None;
        }
        let mut arr = [0u8; 9];
        arr.copy_from_slice(bytes);
        Some(Self::from_bytes(&arr))
    }
}

impl fmt::Display for EdgeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}_{}", self.site, self.seq)
    }
}

impl FromStr for EdgeId {
    type Err = ParseIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split('_').collect();
        if parts.len() != 2 {
            return Err(ParseIdError::InvalidFormat);
        }
        Ok(Self {
            site: parts[0].parse().map_err(|_| ParseIdError::InvalidSite)?,
            seq: parts[1].parse().map_err(|_| ParseIdError::InvalidSeq)?,
        })
    }
}

/// Property value enum matching spec §4.3
///
/// All property values in PelagoDB are one of these types.
/// Null is a distinct value type that matches any PropertyType.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Value {
    String(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    Timestamp(i64), // Unix microseconds
    Bytes(Vec<u8>),
    Null,
}

impl Value {
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::String(_) => "string",
            Value::Int(_) => "int",
            Value::Float(_) => "float",
            Value::Bool(_) => "bool",
            Value::Timestamp(_) => "timestamp",
            Value::Bytes(_) => "bytes",
            Value::Null => "null",
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    /// Get string value if this is a String variant
    pub fn as_string(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    /// Get i64 value if this is an Int variant
    pub fn as_int(&self) -> Option<i64> {
        match self {
            Value::Int(n) => Some(*n),
            _ => None,
        }
    }

    /// Get f64 value if this is a Float variant
    pub fn as_float(&self) -> Option<f64> {
        match self {
            Value::Float(f) => Some(*f),
            _ => None,
        }
    }

    /// Get bool value if this is a Bool variant
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// Get timestamp value if this is a Timestamp variant
    pub fn as_timestamp(&self) -> Option<i64> {
        match self {
            Value::Timestamp(t) => Some(*t),
            _ => None,
        }
    }

    /// Get bytes value if this is a Bytes variant
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Value::Bytes(b) => Some(b),
            _ => None,
        }
    }
}

/// Property type enum for schema definitions
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PropertyType {
    String,
    Int,
    Float,
    Bool,
    Timestamp,
    Bytes,
}

impl PropertyType {
    /// Check if a value matches this property type
    /// Null matches any type (nullable by default)
    pub fn matches(&self, value: &Value) -> bool {
        matches!(
            (self, value),
            (PropertyType::String, Value::String(_))
                | (PropertyType::Int, Value::Int(_))
                | (PropertyType::Float, Value::Float(_))
                | (PropertyType::Bool, Value::Bool(_))
                | (PropertyType::Timestamp, Value::Timestamp(_))
                | (PropertyType::Bytes, Value::Bytes(_))
                | (_, Value::Null) // Null matches any type
        )
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            PropertyType::String => "string",
            PropertyType::Int => "int",
            PropertyType::Float => "float",
            PropertyType::Bool => "bool",
            PropertyType::Timestamp => "timestamp",
            PropertyType::Bytes => "bytes",
        }
    }
}

impl fmt::Display for PropertyType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ParseIdError {
    #[error("invalid ID format, expected 'site_seq'")]
    InvalidFormat,
    #[error("invalid site ID")]
    InvalidSite,
    #[error("invalid sequence number")]
    InvalidSeq,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_node_id_roundtrip() {
        let id = NodeId::new(42, 12345);
        let bytes = id.to_bytes();
        let recovered = NodeId::from_bytes(&bytes);
        assert_eq!(id, recovered);
    }

    #[test]
    fn test_node_id_display_parse() {
        let id = NodeId::new(1, 100);
        let s = id.to_string();
        assert_eq!(s, "1_100");
        let parsed: NodeId = s.parse().unwrap();
        assert_eq!(id, parsed);
    }

    #[test]
    fn test_edge_id_roundtrip() {
        let id = EdgeId::new(5, 999999);
        let bytes = id.to_bytes();
        let recovered = EdgeId::from_bytes(&bytes);
        assert_eq!(id, recovered);
    }

    #[test]
    fn test_property_type_matches() {
        assert!(PropertyType::String.matches(&Value::String("hello".into())));
        assert!(PropertyType::Int.matches(&Value::Int(42)));
        assert!(PropertyType::Float.matches(&Value::Float(3.14)));
        assert!(PropertyType::Bool.matches(&Value::Bool(true)));
        assert!(PropertyType::Timestamp.matches(&Value::Timestamp(1234567890)));
        assert!(PropertyType::Bytes.matches(&Value::Bytes(vec![1, 2, 3])));

        // Null matches any type
        assert!(PropertyType::String.matches(&Value::Null));
        assert!(PropertyType::Int.matches(&Value::Null));

        // Type mismatches
        assert!(!PropertyType::String.matches(&Value::Int(42)));
        assert!(!PropertyType::Int.matches(&Value::String("42".into())));
    }

    #[test]
    fn test_value_accessors() {
        assert_eq!(Value::String("test".into()).as_string(), Some("test"));
        assert_eq!(Value::Int(42).as_int(), Some(42));
        assert_eq!(Value::Float(3.14).as_float(), Some(3.14));
        assert_eq!(Value::Bool(true).as_bool(), Some(true));
        assert_eq!(Value::Timestamp(1000).as_timestamp(), Some(1000));
        assert_eq!(Value::Bytes(vec![1, 2]).as_bytes(), Some(&[1, 2][..]));
        assert!(Value::Null.is_null());
    }
}
