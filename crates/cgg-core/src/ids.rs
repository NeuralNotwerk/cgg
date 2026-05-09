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
            Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize,
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
