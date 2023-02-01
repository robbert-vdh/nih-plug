//! Subcategories for VST3 plugins. This is essentially the same thing as tags, keyword, or
//! categories. Hosts may use these to organize plugins.

/// A subcategory for a VST3 plugin. See
/// <https://github.com/steinbergmedia/vst3_pluginterfaces/blob/bc5ff0f87aaa3cd28c114810f4f03c384421ad2c/vst/ivstaudioprocessor.h#L49-L90>
/// for a list of all predefined subcategories. Multiple subcategories are concatenated to a string
/// separated by pipe characters, and the total length of this string may not exceed 127 characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Vst3SubCategory {
    // These are the main categories, every plugin should have at least one of these, I think
    Fx,
    Instrument,
    Spatial,
    // These are optional
    Analyzer,
    Delay,
    Distortion,
    Drum,
    Dynamics,
    Eq,
    External,
    Filter,
    Generator,
    Mastering,
    Modulation,
    Network,
    Piano,
    PitchShift,
    Restoration,
    Reverb,
    Sampler,
    Synth,
    Tools,
    UpDownmix,
    // These are used for plugins that _only_ support this channel configuration, they're also
    // optional
    Mono,
    Stereo,
    Surround,
    Ambisonics,
    // There are also a couple special 'Only*' subcategories that convey special information about
    // the plugin. The framework is responsible for adding these, and they shouldn't be added
    // manually.
    /// A non-predefined subcategory. Hosts may display this among its plugin categories.
    Custom(&'static str),
}

impl Vst3SubCategory {
    pub fn as_str(&self) -> &'static str {
        match self {
            Vst3SubCategory::Fx => "Fx",
            Vst3SubCategory::Instrument => "Instrument",
            Vst3SubCategory::Spatial => "Spatial",
            Vst3SubCategory::Analyzer => "Analyzer",
            Vst3SubCategory::Delay => "Delay",
            Vst3SubCategory::Distortion => "Distortion",
            Vst3SubCategory::Drum => "Drum",
            Vst3SubCategory::Dynamics => "Dynamics",
            Vst3SubCategory::Eq => "EQ",
            Vst3SubCategory::External => "External",
            Vst3SubCategory::Filter => "Filter",
            Vst3SubCategory::Generator => "Generator",
            Vst3SubCategory::Mastering => "Mastering",
            Vst3SubCategory::Modulation => "Modulation",
            Vst3SubCategory::Network => "Network",
            Vst3SubCategory::Piano => "Piano",
            Vst3SubCategory::PitchShift => "Pitch Shift",
            Vst3SubCategory::Restoration => "Restoration",
            Vst3SubCategory::Reverb => "Reverb",
            Vst3SubCategory::Sampler => "Sampler",
            Vst3SubCategory::Synth => "Synth",
            Vst3SubCategory::Tools => "Tools",
            Vst3SubCategory::UpDownmix => "Up-Downmix",
            Vst3SubCategory::Mono => "Mono",
            Vst3SubCategory::Stereo => "Stereo",
            Vst3SubCategory::Surround => "Surround",
            Vst3SubCategory::Ambisonics => "Ambisonics",
            Vst3SubCategory::Custom(s) => {
                nih_debug_assert!(
                    !s.contains('|'),
                    "'{}' contains a pipe character ('|'), which is not allowed",
                    s
                );

                s
            }
        }
    }
}
