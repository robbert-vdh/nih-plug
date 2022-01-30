// nih-plug: plugins, but rewritten in Rust
// Copyright (C) 2022 Robbert van der Helm
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

//! Utilities for saving a [crate::plugin::Plugin]'s state.

use serde::de::Error;
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
    /// on the [Params] struct that's annotated with `#[persist = "stable_name"]` will be persisted
    /// this way.
    ///
    /// The individual JSON-serialized fields are encoded as base64 strings so they don't take up as
    /// much space in the preset. Storing them as a plain JSON string would have also been possible,
    /// but that can get messy with escaping since those will likely also contain double quotes.
    #[serde(serialize_with = "encode_fields")]
    #[serde(deserialize_with = "decode_fields")]
    pub fields: HashMap<String, Vec<u8>>,
}

fn encode_fields<S>(bytes: &HashMap<String, Vec<u8>>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.collect_map(
        bytes
            .into_iter()
            .map(|(id, json)| (id, base64::encode(json))),
    )
}

fn decode_fields<'de, D>(deserializer: D) -> Result<HashMap<String, Vec<u8>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let base64_map: HashMap<String, String> = HashMap::deserialize(deserializer)?;
    let decoded_map: Result<HashMap<String, Vec<u8>>, D::Error> = base64_map
        .into_iter()
        .map(|(id, base64)| {
            base64::decode(base64)
                .map(|decoded| (id, decoded))
                .map_err(|err| D::Error::custom(format!("base64 decode failed: {}", err)))
        })
        .collect();

    Ok(decoded_map?)
}
