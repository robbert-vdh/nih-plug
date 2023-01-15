use nih_plug::prelude::*;

#[derive(Params)]
struct FlatParams {
    #[id = "one"]
    pub one: BoolParam,

    #[id = "two"]
    pub two: FloatParam,

    #[id = "three"]
    pub three: IntParam,
}

impl Default for FlatParams {
    fn default() -> Self {
        FlatParams {
            one: BoolParam::new("one", true),
            two: FloatParam::new("two", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 }),
            three: IntParam::new("three", 0, IntRange::Linear { min: 0, max: 100 }),
        }
    }
}

#[derive(Params)]
struct GroupedParams {
    #[id = "one"]
    pub one: BoolParam,

    #[nested(group = "Some Group", id_prefix = "group1")]
    pub group1: FlatParams,

    #[id = "three"]
    pub three: IntParam,

    #[nested(group = "Another Group", id_prefix = "group2")]
    pub group2: FlatParams,
}

impl Default for GroupedParams {
    fn default() -> Self {
        GroupedParams {
            one: BoolParam::new("one", true),
            group1: FlatParams::default(),
            three: IntParam::new("three", 0, IntRange::Linear { min: 0, max: 100 }),
            group2: FlatParams::default(),
        }
    }
}

// This should result in the same `.param_map()` as `GroupedParams`
#[derive(Default, Params)]
struct PlainNestedParams {
    #[nested]
    pub inner: GroupedParams,
}

#[derive(Default, Params)]
struct GroupedGroupedParams {
    #[nested(group = "Top-level group")]
    pub one: GroupedParams,
}

#[derive(Params)]
struct NestedParams {
    #[id = "one"]
    pub one: BoolParam,

    #[nested(id_prefix = "two")]
    pub two: FlatParams,

    #[id = "three"]
    pub three: IntParam,
}

impl Default for NestedParams {
    fn default() -> Self {
        NestedParams {
            one: BoolParam::new("one", true),
            two: FlatParams::default(),
            three: IntParam::new("three", 0, IntRange::Linear { min: 0, max: 100 }),
        }
    }
}

#[derive(Params)]
struct NestedArrayParams {
    #[id = "one"]
    pub one: BoolParam,

    #[nested(array, group = "Nested Params")]
    pub lots_of_twos: [FlatParams; 3],

    #[id = "three"]
    pub three: IntParam,
}

impl Default for NestedArrayParams {
    fn default() -> Self {
        NestedArrayParams {
            one: BoolParam::new("one", true),
            lots_of_twos: [
                FlatParams::default(),
                FlatParams::default(),
                FlatParams::default(),
            ],
            three: IntParam::new("three", 0, IntRange::Linear { min: 0, max: 100 }),
        }
    }
}

mod param_order {
    use super::*;

    #[test]
    fn flat() {
        let p = FlatParams::default();

        // Parameters must have the same order as they are defined in
        let param_ids: Vec<String> = p.param_map().into_iter().map(|(id, _, _)| id).collect();
        assert_eq!(param_ids, ["one", "two", "three"]);
    }

    #[test]
    fn grouped() {
        let p = GroupedParams::default();

        let param_ids: Vec<String> = p.param_map().into_iter().map(|(id, _, _)| id).collect();
        assert_eq!(
            param_ids,
            [
                "one",
                "group1_one",
                "group1_two",
                "group1_three",
                "three",
                "group2_one",
                "group2_two",
                "group2_three",
            ]
        );
    }

    #[test]
    fn plain_nested() {
        let plain_nested = PlainNestedParams::default();
        let grouped = GroupedParams::default();

        let plain_nested_ids_groups: Vec<(String, String)> = plain_nested
            .param_map()
            .into_iter()
            .map(|(id, _, group)| (id, group))
            .collect();
        let grouped_param_ids_groups: Vec<(String, String)> = grouped
            .param_map()
            .into_iter()
            .map(|(id, _, group)| (id, group))
            .collect();

        assert_eq!(plain_nested_ids_groups, grouped_param_ids_groups);
    }

    #[test]
    fn grouped_groups() {
        let p = GroupedGroupedParams::default();

        // These don't have ID prefixes, so the IDs should be the same as in `groups()`
        let param_ids: Vec<String> = p.param_map().into_iter().map(|(id, _, _)| id).collect();
        assert_eq!(
            param_ids,
            [
                "one",
                "group1_one",
                "group1_two",
                "group1_three",
                "three",
                "group2_one",
                "group2_two",
                "group2_three",
            ]
        );
    }

    #[test]
    fn nested() {
        let p = NestedParams::default();

        // Parameters must have the same order as they are defined in. The position of nested
        // parameters which are not grouped explicitly is preserved.
        let param_ids: Vec<String> = p.param_map().into_iter().map(|(id, _, _)| id).collect();

        assert_eq!(
            param_ids,
            ["one", "two_one", "two_two", "two_three", "three"]
        );
    }

    #[test]
    fn nested_array() {
        let p = NestedArrayParams::default();

        // Arrays of nested parameter structs have generated IDs
        let param_ids: Vec<String> = p.param_map().into_iter().map(|(id, _, _)| id).collect();
        assert_eq!(
            param_ids,
            [
                "one", "one_1", "two_1", "three_1", "one_2", "two_2", "three_2", "one_3", "two_3",
                "three_3", "three"
            ]
        );
    }
}

mod param_groups {
    use super::*;

    #[test]
    fn flat() {
        let p = FlatParams::default();

        // These groups should be all empty
        let param_groups: Vec<String> = p
            .param_map()
            .into_iter()
            .map(|(_, _, group)| group)
            .collect();
        assert_eq!(param_groups, ["", "", ""]);
    }

    #[test]
    fn grouped() {
        let p = GroupedParams::default();

        let param_groups: Vec<String> = p
            .param_map()
            .into_iter()
            .map(|(_, _, group)| group)
            .collect();
        assert_eq!(
            param_groups,
            [
                "",
                "Some Group",
                "Some Group",
                "Some Group",
                "",
                "Another Group",
                "Another Group",
                "Another Group",
            ]
        );
    }

    #[test]
    fn grouped_groups() {
        let p = GroupedGroupedParams::default();

        let param_groups: Vec<String> = p
            .param_map()
            .into_iter()
            .map(|(_, _, group)| group)
            .collect();
        assert_eq!(
            param_groups,
            [
                "Top-level group",
                "Top-level group/Some Group",
                "Top-level group/Some Group",
                "Top-level group/Some Group",
                "Top-level group",
                "Top-level group/Another Group",
                "Top-level group/Another Group",
                "Top-level group/Another Group",
            ]
        );
    }

    #[test]
    fn nested() {
        let p = NestedParams::default();

        // The nested structs here don't have any groups assigned to them
        let param_groups: Vec<String> = p
            .param_map()
            .into_iter()
            .map(|(_, _, group)| group)
            .collect();
        assert_eq!(param_groups, ["", "", "", "", ""]);
    }

    #[test]
    fn nested_array() {
        let p = NestedArrayParams::default();

        // The groups get a numeric suffix here
        let param_groups: Vec<String> = p
            .param_map()
            .into_iter()
            .map(|(_, _, group)| group)
            .collect();
        assert_eq!(
            param_groups,
            [
                "",
                "Nested Params 1",
                "Nested Params 1",
                "Nested Params 1",
                "Nested Params 2",
                "Nested Params 2",
                "Nested Params 2",
                "Nested Params 3",
                "Nested Params 3",
                "Nested Params 3",
                ""
            ]
        );
    }
}
