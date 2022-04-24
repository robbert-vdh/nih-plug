//! Utilities for saving a [crate::plugin::Plugin]'s state. The actual state object is also exposed
//! to plugins through the [`GuiContext`][crate::prelude::GuiContext].

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

use crate::param::internals::{ParamPtr, Params};
use crate::param::Param;
use crate::plugin::BufferConfig;

// These state objects are also exposed directly to the plugin so it can do its own internal preset
// management

/// A plain, unnormalized value for a parameter.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParamValue {
    F32(f32),
    I32(i32),
    Bool(bool),
}

/// A plugin's state so it can be restored at a later point. This object can be serialized and
/// deserialized using serde.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginState {
    /// The plugin's parameter values. These are stored unnormalized. This mean sthe old values will
    /// be recalled when when the parameter's range gets increased. Doing so may still mess with
    /// parameter automation though, depending on how the host impelments that.
    pub params: HashMap<String, ParamValue>,
    /// Arbitrary fields that should be persisted together with the plugin's parameters. Any field
    /// on the [`Params`][crate::param::internals::Params] struct that's annotated with `#[persist =
    /// "stable_name"]` will be persisted this way.
    ///
    /// The individual fields are also serialized as JSON so they can safely be restored
    /// independently of the other fields.
    pub fields: HashMap<String, String>,
}

/// Create a parameters iterator from the hashtables stored in the plugin wrappers. This avoids
/// having to call `.param_map()` again, which may include expensive user written code.
pub(crate) fn make_params_iter<'a>(
    param_by_hash: &'a HashMap<u32, ParamPtr>,
    param_id_to_hash: &'a HashMap<String, u32>,
) -> impl IntoIterator<Item = (&'a String, ParamPtr)> {
    param_id_to_hash.iter().filter_map(|(param_id_str, hash)| {
        let param_ptr = param_by_hash.get(hash)?;
        Some((param_id_str, *param_ptr))
    })
}

/// Create a getter function that gets a parameter from the hashtables stored in the plugin by
/// string ID.
pub(crate) fn make_params_getter<'a>(
    param_by_hash: &'a HashMap<u32, ParamPtr>,
    param_id_to_hash: &'a HashMap<String, u32>,
) -> impl for<'b> Fn(&'b str) -> Option<ParamPtr> + 'a {
    |param_id_str| {
        param_id_to_hash
            .get(param_id_str)
            .and_then(|hash| param_by_hash.get(hash))
            .copied()
    }
}

/// Serialize a plugin's state to a state object. This is separate from [`serialize_json()`] to
/// allow passing the raw object directly to the plugin. The parameters are not pulled directly from
/// `plugin_params` by default to avoid unnecessary allocations in the `.param_map()` method, as the
/// plugin wrappers will already have a list of parameters handy. See [`make_params_iter()`].
pub(crate) unsafe fn serialize_object<'a>(
    plugin_params: Arc<dyn Params>,
    params_iter: impl IntoIterator<Item = (&'a String, ParamPtr)>,
) -> PluginState {
    // We'll serialize parameter values as a simple `string_param_id: display_value` map.
    let params: HashMap<_, _> = params_iter
        .into_iter()
        .map(|(param_id_str, param_ptr)| match param_ptr {
            ParamPtr::FloatParam(p) => (param_id_str.clone(), ParamValue::F32((*p).plain_value())),
            ParamPtr::IntParam(p) => (param_id_str.clone(), ParamValue::I32((*p).plain_value())),
            ParamPtr::BoolParam(p) => (param_id_str.clone(), ParamValue::Bool((*p).plain_value())),
            ParamPtr::EnumParam(p) => (
                // Enums are serialized based on the active variant's index (which may not be
                // the same as the discriminator)
                param_id_str.clone(),
                ParamValue::I32((*p).plain_value()),
            ),
        })
        .collect();

    // The plugin can also persist arbitrary fields alongside its parameters. This is useful for
    // storing things like sample data.
    let fields = plugin_params.serialize_fields();

    PluginState { params, fields }
}

/// Serialize a plugin's state to a vector containing JSON data. This can (and should) be shared
/// across plugin formats.
pub(crate) unsafe fn serialize_json<'a>(
    plugin_params: Arc<dyn Params>,
    params_iter: impl IntoIterator<Item = (&'a String, ParamPtr)>,
) -> serde_json::Result<Vec<u8>> {
    let plugin_state = serialize_object(plugin_params, params_iter);
    serde_json::to_vec(&plugin_state)
}

/// Deserialize a plugin's state from a [`PluginState`] object. This is used to allow the plugin to
/// do its own internal preset management. Returns `false` and logs an error if the state could not
/// be deserialized.
///
/// This uses a parameter getter function to avoid having to rebuild the parameter map, which may
/// include expensive user written code. See [`make_params_getter()`].
///
/// Make sure to reinitialize plugin after deserializing the state so it can react to the new
/// parameter values. The smoothers have already been reset by this function.
pub(crate) unsafe fn deserialize_object(
    state: &PluginState,
    plugin_params: Arc<dyn Params>,
    params_getter: impl for<'a> Fn(&'a str) -> Option<ParamPtr>,
    current_buffer_config: Option<&BufferConfig>,
) -> bool {
    let sample_rate = current_buffer_config.map(|c| c.sample_rate);
    for (param_id_str, param_value) in &state.params {
        let param_ptr = match params_getter(param_id_str.as_str()) {
            Some(ptr) => ptr,
            None => {
                nih_debug_assert_failure!("Unknown parameter: {}", param_id_str);
                continue;
            }
        };

        match (param_ptr, param_value) {
            (ParamPtr::FloatParam(p), ParamValue::F32(v)) => (*p).set_plain_value(*v),
            (ParamPtr::IntParam(p), ParamValue::I32(v)) => (*p).set_plain_value(*v),
            (ParamPtr::BoolParam(p), ParamValue::Bool(v)) => (*p).set_plain_value(*v),
            // Enums are serialized based on the active variant's index (which may not be the same
            // as the discriminator)
            (ParamPtr::EnumParam(p), ParamValue::I32(variant_idx)) => {
                (*p).set_plain_value(*variant_idx)
            }
            (param_ptr, param_value) => {
                nih_debug_assert_failure!(
                    "Invalid serialized value {:?} for parameter \"{}\" ({:?})",
                    param_value,
                    param_id_str,
                    param_ptr,
                );
            }
        }

        // Make sure everything starts out in sync
        if let Some(sample_rate) = sample_rate {
            param_ptr.update_smoother(sample_rate, true);
        }
    }

    // The plugin can also persist arbitrary fields alongside its parameters. This is useful for
    // storing things like sample data.
    plugin_params.deserialize_fields(&state.fields);

    true
}

/// Deserialize a plugin's state from a vector containing JSON data. This can (and should) be shared
/// across plugin formats. Returns `false` and logs an error if the state could not be deserialized.
///
/// Make sure to reinitialize plugin after deserializing the state so it can react to the new
/// parameter values. The smoothers have already been reset by this function.
pub(crate) unsafe fn deserialize_json(
    state: &[u8],
    plugin_params: Arc<dyn Params>,
    params_getter: impl for<'a> Fn(&'a str) -> Option<ParamPtr>,
    current_buffer_config: Option<&BufferConfig>,
) -> bool {
    let state: PluginState = match serde_json::from_slice(state) {
        Ok(s) => s,
        Err(err) => {
            nih_debug_assert_failure!("Error while deserializing state: {}", err);
            return false;
        }
    };

    deserialize_object(&state, plugin_params, params_getter, current_buffer_config)
}
