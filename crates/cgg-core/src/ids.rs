//! Strongly-typed identifiers for graph entities.
//!
//! Every `Id` type is a transparent wrapper over `u64` so it can hold a
//! content-derived hash (see `StableIds` in the `cgg` crate) rather than
//! a sequential per-run counter. A hash-derived id is stable across runs
//! that add or remove unrelated nodes — the whole point of moving off
//! sequential integers — while the type system still prevents mixing a
//! callable id with a file id.
//!
//! On the wire (JSON, mermaid, dot, graphml) an id is rendered as its
//! type prefix followed by the base36 encoding of the inner value
//! (`"C4k2j9qh3xz"`), not a decimal number — base36 keeps a 52-bit hash
//! to about 10 characters instead of 16, and the non-numeric wire form is
//! a visible reminder that these are not sequential indices to diff
//! across runs.

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::str::FromStr;

const BASE36_ALPHABET: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";

/// Encode `v` as lowercase base36 (alphabet `0-9a-z`), no leading-zero
/// padding — matches how `u64::to_string()` has no padding in base10.
pub fn to_base36(mut v: u64) -> String {
    if v == 0 {
        return "0".to_string();
    }
    let mut digits = Vec::with_capacity(13); // ceil(log36(u64::MAX)) = 13
    while v > 0 {
        digits.push(BASE36_ALPHABET[(v % 36) as usize]);
        v /= 36;
    }
    digits.reverse();
    // SAFETY: every byte pushed above comes from BASE36_ALPHABET, which is
    // ASCII.
    String::from_utf8(digits).expect("base36 digits are always valid UTF-8")
}

/// Decode a lowercase (or uppercase) base36 string back into a `u64`.
/// Returns `None` on an empty string or a non-base36 character.
pub fn from_base36(s: &str) -> Option<u64> {
    if s.is_empty() {
        return None;
    }
    let mut v: u64 = 0;
    for c in s.chars() {
        let digit = c.to_digit(36)?;
        v = v.checked_mul(36)?.checked_add(digit as u64)?;
    }
    Some(v)
}

/// Error returned by `FromStr` for an id type: the string was missing
/// the type's prefix character, or what followed it was not base36.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdParseError(pub String);

impl fmt::Display for IdParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid id string: {:?}", self.0)
    }
}

impl std::error::Error for IdParseError {}

macro_rules! id_type {
    ($name:ident, $prefix:literal) => {
        #[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash)]
        pub struct $name(pub u64);

        impl $name {
            #[inline]
            pub const fn new(v: u32) -> Self {
                Self(v as u64)
            }

            #[inline]
            pub const fn new_u64(v: u64) -> Self {
                Self(v)
            }

            /// Truncating u32 view of the inner value. Content-derived ids
            /// routinely exceed 32 bits, so this loses information — it
            /// exists for API back-compat and small hand-built test ids,
            /// never use it as a dedup/sort key on a real analyzed graph.
            #[inline]
            pub const fn as_u32(self) -> u32 {
                self.0 as u32
            }

            #[inline]
            pub const fn as_u64(self) -> u64 {
                self.0
            }

            #[inline]
            pub const fn as_usize(self) -> usize {
                self.0 as usize
            }

            /// Base36 digits only, no type prefix — for formatters that
            /// want to prepend their own node-name prefix character.
            #[inline]
            pub fn token(self) -> String {
                to_base36(self.0)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}{}", $prefix, to_base36(self.0))
            }
        }

        impl FromStr for $name {
            type Err = IdParseError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                let rest = s
                    .strip_prefix($prefix)
                    .ok_or_else(|| IdParseError(s.to_string()))?;
                let v = from_base36(rest).ok_or_else(|| IdParseError(s.to_string()))?;
                Ok(Self(v))
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.serialize_str(&self.to_string())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let s = String::deserialize(deserializer)?;
                s.parse::<Self>().map_err(D::Error::custom)
            }
        }

        impl From<u32> for $name {
            #[inline]
            fn from(v: u32) -> Self {
                Self(v as u64)
            }
        }

        impl From<u64> for $name {
            #[inline]
            fn from(v: u64) -> Self {
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
    fn base36_round_trips() {
        for v in [0u64, 1, 35, 36, 42, 12345, u32::MAX as u64, u64::MAX] {
            let s = to_base36(v);
            assert!(
                s.chars()
                    .all(|c| c.is_ascii_digit() || c.is_ascii_lowercase()),
                "base36 output must be lowercase alnum: {s:?}"
            );
            assert_eq!(from_base36(&s), Some(v));
        }
    }

    #[test]
    fn display_uses_the_type_prefix_and_base36() {
        // The prefix is what makes an id readable in mermaid/dot node
        // names and in the audit log, and it must differ per type.
        assert_eq!(CallableId::new(7).to_string(), "C7");
        assert_eq!(FileId::new(7).to_string(), "F7");
        // 36 is the first value whose base36 encoding is two digits.
        assert_eq!(CallableId::new(36).to_string(), "C10");
        // A value that needs the full u64 range still round-trips.
        let big = CallableId::new_u64(0x000f_ffff_ffff_ffff); // 52 bits set
        let shown = big.to_string();
        assert!(shown.starts_with('C'));
        assert_eq!(shown.parse::<CallableId>().unwrap(), big);
    }

    #[test]
    fn from_str_round_trips_and_rejects_garbage() {
        let id = CallableId::new_u64(123_456_789);
        let s = id.to_string();
        assert_eq!(s.parse::<CallableId>().unwrap(), id);

        // Wrong prefix.
        assert!("F123".parse::<CallableId>().is_err());
        // No prefix at all.
        assert!("123".parse::<CallableId>().is_err());
        // Non-base36 body.
        assert!("C!!!".parse::<CallableId>().is_err());
    }

    #[test]
    fn token_has_no_prefix() {
        let id = CallableId::new(42);
        assert_eq!(id.token(), to_base36(42));
        assert!(!id.token().contains('C'));
    }

    #[test]
    fn accessors_round_trip() {
        let c = CallableId::new(42);
        assert_eq!(c.as_u32(), 42);
        assert_eq!(c.as_u64(), 42u64);
        assert_eq!(c.as_usize(), 42usize);
        assert_eq!(CallableId::from(42u32), c);
        assert_eq!(CallableId::from(42u64), c);
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
    fn ids_serialize_as_json_strings() {
        // Ids are content-derived hashes now, not sequential indices, so
        // the wire form is a prefixed base36 *string* rather than a bare
        // JSON number: `{"src": "c3", ...}`, not `{"src": 3}`.
        assert_eq!(
            serde_json::to_string(&CallableId::new(3)).unwrap(),
            "\"C3\""
        );
        assert_eq!(serde_json::to_string(&FileId::new(4)).unwrap(), "\"F4\"");
        let back: CallableId = serde_json::from_str("\"C3\"").unwrap();
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
