//! FDB subspace definitions for key layout
//!
//! PelagoDB uses a hierarchical keyspace layout:
//! ```text
//! (db, ns, subspace, ...)
//! ```
//!
//! Subspaces:
//! - `_sys`: System metadata (site registry, etc.)
//! - `_db`: Database metadata
//! - `_ns`: Namespace metadata
//! - `_schema`: Entity schemas
//! - `_cdc`: Change data capture entries
//! - `_jobs`: Background job state
//! - `_ids`: ID allocation counters
//! - `data`: Node data
//! - `loc`: Locality index
//! - `idx`: Property indexes
//! - `edge`: Edge storage

use bytes::{BufMut, Bytes, BytesMut};
use pelago_core::encoding::TupleBuilder;

/// Subspace markers for FDB key prefixes
pub mod markers {
    pub const SYS: &str = "_sys";
    pub const DB: &str = "_db";
    pub const NS: &str = "_ns";
    pub const SCHEMA: &str = "_schema";
    pub const CDC: &str = "_cdc";
    pub const JOBS: &str = "_jobs";
    pub const IDS: &str = "_ids";
    pub const DATA: &str = "data";
    pub const LOC: &str = "loc";
    pub const IDX: &str = "idx";
    pub const EDGE: &str = "edge";
    pub const META: &str = "_meta";
}

/// Edge key markers
pub mod edge_markers {
    /// Forward edge
    pub const FORWARD: u8 = b'f';
    /// Forward edge metadata
    pub const FORWARD_META: u8 = b'm';
    /// Reverse edge
    pub const REVERSE: u8 = b'r';
}

/// Subspace helper for building FDB keys
#[derive(Clone)]
pub struct Subspace {
    prefix: Bytes,
}

impl Subspace {
    /// Create a new root subspace for a database
    pub fn root() -> Self {
        Self {
            prefix: Bytes::new(),
        }
    }

    /// Create a subspace for a database
    pub fn database(db: &str) -> Self {
        Self {
            prefix: TupleBuilder::new().add_string(db).build(),
        }
    }

    /// Create a subspace for a namespace within a database
    pub fn namespace(db: &str, ns: &str) -> Self {
        Self {
            prefix: TupleBuilder::new().add_string(db).add_string(ns).build(),
        }
    }

    /// Create the global system metadata subspace.
    pub fn system() -> Self {
        Self::root().subspace(markers::SYS)
    }

    /// Get the schema subspace
    pub fn schema(&self) -> Self {
        self.subspace(markers::SCHEMA)
    }

    /// Get the data subspace
    pub fn data(&self) -> Self {
        self.subspace(markers::DATA)
    }

    /// Get the index subspace
    pub fn index(&self) -> Self {
        self.subspace(markers::IDX)
    }

    /// Get the edge subspace
    pub fn edge(&self) -> Self {
        self.subspace(markers::EDGE)
    }

    /// Get the CDC subspace
    pub fn cdc(&self) -> Self {
        self.subspace(markers::CDC)
    }

    /// Get the jobs subspace
    pub fn jobs(&self) -> Self {
        self.subspace(markers::JOBS)
    }

    /// Get the IDs subspace
    pub fn ids(&self) -> Self {
        self.subspace(markers::IDS)
    }

    /// Get the meta subspace (for checkpoints, counters, etc.)
    pub fn meta(&self) -> Self {
        self.subspace(markers::META)
    }

    /// Create a named child subspace.
    pub fn child(&self, name: &str) -> Self {
        self.subspace(name)
    }

    /// Create a child subspace
    fn subspace(&self, name: &str) -> Self {
        let mut buf = BytesMut::from(self.prefix.as_ref());
        // Add string tuple element
        buf.put_u8(0x02);
        buf.put_slice(name.as_bytes());
        buf.put_u8(0x00);
        Self {
            prefix: buf.freeze(),
        }
    }

    /// Get the prefix bytes
    pub fn prefix(&self) -> &[u8] {
        &self.prefix
    }

    /// Pack additional elements into a key
    pub fn pack(&self) -> TupleBuilder {
        let mut builder = TupleBuilder::new();
        // Start with our prefix
        builder = builder.add_raw_bytes(&self.prefix);
        builder
    }

    /// Create a range end key for prefix scans
    pub fn range_end(&self) -> Bytes {
        let mut buf = BytesMut::from(self.prefix.as_ref());
        buf.put_u8(0xFF);
        buf.freeze()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_subspace_hierarchy() {
        let ns = Subspace::namespace("mydb", "default");
        let schema = ns.schema();
        let data = ns.data();

        // Verify prefixes are different
        assert_ne!(schema.prefix(), data.prefix());

        // Verify schema prefix contains namespace prefix
        assert!(schema.prefix().starts_with(ns.prefix()));
    }
}
