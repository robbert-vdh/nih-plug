//! Parameter hierarchies in VST3 requires you to define units, which are linearly indexed logical
//! units that have a name, a parent, and then a whole bunch of other data like note numbers and
//! MIDI program state. We'll need to implement some of that to convert our list of slash-separated
//! parameter group paths to units.
//!
//! <https://steinbergmedia.github.io/vst3_doc/vstinterfaces/classSteinberg_1_1Vst_1_1IUnitInfo.html>

use std::collections::{HashMap, HashSet};

use vst3::Steinberg::Vst::kRootUnitId;

/// Transforms a map containing parameter hashes and slash-separated paths to an array of VST3 units
/// and a mapping for each parameter hash to a unit (or to `None` if they belong to the root unit).
/// This is conceptually similar to a prefix tree/trie, but since we don't need any of the lookup
/// properties of those data structures we can brute force it by converting all paths to a set,
/// converting the rightmost component of each unique path to a unit, and then assigning the parent
/// units accordingly.
///
/// <https://steinbergmedia.github.io/vst3_doc/vstinterfaces/classSteinberg_1_1Vst_1_1IUnitInfo.html>
#[derive(Debug)]
pub struct ParamUnits {
    /// The unique units, with flat indices.
    units: Vec<ParamUnit>,
    /// The index of the unit a parameter belongs to, or `None` if it belongs to the root unit.
    ///
    /// NOTE: The returned unit ID is actually one higher than this index because VST3 uses 0 as a
    ///       no-unit value
    unit_id_by_hash: HashMap<u32, i32>,
}

/// A VST3 'unit'. Repurposed for a bunch of things, but we only care about parameter hierarchies.
///
/// <https://steinbergmedia.github.io/vst3_doc/vstinterfaces/structSteinberg_1_1Vst_1_1UnitInfo.html>
#[derive(Debug)]
pub struct ParamUnit {
    /// The name of the unit, without any of the proceeding components.
    pub name: String,
    /// The ID of the parent unit, or `kRootUnitId`/0 if the parent would be the root node. Because
    /// 0 is reserved, these IDs are one higher than the actual index in `ParamUnits::units`.
    pub parent_id: i32,
}

impl ParamUnits {
    /// Construct a [`ParamUnits`] object from an iterator over pairs of `(param_hash, param_group)`
    /// where `param_hash` is the integer hash used to represent a parameter in the VST3 wrapper and
    /// `param_group` is a slash delimited path.
    ///
    /// Returns an error if the iterator contains nested groups without a matching parent.
    pub fn from_param_groups<'a, I>(groups: I) -> Result<Self, &'static str>
    where
        I: Iterator<Item = (u32, &'a str)> + Clone,
    {
        // First we'll build a unit for each unique parameter group. We need to be careful here to
        // expand `foo/bar/baz` into `foo/bar/baz`, `foo/bar` and `foo`, in case the parent groups
        // don't contain any parameters and thus aren't present in `groups`.
        let unique_group_names: HashSet<String> = groups
            .clone()
            .filter_map(|(_, group_name)| {
                // The root should not be included here since that's a special case in VST3
                if !group_name.is_empty() {
                    Some(group_name)
                } else {
                    None
                }
            })
            .flat_map(|group_name| {
                // This is the expansion mentioned above
                let mut expanded_group = String::new();
                let mut expanded_groups = Vec::new();
                for component in group_name.split('/') {
                    if !expanded_group.is_empty() {
                        expanded_group.push('/');
                    }
                    expanded_group.push_str(component);
                    expanded_groups.push(expanded_group.clone());
                }

                expanded_groups
            })
            .collect();
        let mut groups_units: Vec<(&str, ParamUnit)> = unique_group_names
            .iter()
            .map(|group_name| {
                (
                    group_name.as_str(),
                    ParamUnit {
                        name: match group_name.rfind('/') {
                            Some(sep_pos) => group_name[sep_pos + 1..].to_string(),
                            None => group_name.to_string(),
                        },
                        parent_id: kRootUnitId,
                    },
                )
            })
            .collect();

        // Then we need to assign the correct parent IDs. We'll also sort the units so the order is
        // stable.
        groups_units.sort_by(|(group_name_l, _), (group_name_r, _)| group_name_l.cmp(group_name_r));

        // We need to be able to map group names to unit IDs
        // NOTE: Now it starts getting complicated because VST3 units are one indexed, so the unit
        //       IDs are one higher than the index in our vector
        let vst3_unit_id_by_group_name: HashMap<&str, i32> = groups_units
            .iter()
            .enumerate()
            // Note the +1 here
            .map(|(unit_id, (group_name, _))| (*group_name, unit_id as i32 + 1))
            .collect();

        for (group_name, unit) in &mut groups_units {
            // If the group name does not contain any slashes then the unit's parent should stay at
            // the root unit
            if let Some(sep_pos) = group_name.rfind('/') {
                let parent_group_name = &group_name[..sep_pos];
                let parent_unit_id = *vst3_unit_id_by_group_name
                    .get(parent_group_name)
                    .ok_or("Missing parent group")?;
                unit.parent_id = parent_unit_id;
            }
        }

        let unit_id_by_hash: HashMap<u32, i32> = groups
            .map(|(param_hash, group_name)| {
                if group_name.is_empty() {
                    (param_hash, kRootUnitId)
                } else {
                    (param_hash, vst3_unit_id_by_group_name[group_name])
                }
            })
            .collect();
        let units: Vec<ParamUnit> = groups_units.into_iter().map(|(_, unit)| unit).collect();

        Ok(Self {
            units,
            unit_id_by_hash,
        })
    }

    /// Get the number of units.
    pub fn len(&self) -> usize {
        self.units.len()
    }

    /// Get the unit ID and the unit's information for a unit with the given 0-indexed index (to
    /// make everything more confusing).
    pub fn info(&self, index: usize) -> Option<(i32, &ParamUnit)> {
        let info = self.units.get(index)?;

        // NOTE: The VST3 unit indices are off by one because 0 is reserved fro the root unit
        Some((index as i32 + 1, info))
    }

    /// Get the ID of the unit the parameter belongs to. `kRootUnitId`/0 indicates the root unit.
    pub fn get_vst3_unit_id(&self, param_hash: u32) -> Option<i32> {
        self.unit_id_by_hash.get(&param_hash).copied()
    }
}
