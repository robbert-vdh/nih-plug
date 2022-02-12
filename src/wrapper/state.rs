//! Utilities for saving a [crate::plugin::Plugin]'s state.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A plain, unnormalized value for a parameter.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ParamValue {
    F32(f32),
    I32(i32),
    Bool(bool),
}

/// A plugin's state so it can be restored at a later point.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct State {
    /// The plugin's parameter values. These are stored unnormalized. This mean sthe old values will
    /// be recalled when when the parameter's range gets increased. Doing so may still mess with
    /// parmaeter automation though, depending on how the host impelments that.
    pub params: HashMap<String, ParamValue>,
    /// Arbitrary fields that should be persisted together with the plugin's parameters. Any field
    /// on the [crate::param::internals::Params] struct that's annotated with `#[persist =
    /// "stable_name"]` will be persisted this way.
    ///
    /// The individual fields are also serialized as JSON so they can safely be restored
    /// independently of the other fields.
    pub fields: HashMap<String, String>,
}
