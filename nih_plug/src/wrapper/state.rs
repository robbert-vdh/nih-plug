//! Utilities for saving a [`crate::plugin::Plugin`]'s state. The actual state object is also exposed
//! to plugins through the [`GuiContext`][crate::prelude::GuiContext].

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use crate::params::ParamMut;
use crate::prelude::{BufferConfig, Param, ParamPtr, Params, Plugin};

// These state objects are also exposed directly to the plugin so it can do its own internal preset
// management

/// A plain, unnormalized value for a parameter.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParamValue {
    F32(f32),
    I32(i32),
    Bool(bool),
    /// Only used for enum parameters that have the `#[id = "..."]` attribute set.
    String(String),
}

/// A plugin's state so it can be restored at a later point. This object can be serialized and
/// deserialized using serde.
///
/// The fields are stored as `BTreeMap`s so the order in the serialized file is consistent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginState {
    /// The plugin version this state was saved with. Right now this is not used, but later versions
    /// of NIH-plug may allow you to modify the plugin state object directly before it is loaded to
    /// allow migrating plugin states between breaking parameter changes.
    ///
    /// # Notes
    ///
    /// If the saved state is very old, then this field may be empty.
    #[serde(default)]
    pub version: String,

    /// The plugin's parameter values. These are stored unnormalized. This means the old values will
    /// be recalled when when the parameter's range gets increased. Doing so may still mess with
    /// parameter automation though, depending on how the host implements that.
    pub params: BTreeMap<String, ParamValue>,
    /// Arbitrary fields that should be persisted together with the plugin's parameters. Any field
    /// on the [`Params`][crate::params::Params] struct that's annotated with `#[persist =
    /// "stable_name"]` will be persisted this way.
    ///
    /// The individual fields are also serialized as JSON so they can safely be restored
    /// independently of the other fields.
    pub fields: BTreeMap<String, String>,
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
) -> impl Fn(&str) -> Option<ParamPtr> + 'a {
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
pub(crate) unsafe fn serialize_object<'a, P: Plugin>(
    plugin_params: Arc<dyn Params>,
    params_iter: impl IntoIterator<Item = (&'a String, ParamPtr)>,
) -> PluginState {
    // We'll serialize parameter values as a simple `string_param_id: display_value` map.
    // NOTE: If the plugin is being modulated (and the plugin is a CLAP plugin in Bitwig Studio),
    //       then this should save the values without any modulation applied to it
    let params: BTreeMap<_, _> = params_iter
        .into_iter()
        .map(|(param_id_str, param_ptr)| match param_ptr {
            ParamPtr::FloatParam(p) => (
                param_id_str.clone(),
                ParamValue::F32((*p).unmodulated_plain_value()),
            ),
            ParamPtr::IntParam(p) => (
                param_id_str.clone(),
                ParamValue::I32((*p).unmodulated_plain_value()),
            ),
            ParamPtr::BoolParam(p) => (
                param_id_str.clone(),
                ParamValue::Bool((*p).unmodulated_plain_value()),
            ),
            ParamPtr::EnumParam(p) => (
                // Enums are either serialized based on the active variant's index (which may not be
                // the same as the discriminator), or a custom set stable string ID. The latter
                // allows the variants to be reordered.
                param_id_str.clone(),
                match (*p).unmodulated_plain_id() {
                    Some(id) => ParamValue::String(id.to_owned()),
                    None => ParamValue::I32((*p).unmodulated_plain_value()),
                },
            ),
        })
        .collect();

    // The plugin can also persist arbitrary fields alongside its parameters. This is useful for
    // storing things like sample data.
    let fields = plugin_params.serialize_fields();

    PluginState {
        version: String::from(P::VERSION),
        params,
        fields,
    }
}

/// Serialize a plugin's state to a vector containing JSON data. This can (and should) be shared
/// across plugin formats. If the `zstd` feature is enabled, then the state will be compressed using
/// Zstandard.
pub(crate) unsafe fn serialize_json<'a, P: Plugin>(
    plugin_params: Arc<dyn Params>,
    params_iter: impl IntoIterator<Item = (&'a String, ParamPtr)>,
) -> Result<Vec<u8>> {
    let plugin_state = serialize_object::<P>(plugin_params, params_iter);
    let json = serde_json::to_vec(&plugin_state).context("Could not format as JSON")?;

    #[cfg(feature = "zstd")]
    {
        let compressed = zstd::encode_all(json.as_slice(), zstd::DEFAULT_COMPRESSION_LEVEL)
            .context("Could not compress state")?;

        let state_bytes = json.len();
        let compressed_state_bytes = compressed.len();
        let compression_ratio = compressed_state_bytes as f32 / state_bytes as f32 * 100.0;
        nih_trace!(
            "Compressed {state_bytes} bytes of state to {compressed_state_bytes} bytes \
             ({compression_ratio:.1}% compression ratio)"
        );

        Ok(compressed)
    }
    #[cfg(not(feature = "zstd"))]
    {
        Ok(json)
    }
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
///
/// The [`Plugin`] argument is used to call [`Plugin::filter_state()`] just before loading the
/// state.
pub(crate) unsafe fn deserialize_object<P: Plugin>(
    state: &mut PluginState,
    plugin_params: Arc<dyn Params>,
    params_getter: impl Fn(&str) -> Option<ParamPtr>,
    current_buffer_config: Option<&BufferConfig>,
) -> bool {
    // This lets the plugin perform migrations on old state if needed
    P::filter_state(state);

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
            (ParamPtr::FloatParam(p), ParamValue::F32(v)) => {
                (*p).set_plain_value(*v);
            }
            (ParamPtr::IntParam(p), ParamValue::I32(v)) => {
                (*p).set_plain_value(*v);
            }
            (ParamPtr::BoolParam(p), ParamValue::Bool(v)) => {
                (*p).set_plain_value(*v);
            }
            // Enums are either serialized based on the active variant's index (which may not be the
            // same as the discriminator), or a custom set stable string ID. The latter allows the
            // variants to be reordered.
            (ParamPtr::EnumParam(p), ParamValue::I32(variant_idx)) => {
                (*p).set_plain_value(*variant_idx);
            }
            (ParamPtr::EnumParam(p), ParamValue::String(id)) => {
                let deserialized_enum = (*p).set_from_id(id);
                nih_debug_assert!(
                    deserialized_enum,
                    "Unknown ID {:?} for enum parameter \"{}\"",
                    id,
                    param_id_str,
                );
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

/// Deserialize a plugin's state from a vector containing (compressed) JSON data. Doesn't load the
/// plugin state since doing so should be accompanied by calls to `Plugin::init()` and
/// `Plugin::reset()`, and this way all of that behavior can be encapsulated so it can be reused in
/// multiple places. The returned state object can be passed to [`deserialize_object()`].
pub(crate) unsafe fn deserialize_json(state: &[u8]) -> Option<PluginState> {
    #[cfg(feature = "zstd")]
    let result: Option<PluginState> = match zstd::decode_all(state) {
        Ok(decompressed) => match serde_json::from_slice(decompressed.as_slice()) {
            Ok(s) => {
                let state_bytes = decompressed.len();
                let compressed_state_bytes = state.len();
                let compression_ratio = compressed_state_bytes as f32 / state_bytes as f32 * 100.0;
                nih_trace!(
                    "Inflated {compressed_state_bytes} bytes of state to {state_bytes} bytes \
                     ({compression_ratio:.1}% compression ratio)"
                );

                Some(s)
            }
            Err(err) => {
                nih_debug_assert_failure!("Error while deserializing state: {}", err);
                None
            }
        },
        // Uncompressed state files can still be loaded after enabling this feature to prevent
        // breaking existing plugin instances
        Err(zstd_err) => match serde_json::from_slice(state) {
            Ok(s) => {
                nih_trace!("Older uncompressed state found");
                Some(s)
            }
            Err(json_err) => {
                nih_debug_assert_failure!(
                    "Error while deserializing state as either compressed or uncompressed state: \
                     {}, {}",
                    zstd_err,
                    json_err
                );
                None
            }
        },
    };

    #[cfg(not(feature = "zstd"))]
    let result: Option<PluginState> = match serde_json::from_slice(state) {
        Ok(s) => Some(s),
        Err(err) => {
            nih_debug_assert_failure!("Error while deserializing state: {}", err);
            None
        }
    };

    result
}
