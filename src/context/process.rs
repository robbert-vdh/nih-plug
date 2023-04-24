//! A context passed during the process function.

use super::PluginApi;
use crate::prelude::{Plugin, PluginNoteEvent};

/// Contains both context data and callbacks the plugin can use during processing. Most notably this
/// is how a plugin sends and receives note events, gets transport information, and accesses
/// sidechain inputs and auxiliary outputs. This is passed to the plugin during as part of
/// [`Plugin::process()`][crate::plugin::Plugin::process()].
//
// # Safety
//
// The implementing wrapper needs to be able to handle concurrent requests, and it should perform
// the actual callback within [MainThreadQueue::schedule_gui].
pub trait ProcessContext<P: Plugin> {
    /// Get the current plugin API.
    fn plugin_api(&self) -> PluginApi;

    /// Execute a task on a background thread using `[Plugin::task_executor]`. This allows you to
    /// defer expensive tasks for later without blocking either the process function or the GUI
    /// thread. As long as creating the `task` is realtime-safe, this operation is too.
    ///
    /// # Note
    ///
    /// Scheduling the same task multiple times will cause those duplicate tasks to pile up. Try to
    /// either prevent this from happening, or check whether the task still needs to be completed in
    /// your task executor.
    fn execute_background(&self, task: P::BackgroundTask);

    /// Execute a task on a background thread using `[Plugin::task_executor]`. As long as creating
    /// the `task` is realtime-safe, this operation is too.
    ///
    /// # Note
    ///
    /// Scheduling the same task multiple times will cause those duplicate tasks to pile up. Try to
    /// either prevent this from happening, or check whether the task still needs to be completed in
    /// your task executor.
    fn execute_gui(&self, task: P::BackgroundTask);

    /// Get information about the current transport position and status.
    fn transport(&self) -> &Transport;

    /// Returns the next note event, if there is one. Use
    /// [`NoteEvent::timing()`][crate::prelude::NoteEvent::timing()] to get the event's timing
    /// within the buffer. Only available when
    /// [`Plugin::MIDI_INPUT`][crate::prelude::Plugin::MIDI_INPUT] is set.
    ///
    /// # Usage
    ///
    /// You will likely want to use this with a loop, since there may be zero, one, or more events
    /// for a sample:
    ///
    /// ```ignore
    /// let mut next_event = context.next_event();
    /// for (sample_id, channel_samples) in buffer.iter_samples().enumerate() {
    ///     while let Some(event) = next_event {
    ///         if event.timing() != sample_id as u32 {
    ///             break;
    ///         }
    ///
    ///         match event {
    ///             NoteEvent::NoteOn { note, velocity, .. } => { ... },
    ///             NoteEvent::NoteOff { note, .. } if note == 69 => { ... },
    ///             NoteEvent::PolyPressure { note, pressure, .. } { ... },
    ///             _ => (),
    ///         }
    ///
    ///         next_event = context.next_event();
    ///     }
    ///
    ///     // Do something with `channel_samples`...
    /// }
    ///
    /// ProcessStatus::Normal
    /// ```
    fn next_event(&mut self) -> Option<PluginNoteEvent<P>>;

    /// Send an event to the host. Only available when
    /// [`Plugin::MIDI_OUTPUT`][crate::prelude::Plugin::MIDI_INPUT] is set. Will not do anything
    /// otherwise.
    fn send_event(&mut self, event: PluginNoteEvent<P>);

    /// Update the current latency of the plugin. If the plugin is currently processing audio, then
    /// this may cause audio playback to be restarted.
    fn set_latency_samples(&self, samples: u32);

    /// Set the current voice **capacity** for this plugin (so not the number of currently active
    /// voices). This may only be called if
    /// [`ClapPlugin::CLAP_POLY_MODULATION_CONFIG`][crate::prelude::ClapPlugin::CLAP_POLY_MODULATION_CONFIG]
    /// is set. `capacity` must be between 1 and the configured maximum capacity. Changing this at
    /// runtime allows the host to better optimize polyphonic modulation, or to switch to strictly
    /// monophonic modulation when dropping the capacity down to 1.
    fn set_current_voice_capacity(&self, capacity: u32);

    // TODO: Add this, this works similar to [GuiContext::set_parameter] but it adds the parameter
    //       change to a queue (or directly to the VST3 plugin's parameter output queues) instead of
    //       using main thread host automation (and all the locks involved there).
    // fn set_parameter<P: Param>(&self, param: &P, value: P::Plain);
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
    /// The project's tempo in beats per minute.
    pub tempo: Option<f64>,
    /// The time signature's numerator.
    pub time_sig_numerator: Option<i32>,
    /// The time signature's denominator.
    pub time_sig_denominator: Option<i32>,

    // XXX: VST3 also has a continuous time in samples that ignores loops, but we can't reconstruct
    //      something similar in CLAP so it may be best to just ignore that so you can't rely on it
    /// The position in the song in samples. Can be used to calculate the time in seconds if needed.
    pub(crate) pos_samples: Option<i64>,
    /// The position in the song in seconds. Can be used to calculate the time in samples if needed.
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
    /// end is exclusive. Can be calculated from the other loop range information if needed.
    pub(crate) loop_range_samples: Option<(i64, i64)>,
    /// The loop range in seconds, if the loop is active and this information is available. None of
    /// the plugin API docs mention whether this is exclusive or inclusive, but just assume that the
    /// end is exclusive. Can be calculated from the other loop range information if needed.
    pub(crate) loop_range_seconds: Option<(f64, f64)>,
    /// The loop range in quarter notes, if the loop is active and this information is available.
    /// None of the plugin API docs mention whether this is exclusive or inclusive, but just assume
    /// that the end is exclusive. Can be calculated from the other loop range information if
    /// needed.
    pub(crate) loop_range_beats: Option<(f64, f64)>,
}

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

    /// The position in the song in seconds. Can be used to calculate the time in samples if needed.
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
                Some((pos_beats / quarter_note_bar_length).floor() * quarter_note_bar_length)
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
