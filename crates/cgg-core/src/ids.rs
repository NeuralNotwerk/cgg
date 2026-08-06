//! Strongly-typed identifiers for graph entities.
//!
//! Every `Id` type is a transparent wrapper over `u32` so the graph
//! stays compact in memory while the type system prevents mixing a
//! callable id with a file id.

use serde::{Deserialize, Serialize};
use std::fmt;

macro_rules! id_type {
    ($name:ident, $prefix:literal) => {
        #[derive(
            Copy,
            Clone,
            Debug,
            Default,
            Eq,
            PartialEq,
            Ord,
            PartialOrd,
            Hash,
            Serialize,
            Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub u32);

        impl $name {
            #[inline]
            pub const fn new(v: u32) -> Self {
                Self(v)
            }

            #[inline]
            pub const fn as_u32(self) -> u32 {
                self.0
            }

            #[inline]
            pub const fn as_usize(self) -> usize {
                self.0 as usize
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}{}", $prefix, self.0)
            }
        }

        impl From<u32> for $name {
            #[inline]
            fn from(v: u32) -> Self {
                Self(v)
            }
        }
    };
}

id_type!(CallableId, "C");
id_type!(FileId, "F");

/// Identifies which resolver produced an edge.
///
/// Kept as an interned string because the universe is small, predictable,
/// and we want friendly values in the audit output
/// (`"stack-graphs:python"`, `"tsg:rust"`, `"ffi:c-abi"`,
/// `"intra-file"`, `"custom:c"`, `"custom:cpp"`).
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ResolverId(pub String);

impl ResolverId {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ResolverId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for ResolverId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_uses_the_type_prefix() {
        // The prefix is what makes an id readable in mermaid/dot node
        // names and in the audit log, and it must differ per type.
        assert_eq!(CallableId::new(7).to_string(), "C7");
        assert_eq!(FileId::new(7).to_string(), "F7");
    }

    #[test]
    fn accessors_round_trip() {
        let c = CallableId::new(42);
        assert_eq!(c.as_u32(), 42);
        assert_eq!(c.as_usize(), 42usize);
        assert_eq!(CallableId::from(42u32), c);
        assert_eq!(FileId::default(), FileId::new(0));
    }

    #[test]
    fn ids_order_and_hash_by_value() {
        use std::collections::HashSet;
        assert!(CallableId::new(1) < CallableId::new(2));
        let set: HashSet<_> = [CallableId::new(1), CallableId::new(1)]
            .into_iter()
            .collect();
        assert_eq!(set.len(), 1, "equal ids must collapse in a set");
    }

    #[test]
    fn ids_serialize_transparently() {
        // `#[serde(transparent)]` is load-bearing: the JSON format
        // documents edges as `{"src": 0, ...}`, not `{"src": {"0": 0}}`.
        assert_eq!(serde_json::to_string(&CallableId::new(3)).unwrap(), "3");
        assert_eq!(serde_json::to_string(&FileId::new(4)).unwrap(), "4");
        let back: CallableId = serde_json::from_str("3").unwrap();
        assert_eq!(back, CallableId::new(3));
    }

    #[test]
    fn resolver_id_is_a_transparent_string() {
        let r = ResolverId::new("ffi:c-abi");
        assert_eq!(r.as_str(), "ffi:c-abi");
        assert_eq!(r.to_string(), "ffi:c-abi");
        assert_eq!(ResolverId::from("intra-file").as_str(), "intra-file");
        assert_eq!(serde_json::to_string(&r).unwrap(), "\"ffi:c-abi\"");
        let back: ResolverId = serde_json::from_str("\"tsg:rust\"").unwrap();
        assert_eq!(back, ResolverId::new("tsg:rust"));
    }
}
