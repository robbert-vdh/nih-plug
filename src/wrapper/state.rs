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

//! Utilities for saving a [Plugin]'s state.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A plain, unnormalized value for a parameter.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ParamValue {
    F32(f32),
    I32(i32),
}

/// A plugin's state so it can be restored at a later point.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct State {
    /// The plugin's parameter values. These are stored unnormalized. This mean sthe old values will
    /// be recalled when when the parameter's range gets increased. Doing so may still mess with
    /// parmaeter automation though, depending on how the host impelments that.
    pub params: HashMap<String, ParamValue>,
}
