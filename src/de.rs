//! Lenient deserialisation helpers used by the generated types.
// Which helpers the generated code uses depends on the spec (unions, response structs).
#![allow(dead_code)]

use serde::de::{Deserialize, Deserializer, Error as _};
use serde_json::Value;

/// Reads `null` as the type's default, for response fields the schema says are non-null.
pub(crate) fn null_as_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

/// One variant of an untagged union, as seen by [`union`].
pub(crate) struct Variant<T> {
    /// JSON keys the variant's schema requires (empty for non-object variants).
    pub(crate) required: &'static [&'static str],
    /// `const` string fields that identify the variant, as `(key, value)`.
    pub(crate) consts: &'static [(&'static str, &'static str)],
    /// Decodes the value as this variant.
    pub(crate) decode: fn(Value) -> serde_json::Result<T>,
}

/// Deserialises an untagged union.
///
/// Response structs are lenient (every field has a default), so the first object variant would
/// accept any object. The variant is chosen in three passes instead, keeping the first that
/// decodes: (1) all required keys present and no `const` field contradicted, (2) no `const` field
/// contradicted, (3) any variant.
pub(crate) fn union<'de, D, T>(
    deserializer: D,
    name: &str,
    variants: &[Variant<T>],
) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    let object = value.as_object();
    let consts_hold = |v: &Variant<T>| {
        object.is_none_or(|o| {
            v.consts.iter().all(|(key, expected)| {
                o.get(*key)
                    .is_none_or(|actual| actual.is_null() || actual.as_str() == Some(expected))
            })
        })
    };
    let required_present = |v: &Variant<T>| {
        v.required.is_empty()
            || object.is_some_and(|o| {
                v.required
                    .iter()
                    .all(|key| o.get(*key).is_some_and(|x| !x.is_null()))
            })
    };
    let mut failed = vec![false; variants.len()];
    for pass in 0..3 {
        for (index, variant) in variants.iter().enumerate() {
            let eligible = match pass {
                0 => required_present(variant) && consts_hold(variant),
                1 => consts_hold(variant),
                _ => true,
            };
            if !eligible || failed[index] {
                continue;
            }
            match (variant.decode)(value.clone()) {
                Ok(decoded) => return Ok(decoded),
                Err(_) => failed[index] = true,
            }
        }
    }
    Err(D::Error::custom(format!(
        "data did not match any variant of {name}"
    )))
}
