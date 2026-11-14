//! A transport's settings as guatiao values: the schema a host checks a configuration against, the value a
//! configuration is, and the settings that differ from their default.
//!
//! The settings are any type that derives `coqui_builder::Describe`; nothing here needs the transport's own
//! crate to know guatiao. A transport crate keeps its settings types as they are, and the crate that registers
//! it names them here.

use alloc::string::ToString;
use alloc::vec::Vec;

use coqui_builder::{Describe, Inspect};
use guatiao::Value;
use guatiao::value::ValueError;
use guatiao_builder::FromError;

pub use guatiao_builder::Described;

use crate::driver::Driver;
use crate::kind::NonDefault;

/// The configuration type of a [`Driver`]'s provider: its settings, read from and written as a guatiao value.
pub type Settings<D> = Described<<D as Driver>::Settings>;

/// The JSON Schema of `S`, as a guatiao value: what a host checks a configuration against, and shows a person.
///
/// # Errors
///
/// When `S` is not a group of settings.
pub fn schema<S: Describe>() -> Result<Value, ValueError> {
    guatiao_builder::schema(&S::description())
}

/// `settings` as a guatiao value, secrets included: it is the configuration.
///
/// # Errors
///
/// When a setting cannot be written as a value.
pub fn encode<S: Describe + Inspect>(settings: &S) -> Result<Value, ValueError> {
    guatiao_builder::to_value(settings)
}

/// The settings a guatiao value holds, with every one it leaves out at its default.
///
/// # Errors
///
/// A key `S` does not have where it refuses one, a text a setting does not read, or a setting required and
/// missing. The error never quotes the value.
pub fn decode<S: Describe>(value: &Value) -> Result<S, FromError> {
    guatiao_builder::from_value(value)
}

/// Each setting of `settings` that differs from its default, by key path, generated from the description.
#[must_use]
pub fn non_defaults<S: Describe + Inspect>(settings: &S) -> Vec<NonDefault> {
    coqui_builder::non_defaults(settings)
        .into_iter()
        .map(|difference| {
            NonDefault::new(
                difference.setting,
                difference.value.to_string(),
                difference.default.to_string(),
            )
        })
        .collect()
}
