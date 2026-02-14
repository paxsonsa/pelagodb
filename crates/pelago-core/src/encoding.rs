//! Encoding utilities: tuple encoding, CBOR helpers, sort-order encoding
//!
//! This module provides encoding utilities for FDB key construction:
//! - Sort-order preserving encoding for integers and floats
//! - Tuple encoding following FDB conventions
//! - CBOR serialization for property values

use crate::{PelagoError, Value};
use bytes::{BufMut, Bytes, BytesMut};

/// Encode i64 for range index (sort-order preserving)
///
/// Flips the sign bit so that negative numbers sort before positive:
/// - Without flip: -1 (0xFF..FF) sorts after 1 (0x00..01)
/// - With flip: -1 (0x7F..FF) sorts before 1 (0x80..01)
pub fn encode_int_for_index(n: i64) -> [u8; 8] {
    let mut bytes = n.to_be_bytes();
    bytes[0] ^= 0x80; // Flip sign bit
    bytes
}

/// Decode i64 from range index encoding
pub fn decode_int_from_index(bytes: &[u8; 8]) -> i64 {
    let mut buf = *bytes;
    buf[0] ^= 0x80;
    i64::from_be_bytes(buf)
}

/// Encode f64 for range index (IEEE 754 order-preserving)
///
/// IEEE 754 floats don't sort correctly as bytes because:
/// - Negative floats have the sign bit set (0x80..), sorting after positives
/// - Negative floats sort in reverse order of magnitude
///
/// This encoding fixes both:
/// - Positive: flip sign bit (0x80 -> 0x00 prefix becomes 0x80 prefix)
/// - Negative: invert all bits (reverses the order and moves below positives)
pub fn encode_float_for_index(f: f64) -> [u8; 8] {
    let bits = f.to_bits();
    let encoded = if bits & 0x8000_0000_0000_0000 != 0 {
        !bits // Negative: invert all bits
    } else {
        bits ^ 0x8000_0000_0000_0000 // Positive: flip sign bit
    };
    encoded.to_be_bytes()
}

/// Decode f64 from range index encoding
pub fn decode_float_from_index(bytes: &[u8; 8]) -> f64 {
    let encoded = u64::from_be_bytes(*bytes);
    let bits = if encoded & 0x8000_0000_0000_0000 == 0 {
        !encoded // Was negative
    } else {
        encoded ^ 0x8000_0000_0000_0000 // Was positive
    };
    f64::from_bits(bits)
}

/// Encode a Value for index key position
pub fn encode_value_for_index(value: &Value) -> Result<Vec<u8>, PelagoError> {
    match value {
        Value::String(s) => Ok(s.as_bytes().to_vec()),
        Value::Int(n) => Ok(encode_int_for_index(*n).to_vec()),
        Value::Float(f) => Ok(encode_float_for_index(*f).to_vec()),
        Value::Timestamp(t) => Ok(encode_int_for_index(*t).to_vec()),
        Value::Bool(b) => Ok(vec![if *b { 0x01 } else { 0x00 }]),
        Value::Bytes(b) => Ok(b.clone()),
        Value::Null => Err(PelagoError::Internal("Cannot encode null for index".into())),
    }
}

/// Serialize properties to CBOR
pub fn encode_cbor<T: serde::Serialize>(value: &T) -> Result<Vec<u8>, PelagoError> {
    let mut buf = Vec::new();
    ciborium::into_writer(value, &mut buf)
        .map_err(|e| PelagoError::Internal(format!("CBOR encode error: {}", e)))?;
    Ok(buf)
}

/// Deserialize from CBOR
pub fn decode_cbor<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, PelagoError> {
    ciborium::from_reader(bytes)
        .map_err(|e| PelagoError::Internal(format!("CBOR decode error: {}", e)))
}

/// Build a tuple-encoded key from components
///
/// Uses FDB tuple layer conventions:
/// - Type byte prefix for each element
/// - Null-terminated strings
/// - Big-endian integers with sign flip
pub struct TupleBuilder {
    buf: BytesMut,
}

impl TupleBuilder {
    pub fn new() -> Self {
        Self {
            buf: BytesMut::new(),
        }
    }

    /// Add string element (type byte 0x02, null-terminated)
    pub fn add_string(mut self, s: &str) -> Self {
        self.buf.put_u8(0x02);
        // Escape any null bytes in the string
        for byte in s.as_bytes() {
            if *byte == 0x00 {
                self.buf.put_u8(0x00);
                self.buf.put_u8(0xFF);
            } else {
                self.buf.put_u8(*byte);
            }
        }
        self.buf.put_u8(0x00);
        self
    }

    /// Add bytes element (type byte 0x01, null-terminated with escaping)
    pub fn add_bytes(mut self, b: &[u8]) -> Self {
        self.buf.put_u8(0x01);
        for byte in b {
            if *byte == 0x00 {
                self.buf.put_u8(0x00);
                self.buf.put_u8(0xFF);
            } else {
                self.buf.put_u8(*byte);
            }
        }
        self.buf.put_u8(0x00);
        self
    }

    /// Add raw bytes without type prefix or null termination
    /// Used for NodeId/EdgeId bytes which have fixed length
    pub fn add_raw_bytes(mut self, b: &[u8]) -> Self {
        self.buf.put_slice(b);
        self
    }

    /// Add i64 element (type byte 0x14, big-endian with sign flip)
    pub fn add_int(mut self, n: i64) -> Self {
        self.buf.put_u8(0x14);
        self.buf.put_slice(&encode_int_for_index(n));
        self
    }

    /// Add a single byte marker (used for subspace prefixes)
    pub fn add_marker(mut self, marker: u8) -> Self {
        self.buf.put_u8(marker);
        self
    }

    /// Build the final key bytes
    pub fn build(self) -> Bytes {
        self.buf.freeze()
    }

    /// Build with a range-end suffix for prefix scans
    pub fn build_range_end(mut self) -> Bytes {
        self.buf.put_u8(0xFF);
        self.buf.freeze()
    }
}

impl Default for TupleBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_int_encoding_preserves_order() {
        let values = vec![-1000i64, -1, 0, 1, 1000];
        let encoded: Vec<_> = values.iter().map(|&v| encode_int_for_index(v)).collect();

        for i in 0..encoded.len() - 1 {
            assert!(encoded[i] < encoded[i + 1], "Order not preserved");
        }
    }

    #[test]
    fn test_int_roundtrip() {
        for n in [-1000i64, -1, 0, 1, 1000, i64::MIN, i64::MAX] {
            let encoded = encode_int_for_index(n);
            let decoded = decode_int_from_index(&encoded);
            assert_eq!(n, decoded);
        }
    }

    #[test]
    fn test_float_encoding_preserves_order() {
        let values = vec![-100.0f64, -0.1, 0.0, 0.1, 100.0];
        let encoded: Vec<_> = values.iter().map(|&v| encode_float_for_index(v)).collect();

        for i in 0..encoded.len() - 1 {
            assert!(
                encoded[i] < encoded[i + 1],
                "Order not preserved: {:?} should be < {:?}",
                encoded[i],
                encoded[i + 1]
            );
        }
    }

    #[test]
    fn test_float_roundtrip() {
        for f in [-100.0f64, -0.1, 0.0, 0.1, 100.0, f64::MIN, f64::MAX] {
            let encoded = encode_float_for_index(f);
            let decoded = decode_float_from_index(&encoded);
            assert_eq!(f, decoded);
        }
    }

    #[test]
    fn test_cbor_roundtrip() {
        let mut props = HashMap::new();
        props.insert("name".to_string(), Value::String("Alice".to_string()));
        props.insert("age".to_string(), Value::Int(30));

        let encoded = encode_cbor(&props).unwrap();
        let decoded: HashMap<String, Value> = decode_cbor(&encoded).unwrap();

        assert_eq!(props, decoded);
    }

    #[test]
    fn test_tuple_builder() {
        let key = TupleBuilder::new()
            .add_string("test")
            .add_string("ns")
            .add_int(42)
            .build();

        // Verify it starts with string type byte
        assert_eq!(key[0], 0x02);
    }

    #[test]
    fn test_encode_value_for_index() {
        assert!(encode_value_for_index(&Value::String("test".into())).is_ok());
        assert!(encode_value_for_index(&Value::Int(42)).is_ok());
        assert!(encode_value_for_index(&Value::Float(3.14)).is_ok());
        assert!(encode_value_for_index(&Value::Bool(true)).is_ok());
        assert!(encode_value_for_index(&Value::Timestamp(1234)).is_ok());
        assert!(encode_value_for_index(&Value::Bytes(vec![1, 2, 3])).is_ok());
        assert!(encode_value_for_index(&Value::Null).is_err());
    }
}
