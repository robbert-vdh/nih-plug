//! Different contexts the plugin can use to make callbacks to the host in different...contexts.

use crate::param::internals::ParamPtr;
use crate::param::Param;
use crate::plugin::NoteEvent;

// TODO: ProcessContext for parameter automation and sending events

/// General callbacks the plugin can make during its lifetime. This is passed to the plugin during
/// [`Plugin::initialize()`][crate::plugin::Plugin::initialize()] and as part of
/// [`Plugin::process()`][crate::plugin::Plugin::process()].
//
// # Safety
//
// The implementing wrapper needs to be able to handle concurrent requests, and it should perform
// the actual callback within [MainThreadQueue::do_maybe_async].
pub trait ProcessContext {
    /// Get information about the current transport position and status.
    fn transport(&self) -> &Transport;

    /// Return the next note event, if there is one. The event contains the timing
    ///
    /// TODO: Rethink this API, both in terms of ergonomics, and if we can do this in a way that
    ///       doesn't require locks (because of the thread safe-ness, which we don't really need
    ///       here)
    fn next_midi_event(&mut self) -> Option<NoteEvent>;

    /// Update the current latency of the plugin. If the plugin is currently processing audio, then
    /// this may cause audio playback to be restarted.
    fn set_latency_samples(&self, samples: u32);

    // TODO: Add this, this works similar to [GuiContext::set_parameter] but it adds the parameter
    //       change to a queue (or directly to the VST3 plugin's parameter output queues) instead of
    //       using main thread host automation (and all the locks involved there).
    // fn set_parameter<P: Param>(&self, param: &P, value: P::Plain);
}

/// Callbacks the plugin can make when the user interacts with its GUI such as updating parameter
/// values. This is passed to the plugin during [`Editor::spawn()`][crate::prelude::Editor::spawn()]. All of
/// these functions assume they're being called from the main GUI thread.
//
// # Safety
//
// The implementing wrapper can assume that everything is being called from the main thread. Since
// NIH-plug doesn't own the GUI event loop, this invariant cannot be part of the interface.
pub trait GuiContext: Send + Sync + 'static {
    /// Ask the host to resize the editor window to the size specified by [crate::Editor::size()].
    /// This will return false if the host somehow didn't like this and rejected the resize, in
    /// which case the window should revert to its old size. You should only actually resize your
    /// embedded window once this returns `true`.
    ///
    /// TODO: Host->Plugin resizing has not been implemented yet
    fn request_resize(&self) -> bool;

    /// Inform the host a parameter will be automated. Create a [`ParamSetter`] and use
    /// [`ParamSetter::begin_set_parameter()`] instead for a safe, user friendly API.
    ///
    /// # Safety
    ///
    /// The implementing function still needs to check if `param` actually exists. This function is
    /// mostly marked as unsafe for API reasons.
    unsafe fn raw_begin_set_parameter(&self, param: ParamPtr);

    /// Inform the host a parameter is being automated with an already normalized value. Create a
    /// [`ParamSetter`] and use [`ParamSetter::set_parameter()`] instead for a safe, user friendly
    /// API.
    ///
    /// # Safety
    ///
    /// The implementing function still needs to check if `param` actually exists. This function is
    /// mostly marked as unsafe for API reasons.
    unsafe fn raw_set_parameter_normalized(&self, param: ParamPtr, normalized: f32);

    /// Inform the host a parameter has been automated. Create a [`ParamSetter`] and use
    /// [`ParamSetter::end_set_parameter()`] instead for a safe, user friendly API.
    ///
    /// # Safety
    ///
    /// The implementing function still needs to check if `param` actually exists. This function is
    /// mostly marked as unsafe for API reasons.
    unsafe fn raw_end_set_parameter(&self, param: ParamPtr);
}

/// Information about the plugin's transport. Depending on the plugin API and the host not all
/// fields may be available.
#[derive(Debug)]
pub struct Transport {
    /// Whether the transport is currently running.
    pub playing: bool,
    /// Whether recording is enabled in the project.
    pub recording: bool,
    /// Whether the pre-roll is currently active, if the plugin API reports this information.
    pub preroll_active: Option<bool>,

    /// The sample rate in Hertz. Also passed in
    /// [`Plugin::initialize()`][crate::prelude::Plugin::initialize()], so if you need this then you
    /// can also store that value.
    pub sample_rate: f32,
    /// The proejct's tempo in beats per minute.
    pub tempo: Option<f64>,
    /// The time signature's numerator.
    pub time_sig_numerator: Option<i32>,
    /// The time signature's denominator.
    pub time_sig_denominator: Option<i32>,

    // XXX: VST3 also has a continuous time in samples that ignores loops, but we can't reconstruct
    //      something similar in CLAP so it may be best to just ignore that so you can't rely on it
    /// The position in the song in samples. Can be used to calculate the time in seconds if needed.
    pub(crate) pos_samples: Option<i64>,
    /// The position in the song in quarter notes. Can be used to calculate the time in samples if
    /// needed.
    pub(crate) pos_seconds: Option<f64>,
    /// The position in the song in quarter notes. Can be calculated from the time in seconds and
    /// the tempo if needed.
    pub(crate) pos_beats: Option<f64>,
    /// The last bar's start position in beats. Can be calculated from the beat position and time
    /// signature if needed.
    pub(crate) bar_start_pos_beats: Option<f64>,
    /// The number of the bar at `bar_start_pos_beats`. This starts at 0 for the very first bar at
    /// the start of the song. Can be calculated from the beat position and time signature if
    /// needed.
    pub(crate) bar_number: Option<i32>,

    /// The loop range in samples, if the loop is active and this information is available. None of
    /// the plugin API docs mention whether this is exclusive or inclusive, but just assume that the
    /// end is exclusive. Can be calulcated from the other loop range information if needed.
    pub(crate) loop_range_samples: Option<(i64, i64)>,
    /// The loop range in seconds, if the loop is active and this information is available. None of
    /// the plugin API docs mention whether this is exclusive or inclusive, but just assume that the
    /// end is exclusive. Can be calulcated from the other loop range information if needed.
    pub(crate) loop_range_seconds: Option<(f64, f64)>,
    /// The loop range in quarter notes, if the loop is active and this information is available.
    /// None of the plugin API docs mention whether this is exclusive or inclusive, but just assume
    /// that the end is exclusive. Can be calulcated from the other loop range information if
    /// needed.
    pub(crate) loop_range_beats: Option<(f64, f64)>,
}

/// A convenience helper for setting parameter values. Any changes made here will be broadcasted to
/// the host and reflected in the plugin's [`Params`][crate::param::internals::Params] object. These
/// functions should only be called from the main thread.
pub struct ParamSetter<'a> {
    pub raw_context: &'a dyn GuiContext,
}

// TODO: These conversions have not really been tested yet, there might be an error in there somewhere
impl Transport {
    /// Initialize the transport struct without any information.
    pub(crate) fn new(sample_rate: f32) -> Self {
        Self {
            playing: false,
            recording: false,
            preroll_active: None,

            sample_rate,
            tempo: None,
            time_sig_numerator: None,
            time_sig_denominator: None,

            pos_samples: None,
            pos_seconds: None,
            pos_beats: None,
            bar_start_pos_beats: None,
            bar_number: None,

            loop_range_samples: None,
            loop_range_seconds: None,
            loop_range_beats: None,
        }
    }

    /// The position in the song in samples. Will be calculated from other information if needed.
    pub fn pos_samples(&self) -> Option<i64> {
        match (
            self.pos_samples,
            self.pos_seconds,
            self.pos_beats,
            self.tempo,
        ) {
            (Some(pos_samples), _, _, _) => Some(pos_samples),
            (_, Some(pos_seconds), _, _) => {
                Some((pos_seconds * self.sample_rate as f64).round() as i64)
            }
            (_, _, Some(pos_beats), Some(tempo)) => {
                Some((pos_beats / tempo * 60.0 * self.sample_rate as f64).round() as i64)
            }
            (_, _, _, _) => None,
        }
    }

    /// The position in the song in quarter notes. Will be calculated from other information if
    /// needed.
    pub fn pos_seconds(&self) -> Option<f64> {
        match (
            self.pos_samples,
            self.pos_seconds,
            self.pos_beats,
            self.tempo,
        ) {
            (_, Some(pos_seconds), _, _) => Some(pos_seconds),
            (Some(pos_samples), _, _, _) => Some(pos_samples as f64 / self.sample_rate as f64),
            (_, _, Some(pos_beats), Some(tempo)) => Some(pos_beats / tempo * 60.0),
            (_, _, _, _) => None,
        }
    }

    /// The position in the song in quarter notes. Will be calculated from other information if
    /// needed.
    pub fn pos_beats(&self) -> Option<f64> {
        match (
            self.pos_samples,
            self.pos_seconds,
            self.pos_beats,
            self.tempo,
        ) {
            (_, _, Some(pos_beats), _) => Some(pos_beats),
            (_, Some(pos_seconds), _, Some(tempo)) => Some(pos_seconds / 60.0 * tempo),
            (Some(pos_samples), _, _, Some(tempo)) => {
                Some(pos_samples as f64 / self.sample_rate as f64 / 60.0 * tempo)
            }
            (_, _, _, _) => None,
        }
    }

    /// The last bar's start position in beats. Will be calculated from other information if needed.
    pub fn bar_start_pos_beats(&self) -> Option<f64> {
        if self.bar_start_pos_beats.is_some() {
            return self.bar_start_pos_beats;
        }

        match (
            self.time_sig_numerator,
            self.time_sig_denominator,
            self.pos_beats(),
        ) {
            (Some(time_sig_numerator), Some(time_sig_denominator), Some(pos_beats)) => {
                let quarter_note_bar_length =
                    time_sig_numerator as f64 / time_sig_denominator as f64 * 4.0;
                Some(pos_beats.div_euclid(quarter_note_bar_length))
            }
            (_, _, _) => None,
        }
    }

    /// The number of the bar at `bar_start_pos_beats`. This starts at 0 for the very first bar at
    /// the start of the song. Will be calculated from other information if needed.
    pub fn bar_number(&self) -> Option<i32> {
        if self.bar_number.is_some() {
            return self.bar_number;
        }

        match (
            self.time_sig_numerator,
            self.time_sig_denominator,
            self.pos_beats(),
        ) {
            (Some(time_sig_numerator), Some(time_sig_denominator), Some(pos_beats)) => {
                let quarter_note_bar_length =
                    time_sig_numerator as f64 / time_sig_denominator as f64 * 4.0;
                Some((pos_beats / quarter_note_bar_length).floor() as i32)
            }
            (_, _, _) => None,
        }
    }

    /// The loop range in samples, if the loop is active and this information is available. None of
    /// the plugin API docs mention whether this is exclusive or inclusive, but just assume that the
    /// end is exclusive. Will be calculated from other information if needed.
    pub fn loop_range_samples(&self) -> Option<(i64, i64)> {
        match (
            self.loop_range_samples,
            self.loop_range_seconds,
            self.loop_range_beats,
            self.tempo,
        ) {
            (Some(loop_range_samples), _, _, _) => Some(loop_range_samples),
            (_, Some((start_seconds, end_seconds)), _, _) => Some((
                ((start_seconds * self.sample_rate as f64).round() as i64),
                ((end_seconds * self.sample_rate as f64).round() as i64),
            )),
            (_, _, Some((start_beats, end_beats)), Some(tempo)) => Some((
                (start_beats / tempo * 60.0 * self.sample_rate as f64).round() as i64,
                (end_beats / tempo * 60.0 * self.sample_rate as f64).round() as i64,
            )),
            (_, _, _, _) => None,
        }
    }

    /// The loop range in seconds, if the loop is active and this information is available. None of
    /// the plugin API docs mention whether this is exclusive or inclusive, but just assume that the
    /// end is exclusive. Will be calculated from other information if needed.
    pub fn loop_range_seconds(&self) -> Option<(f64, f64)> {
        match (
            self.loop_range_samples,
            self.loop_range_seconds,
            self.loop_range_beats,
            self.tempo,
        ) {
            (_, Some(loop_range_seconds), _, _) => Some(loop_range_seconds),
            (Some((start_samples, end_samples)), _, _, _) => Some((
                start_samples as f64 / self.sample_rate as f64,
                end_samples as f64 / self.sample_rate as f64,
            )),
            (_, _, Some((start_beats, end_beats)), Some(tempo)) => {
                Some((start_beats / tempo * 60.0, end_beats / tempo * 60.0))
            }
            (_, _, _, _) => None,
        }
    }

    /// The loop range in quarter notes, if the loop is active and this information is available.
    /// None of the plugin API docs mention whether this is exclusive or inclusive, but just assume
    /// that the end is exclusive. Will be calculated from other information if needed.
    pub fn loop_range_beats(&self) -> Option<(f64, f64)> {
        match (
            self.loop_range_samples,
            self.loop_range_seconds,
            self.loop_range_beats,
            self.tempo,
        ) {
            (_, _, Some(loop_range_beats), _) => Some(loop_range_beats),
            (_, Some((start_seconds, end_seconds)), _, Some(tempo)) => {
                Some((start_seconds / 60.0 * tempo, end_seconds / 60.0 * tempo))
            }
            (Some((start_samples, end_samples)), _, _, Some(tempo)) => Some((
                start_samples as f64 / self.sample_rate as f64 / 60.0 * tempo,
                end_samples as f64 / self.sample_rate as f64 / 60.0 * tempo,
            )),
            (_, _, _, _) => None,
        }
    }
}

impl<'a> ParamSetter<'a> {
    pub fn new(context: &'a dyn GuiContext) -> Self {
        Self {
            raw_context: context,
        }
    }

    /// Inform the host that you will start automating a parmater. This needs to be called before
    /// calling [`set_parameter()`][Self::set_parameter()] for the specified parameter.
    pub fn begin_set_parameter<P: Param>(&self, param: &P) {
        unsafe { self.raw_context.raw_begin_set_parameter(param.as_ptr()) };
    }

    /// Set a parameter to the specified parameter value. You will need to call
    /// [`begin_set_parameter()`][Self::begin_set_parameter()] before and
    /// [`end_set_parameter()`][Self::end_set_parameter()] after calling this so the host can
    /// properly record automation for the parameter. This can be called multiple times in a row
    /// before calling [`end_set_parameter()`][Self::end_set_parameter()], for instance when moving
    /// a slider around.
    ///
    /// This function assumes you're already calling this from a GUI thread. Calling any of these
    /// functions from any other thread may result in unexpected behavior.
    pub fn set_parameter<P: Param>(&self, param: &P, value: P::Plain) {
        let ptr = param.as_ptr();
        let normalized = param.preview_normalized(value);
        unsafe {
            self.raw_context
                .raw_set_parameter_normalized(ptr, normalized)
        };
    }

    /// Set a parameter to an already normalized value. Works exactly the same as
    /// [`set_parameter()`][Self::set_parameter()] and needs to follow the same rules, but this may
    /// be useful when implementing a GUI.
    ///
    /// This does not perform any snapping. Consider converting the normalized value to a plain
    /// value and setting that with [`set_parameter()`][Self::set_parameter()] instead so the
    /// normalized value known to the host matches `param.normalized_value()`.
    pub fn set_parameter_normalized<P: Param>(&self, param: &P, normalized: f32) {
        let ptr = param.as_ptr();
        unsafe {
            self.raw_context
                .raw_set_parameter_normalized(ptr, normalized)
        };
    }

    /// Inform the host that you are done automating a parameter. This needs to be called after one
    /// or more [`set_parameter()`][Self::set_parameter()] calls for a parameter so the host knows
    /// the automation gesture has finished.
    pub fn end_set_parameter<P: Param>(&self, param: &P) {
        unsafe { self.raw_context.raw_end_set_parameter(param.as_ptr()) };
    }
}
