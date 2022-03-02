//! Utilities for saving a [crate::plugin::Plugin]'s state.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::param::internals::ParamPtr;
use crate::param::Param;
use crate::Params;

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

/// Serialize a plugin's state to a vector containing JSON data. This can (and should) be shared
/// across plugin formats.
pub(crate) unsafe fn serialize(
    plugin_params: Pin<&dyn Params>,
    param_by_hash: &HashMap<u32, ParamPtr>,
    param_id_to_hash: &HashMap<&'static str, u32>,
    bypass_param_id: &str,
    bypass_state: &AtomicBool,
) -> serde_json::Result<Vec<u8>> {
    // We'll serialize parmaeter values as a simple `string_param_id: display_value` map.
    let mut params: HashMap<_, _> = param_id_to_hash
        .iter()
        .filter_map(|(param_id_str, hash)| {
            let param_ptr = param_by_hash.get(hash)?;
            Some((param_id_str, param_ptr))
        })
        .map(|(&param_id_str, &param_ptr)| match param_ptr {
            ParamPtr::FloatParam(p) => (
                param_id_str.to_string(),
                ParamValue::F32((*p).plain_value()),
            ),
            ParamPtr::IntParam(p) => (
                param_id_str.to_string(),
                ParamValue::I32((*p).plain_value()),
            ),
            ParamPtr::BoolParam(p) => (
                param_id_str.to_string(),
                ParamValue::Bool((*p).plain_value()),
            ),
            ParamPtr::EnumParam(p) => (
                // Enums are serialized based on the active variant's index (which may not be
                // the same as the discriminator)
                param_id_str.to_string(),
                ParamValue::I32((*p).plain_value()),
            ),
        })
        .collect();

    // Don't forget about the bypass parameter
    params.insert(
        bypass_param_id.to_string(),
        ParamValue::Bool(bypass_state.load(Ordering::SeqCst)),
    );

    // The plugin can also persist arbitrary fields alongside its parameters. This is useful for
    // storing things like sample data.
    let fields = plugin_params.serialize_fields();

    let plugin_state = State { params, fields };
    serde_json::to_vec(&plugin_state)
}
