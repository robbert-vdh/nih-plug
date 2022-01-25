use nih_plug::params::{FloatParam, IntParam, Params, Range};
use nih_plug_derive::Params;

#[derive(Params)]
struct FooParams {
    #[id("pain")]
    pub pain: FloatParam,
    #[id("pain_stages")]
    pub pain_stages: IntParam,

    #[id("identifiers_are_stable")]
    pub but_field_names_can_change: FloatParam,
}

impl Default for FooParams {
    fn default() -> Self {
        Self {
            pain: FloatParam {
                value: 69.0,
                range: Range::Linear {
                    min: -420.0,
                    max: 420.0,
                },
                name: "Pain",
                unit: " Hertz",
                value_to_string: None,
                string_to_value: None,
            },
            pain_stages: todo!(),
            but_field_names_can_change: todo!(),
        }
    }
}
