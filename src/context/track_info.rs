//! Information about the track/channel the plugin is inserted on.

/// Information about the track the plugin is on. Not all hosts and plugin APIs support all fields,
/// so most of them are optional.
///
/// This is queried from the host and may change at any time. It can be accessed from
/// [`InitContext`][super::init::InitContext],
/// [`ProcessContext`][super::process::ProcessContext], and
/// [`GuiContext`][super::gui::GuiContext].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TrackInfo {
    /// The name of the track, if available.
    pub name: Option<String>,
    /// The color assigned to the track, if available.
    pub color: Option<TrackColor>,
    /// The number of audio channels on the track, if available.
    pub audio_channel_count: Option<u32>,
    /// The type of track the plugin is on.
    pub track_type: TrackType,
}

/// An RGBA color value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TrackColor {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
    pub alpha: u8,
}

/// The type of track the plugin is inserted on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TrackType {
    /// A regular track.
    #[default]
    Regular,
    /// A return/FX track.
    Return,
    /// A bus/group track.
    Bus,
    /// The master/main output track.
    Master,
}
