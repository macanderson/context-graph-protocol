//! [`ExtensionValue`] — this crate's own JSON value model, for the parts of the
//! wire the reference types deliberately leave open.
//!
//! # Why a value model here at all
//!
//! `SPEC.md` §13 U1 requires a receiver to **ignore** members it does not
//! recognise, and §13 U3 makes namespaced extension members (`vendor:name`) a
//! first-class part of the wire. "Ignore" is not "discard": a host that relays a
//! record, or content-addresses one with
//! [`record_hash_of`](crate::record_attest::record_hash_of), has to carry the
//! members it does not understand through untouched, or it changes the bytes it
//! was handed. So the reference types need somewhere to *keep* an unmodelled
//! member, and keeping one requires a type that can hold any JSON value.
//!
//! # Why not `serde_json::Value`
//!
//! `contextgraph-types` is **MIT licensed with zero dependencies beyond
//! `serde`** — that promise is in the crate's `Cargo.toml`, its `README.md`, and
//! the lib docs, and it is the reason a third party can implement CGP without
//! adopting the rest of this repository's dependency tree. `serde_json` is an
//! *optional* dependency behind the `record-hash` feature; [`record`](crate::record)
//! compiles with no features at all. Reaching for `serde_json::Value` here would
//! mean one of two bad trades:
//!
//! - make `serde_json` non-optional, breaking the zero-dependency promise for
//!   every consumer that only wants the wire types; or
//! - feature-gate the field, so [`ContextRecord`](crate::ContextRecord)'s
//!   *shape* — and therefore what it round-trips — would depend on a Cargo
//!   feature. A type that silently drops wire members unless a feature is on is
//!   a worse version of the defect this exists to fix.
//!
//! `ExtensionValue` is the third option: about a hundred lines of `serde`
//! plumbing, no dependency, and the same shape in every build.
//!
//! # Fidelity
//!
//! The variants mirror the JSON data model exactly, including the distinction
//! `serde_json::Value` draws between an unsigned integer, a signed integer, and
//! a float — because that distinction is what decides whether `7` round-trips as
//! `7` or as `7.0`, and a digit that changes changes the digest. An
//! [`Object`](ExtensionValue::Object) is a [`BTreeMap`], so members are held in
//! sorted order; that is invisible to the record hash, which canonicalizes with
//! RFC 8785 (JCS) and sorts members anyway.

use std::collections::BTreeMap;
use std::fmt;

use serde::de::{IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::ser::{SerializeMap, SerializeSeq};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Any JSON value, modelled without a JSON dependency.
///
/// Used for the members of the wire the reference types leave open:
/// [`ContextRecord::extensions`](crate::ContextRecord::extensions) and
/// [`ContextRecord::extra`](crate::ContextRecord::extra).
#[derive(Debug, Clone, PartialEq)]
pub enum ExtensionValue {
    /// JSON `null`.
    Null,
    /// JSON `true` / `false`.
    Bool(bool),
    /// A non-negative integer that fits in a `u64`.
    UnsignedInteger(u64),
    /// A negative integer that fits in an `i64`.
    SignedInteger(i64),
    /// A number that is not an integer, or one outside the integer range.
    Float(f64),
    /// A JSON string.
    String(String),
    /// A JSON array.
    Array(Vec<ExtensionValue>),
    /// A JSON object. Sorted by member name; the record hash sorts anyway.
    Object(BTreeMap<String, ExtensionValue>),
}

impl ExtensionValue {
    /// Whether this is JSON `null`.
    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    /// The string, if this is a JSON string.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(text) => Some(text),
            _ => None,
        }
    }

    /// The boolean, if this is a JSON boolean.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(flag) => Some(*flag),
            _ => None,
        }
    }

    /// The value as an `f64`, if this is any JSON number. Lossy for integers
    /// beyond 2^53, which is why the integer variants exist separately.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::UnsignedInteger(value) => Some(*value as f64),
            Self::SignedInteger(value) => Some(*value as f64),
            Self::Float(value) => Some(*value),
            _ => None,
        }
    }

    /// The elements, if this is a JSON array.
    pub fn as_array(&self) -> Option<&[ExtensionValue]> {
        match self {
            Self::Array(items) => Some(items),
            _ => None,
        }
    }

    /// The members, if this is a JSON object.
    pub fn as_object(&self) -> Option<&BTreeMap<String, ExtensionValue>> {
        match self {
            Self::Object(members) => Some(members),
            _ => None,
        }
    }
}

impl From<&str> for ExtensionValue {
    fn from(text: &str) -> Self {
        Self::String(text.to_string())
    }
}

impl From<String> for ExtensionValue {
    fn from(text: String) -> Self {
        Self::String(text)
    }
}

impl Serialize for ExtensionValue {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            // `serialize_unit` rather than `serialize_none`: a JSON `null` that
            // is a *value* is not an absent member, and only the former
            // survives a `Serializer` that skips `None`.
            Self::Null => serializer.serialize_unit(),
            Self::Bool(flag) => serializer.serialize_bool(*flag),
            Self::UnsignedInteger(value) => serializer.serialize_u64(*value),
            Self::SignedInteger(value) => serializer.serialize_i64(*value),
            Self::Float(value) => serializer.serialize_f64(*value),
            Self::String(text) => serializer.serialize_str(text),
            Self::Array(items) => {
                let mut seq = serializer.serialize_seq(Some(items.len()))?;
                for item in items {
                    seq.serialize_element(item)?;
                }
                seq.end()
            }
            Self::Object(members) => {
                let mut map = serializer.serialize_map(Some(members.len()))?;
                for (name, value) in members {
                    map.serialize_entry(name, value)?;
                }
                map.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for ExtensionValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(ExtensionValueVisitor)
    }
}

struct ExtensionValueVisitor;

impl<'de> Visitor<'de> for ExtensionValueVisitor {
    type Value = ExtensionValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("any JSON value")
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(ExtensionValue::Null)
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(ExtensionValue::Null)
    }

    fn visit_some<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_any(self)
    }

    fn visit_bool<E>(self, flag: bool) -> Result<Self::Value, E> {
        Ok(ExtensionValue::Bool(flag))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(ExtensionValue::UnsignedInteger(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        // A non-negative integer is held in the unsigned variant whichever
        // visitor method delivered it, so `1` compares equal to `1` no matter
        // which side of the wire it came from.
        Ok(match u64::try_from(value) {
            Ok(unsigned) => ExtensionValue::UnsignedInteger(unsigned),
            Err(_) => ExtensionValue::SignedInteger(value),
        })
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E> {
        Ok(ExtensionValue::Float(value))
    }

    fn visit_str<E>(self, text: &str) -> Result<Self::Value, E> {
        Ok(ExtensionValue::String(text.to_string()))
    }

    fn visit_string<E>(self, text: String) -> Result<Self::Value, E> {
        Ok(ExtensionValue::String(text))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let mut items = Vec::with_capacity(seq.size_hint().unwrap_or(0));
        while let Some(item) = seq.next_element()? {
            items.push(item);
        }
        Ok(ExtensionValue::Array(items))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut members = BTreeMap::new();
        while let Some(name) = map.next_key::<String>()? {
            members.insert(name, map.next_value()?);
        }
        Ok(ExtensionValue::Object(members))
    }
}

/// Read one map entry into an [`ExtensionValue`], or discard it.
///
/// Shared by the record's extension plumbing: a member the reference types
/// already model must still have its value consumed from the [`MapAccess`], and
/// consuming it as [`IgnoredAny`] is both cheaper and unconditionally
/// infallible compared with parsing a value that is about to be thrown away.
pub(crate) fn take_or_ignore<'de, A: MapAccess<'de>>(
    map: &mut A,
    keep: bool,
) -> Result<Option<ExtensionValue>, A::Error> {
    if keep {
        map.next_value().map(Some)
    } else {
        map.next_value::<IgnoredAny>().map(|_| None)
    }
}
