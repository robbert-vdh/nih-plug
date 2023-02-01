use std::borrow::Borrow;
use std::cmp;
use std::ffi::c_void;
use std::mem::{self, MaybeUninit};
use std::ptr;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use vst3_com::vst::{DataEvent, IProcessContextRequirementsFlags, ProcessModes};
use vst3_sys::base::{kInvalidArgument, kNoInterface, kResultFalse, kResultOk, tresult, TBool};
use vst3_sys::base::{IBStream, IPluginBase};
use vst3_sys::utils::SharedVstPtr;
use vst3_sys::vst::{
    kNoParamId, kNoParentUnitId, kNoProgramListId, kRootUnitId, Event, EventTypes, IAudioProcessor,
    IComponent, IEditController, IEventList, IMidiMapping, INoteExpressionController,
    IParamValueQueue, IParameterChanges, IProcessContextRequirements, IUnitInfo,
    LegacyMidiCCOutEvent, NoteExpressionTypeInfo, NoteExpressionValueDescription, NoteOffEvent,
    NoteOnEvent, ParameterFlags, PolyPressureEvent, ProgramListInfo, TChar, UnitInfo,
};
use vst3_sys::VST3;
use widestring::U16CStr;

use super::inner::{ProcessEvent, Task, WrapperInner};
use super::note_expressions::{self, NoteExpressionController};
use super::util::{
    u16strlcpy, VstPtr, VST3_MIDI_CCS, VST3_MIDI_NUM_PARAMS, VST3_MIDI_PARAMS_START,
};
use super::util::{VST3_MIDI_CHANNELS, VST3_MIDI_PARAMS_END};
use super::view::WrapperView;
use crate::buffer::Buffer;
use crate::context::process::Transport;
use crate::midi::sysex::SysExMessage;
use crate::midi::{MidiConfig, NoteEvent};
use crate::params::ParamFlags;
use crate::plugin::{
    AuxiliaryBuffers, AuxiliaryIOConfig, BufferConfig, BusConfig, ProcessMode, ProcessStatus,
    Vst3Plugin,
};
use crate::util::permit_alloc;
use crate::wrapper::state;
use crate::wrapper::util::process_wrapper;

// Alias needed for the VST3 attribute macro
use vst3_sys as vst3_com;

#[VST3(implements(
    IComponent,
    IEditController,
    IAudioProcessor,
    IMidiMapping,
    INoteExpressionController,
    IProcessContextRequirements,
    IUnitInfo
))]
pub(crate) struct Wrapper<P: Vst3Plugin> {
    inner: Arc<WrapperInner<P>>,
}

impl<P: Vst3Plugin> Wrapper<P> {
    pub fn new() -> Box<Self> {
        Self::allocate(WrapperInner::new())
    }
}

impl<P: Vst3Plugin> IPluginBase for Wrapper<P> {
    unsafe fn initialize(&self, _context: *mut c_void) -> tresult {
        // We currently don't need or allow any initialization logic
        kResultOk
    }

    unsafe fn terminate(&self) -> tresult {
        kResultOk
    }
}

impl<P: Vst3Plugin> IComponent for Wrapper<P> {
    unsafe fn get_controller_class_id(&self, _tuid: *mut vst3_sys::IID) -> tresult {
        // We won't separate the edit controller to keep the implementation a bit smaller
        kNoInterface
    }

    unsafe fn set_io_mode(&self, _mode: vst3_sys::vst::IoMode) -> tresult {
        // Not quite sure what the point of this is when the processing setup also receives similar
        // information
        kResultOk
    }

    unsafe fn get_bus_count(
        &self,
        type_: vst3_sys::vst::MediaType,
        dir: vst3_sys::vst::BusDirection,
    ) -> i32 {
        // HACK: Bitwig will not call the process function at all if the plugin does not have any
        //       audio IO, so we'll add a zero channel output to work around this if that is the
        //       case
        let no_main_audio_io = P::DEFAULT_INPUT_CHANNELS == 0 && P::DEFAULT_OUTPUT_CHANNELS == 0;

        // A plugin has a main input and output bus if the default number of channels is non-zero,
        // and a plugin can also have auxiliary input and output busses
        match type_ {
            x if x == vst3_sys::vst::MediaTypes::kAudio as i32
                && dir == vst3_sys::vst::BusDirections::kInput as i32 =>
            {
                let main_busses = if P::DEFAULT_INPUT_CHANNELS > 0 { 1 } else { 0 };
                let aux_busses = P::DEFAULT_AUX_INPUTS.unwrap_or_default().num_busses as i32;

                main_busses + aux_busses
            }
            x if x == vst3_sys::vst::MediaTypes::kAudio as i32
                && dir == vst3_sys::vst::BusDirections::kOutput as i32 =>
            {
                let main_busses = if P::DEFAULT_OUTPUT_CHANNELS > 0 { 1 } else { 0 };
                let aux_busses = P::DEFAULT_AUX_OUTPUTS.unwrap_or_default().num_busses as i32;

                if no_main_audio_io {
                    1
                } else {
                    main_busses + aux_busses
                }
            }
            x if x == vst3_sys::vst::MediaTypes::kEvent as i32
                && dir == vst3_sys::vst::BusDirections::kInput as i32
                && P::MIDI_INPUT >= MidiConfig::Basic =>
            {
                1
            }
            x if x == vst3_sys::vst::MediaTypes::kEvent as i32
                && dir == vst3_sys::vst::BusDirections::kOutput as i32
                && P::MIDI_OUTPUT >= MidiConfig::Basic =>
            {
                1
            }
            _ => 0,
        }
    }

    unsafe fn get_bus_info(
        &self,
        type_: vst3_sys::vst::MediaType,
        dir: vst3_sys::vst::BusDirection,
        index: i32,
        info: *mut vst3_sys::vst::BusInfo,
    ) -> tresult {
        check_null_ptr!(info);

        // HACK: Bitwig will not call the process function at all if the plugin does not have any
        //       audio IO, so we'll add a zero channel output to work around this if that is the
        //       case
        let no_main_audio_io = P::DEFAULT_INPUT_CHANNELS == 0 && P::DEFAULT_OUTPUT_CHANNELS == 0;

        match (type_, dir, index) {
            (t, _, _) if t == vst3_sys::vst::MediaTypes::kAudio as i32 => {
                *info = mem::zeroed();

                let info = &mut *info;
                info.media_type = vst3_sys::vst::MediaTypes::kAudio as i32;
                info.direction = dir;
                info.flags = vst3_sys::vst::BusFlags::kDefaultActive as u32;

                // This is fun since main IO is optional
                let bus_config = self.inner.current_bus_config.load();
                if dir == vst3_sys::vst::BusDirections::kInput as i32 {
                    let aux_inputs_only =
                        P::DEFAULT_INPUT_CHANNELS == 0 && P::DEFAULT_AUX_INPUTS.is_some();
                    let aux_input_start_idx = if aux_inputs_only { 0 } else { 1 };
                    if !aux_inputs_only && index == 0 {
                        info.bus_type = vst3_sys::vst::BusTypes::kMain as i32;
                        info.channel_count = bus_config.num_input_channels as i32;
                        u16strlcpy(&mut info.name, P::PORT_NAMES.main_input.unwrap_or("Input"));

                        kResultOk
                    } else if (aux_input_start_idx
                        ..(aux_input_start_idx + bus_config.aux_input_busses.num_busses as i32))
                        .contains(&index)
                    {
                        info.bus_type = vst3_sys::vst::BusTypes::kAux as i32;
                        info.channel_count = bus_config.aux_input_busses.num_channels as i32;

                        let aux_input_idx = index - aux_input_start_idx;
                        let custom_port_name = P::PORT_NAMES
                            .aux_inputs
                            .map(|names| names[aux_input_idx as usize]);
                        if bus_config.aux_input_busses.num_busses <= 1 {
                            u16strlcpy(
                                &mut info.name,
                                custom_port_name.unwrap_or("Sidechain Input"),
                            );
                        } else {
                            u16strlcpy(
                                &mut info.name,
                                custom_port_name
                                    .unwrap_or(&format!("Sidechain Input {}", aux_input_idx + 1)),
                            );
                        }

                        kResultOk
                    } else {
                        kInvalidArgument
                    }
                } else if dir == vst3_sys::vst::BusDirections::kOutput as i32 {
                    let aux_outputs_only =
                        P::DEFAULT_OUTPUT_CHANNELS == 0 && P::DEFAULT_AUX_OUTPUTS.is_some();
                    let aux_output_start_idx = if aux_outputs_only { 0 } else { 1 };
                    if (!aux_outputs_only || no_main_audio_io) && index == 0 {
                        info.bus_type = vst3_sys::vst::BusTypes::kMain as i32;
                        info.channel_count = bus_config.num_output_channels as i32;
                        u16strlcpy(
                            &mut info.name,
                            P::PORT_NAMES.main_output.unwrap_or("Output"),
                        );

                        kResultOk
                    } else if (aux_output_start_idx
                        ..(aux_output_start_idx + bus_config.aux_output_busses.num_busses as i32))
                        .contains(&index)
                    {
                        info.bus_type = vst3_sys::vst::BusTypes::kAux as i32;
                        info.channel_count = bus_config.aux_output_busses.num_channels as i32;

                        let aux_output_idx = index - aux_output_start_idx;
                        let custom_port_name = P::PORT_NAMES
                            .aux_outputs
                            .map(|names| names[aux_output_idx as usize]);
                        if bus_config.aux_output_busses.num_busses <= 1 {
                            u16strlcpy(
                                &mut info.name,
                                custom_port_name.unwrap_or("Auxiliary Output"),
                            );
                        } else {
                            u16strlcpy(
                                &mut info.name,
                                custom_port_name
                                    .unwrap_or(&format!("Auxiliary Output {}", aux_output_idx + 1)),
                            );
                        }

                        kResultOk
                    } else {
                        kInvalidArgument
                    }
                } else {
                    kInvalidArgument
                }
            }
            (t, d, 0)
                if t == vst3_sys::vst::MediaTypes::kEvent as i32
                    && d == vst3_sys::vst::BusDirections::kInput as i32
                    && P::MIDI_INPUT >= MidiConfig::Basic =>
            {
                *info = mem::zeroed();

                let info = &mut *info;
                info.media_type = vst3_sys::vst::MediaTypes::kEvent as i32;
                info.direction = vst3_sys::vst::BusDirections::kInput as i32;
                info.channel_count = 16;
                u16strlcpy(&mut info.name, "Note Input");
                info.bus_type = vst3_sys::vst::BusTypes::kMain as i32;
                info.flags = vst3_sys::vst::BusFlags::kDefaultActive as u32;
                kResultOk
            }
            (t, d, 0)
                if t == vst3_sys::vst::MediaTypes::kEvent as i32
                    && d == vst3_sys::vst::BusDirections::kOutput as i32
                    && P::MIDI_OUTPUT >= MidiConfig::Basic =>
            {
                *info = mem::zeroed();

                let info = &mut *info;
                info.media_type = vst3_sys::vst::MediaTypes::kEvent as i32;
                info.direction = vst3_sys::vst::BusDirections::kOutput as i32;
                info.channel_count = 16;
                u16strlcpy(&mut info.name, "Note Output");
                info.bus_type = vst3_sys::vst::BusTypes::kMain as i32;
                info.flags = vst3_sys::vst::BusFlags::kDefaultActive as u32;
                kResultOk
            }
            _ => kInvalidArgument,
        }
    }

    unsafe fn get_routing_info(
        &self,
        in_info: *mut vst3_sys::vst::RoutingInfo,
        out_info: *mut vst3_sys::vst::RoutingInfo,
    ) -> tresult {
        check_null_ptr!(in_info, out_info);

        *out_info = mem::zeroed();

        let in_info = &*in_info;
        let out_info = &mut *out_info;
        match (in_info.media_type, in_info.bus_index) {
            (t, 0)
                if t == vst3_sys::vst::MediaTypes::kAudio as i32
                    // We only have an IO pair when the plugin has both a main input and a main output
                    && P::DEFAULT_INPUT_CHANNELS > 0
                    && P::DEFAULT_OUTPUT_CHANNELS > 0 =>
            {
                out_info.media_type = vst3_sys::vst::MediaTypes::kAudio as i32;
                out_info.bus_index = in_info.bus_index;
                out_info.channel = in_info.channel;

                kResultOk
            }
            (t, 0)
                if t == vst3_sys::vst::MediaTypes::kEvent as i32
                    && P::MIDI_INPUT >= MidiConfig::Basic
                    && P::MIDI_OUTPUT >= MidiConfig::Basic =>
            {
                out_info.media_type = vst3_sys::vst::MediaTypes::kEvent as i32;
                out_info.bus_index = in_info.bus_index;
                out_info.channel = in_info.channel;

                kResultOk
            }
            _ => kResultFalse,
        }
    }

    unsafe fn activate_bus(
        &self,
        type_: vst3_sys::vst::MediaType,
        dir: vst3_sys::vst::BusDirection,
        index: i32,
        _state: vst3_sys::base::TBool,
    ) -> tresult {
        // HACK: Bitwig will not call the process function at all if the plugin does not have any
        //       audio IO, so we'll add a zero channel output to work around this if that is the
        //       case
        let no_main_audio_io = P::DEFAULT_INPUT_CHANNELS == 0 && P::DEFAULT_OUTPUT_CHANNELS == 0;

        // We don't need any special handling here
        match (type_, dir, index) {
            (t, d, _)
                if t == vst3_sys::vst::MediaTypes::kAudio as i32
                    && d == vst3_sys::vst::BusDirections::kInput as i32 =>
            {
                let main_busses = if P::DEFAULT_INPUT_CHANNELS > 0 { 1 } else { 0 };
                let aux_busses = P::DEFAULT_AUX_INPUTS.unwrap_or_default().num_busses as i32;

                if (0..main_busses + aux_busses).contains(&index) {
                    kResultOk
                } else {
                    kInvalidArgument
                }
            }
            (t, d, _)
                if t == vst3_sys::vst::MediaTypes::kAudio as i32
                    && d == vst3_sys::vst::BusDirections::kOutput as i32 =>
            {
                let main_busses = if P::DEFAULT_OUTPUT_CHANNELS > 0 || no_main_audio_io {
                    1
                } else {
                    0
                };
                let aux_busses = P::DEFAULT_AUX_OUTPUTS.unwrap_or_default().num_busses as i32;

                if (0..main_busses + aux_busses).contains(&index) {
                    kResultOk
                } else {
                    kInvalidArgument
                }
            }
            (t, d, 0)
                if t == vst3_sys::vst::MediaTypes::kEvent as i32
                    && d == vst3_sys::vst::BusDirections::kInput as i32
                    && P::MIDI_INPUT >= MidiConfig::Basic =>
            {
                kResultOk
            }
            (t, d, 0)
                if t == vst3_sys::vst::MediaTypes::kEvent as i32
                    && d == vst3_sys::vst::BusDirections::kOutput as i32
                    && P::MIDI_OUTPUT >= MidiConfig::Basic =>
            {
                kResultOk
            }
            _ => kInvalidArgument,
        }
    }

    unsafe fn set_active(&self, state: TBool) -> tresult {
        // We could call initialize in `IAudioProcessor::setup_processing()`, but REAPER will set
        // the bus arrangements between that function and this function. So to be able to handle
        // custom channel layout overrides we need to initialize here.
        match (state != 0, self.inner.current_buffer_config.load()) {
            (true, Some(buffer_config)) => {
                // Before initializing the plugin, make sure all smoothers are set the the default values
                for param in self.inner.param_by_hash.values() {
                    param.update_smoother(buffer_config.sample_rate, true);
                }

                // NOTE: This needs to be dropped after the `plugin` lock to avoid deadlocks
                let mut init_context = self.inner.make_init_context();
                let bus_config = self.inner.current_bus_config.load();
                let mut plugin = self.inner.plugin.lock();
                if plugin.initialize(&bus_config, &buffer_config, &mut init_context) {
                    // NOTE: We don't call `Plugin::reset()` here. The call is done in `set_process()`
                    //       instead. Otherwise we would call the function twice, and `set_process()` needs
                    //       to be called after this function before the plugin may process audio again.

                    // Preallocate enough room in the output slices vector so we can convert a `*mut *mut
                    // f32` to a `&mut [&mut f32]` in the process call
                    self.inner
                        .output_buffer
                        .borrow_mut()
                        .set_slices(0, |output_slices| {
                            output_slices
                                .resize_with(bus_config.num_output_channels as usize, || &mut [])
                        });

                    // Also allocate both the buffers and the slices pointing to those buffers for
                    // sidechain inputs. The slices will be assigned in the process function as this
                    // object may have been moved before then.
                    let mut aux_input_storage = self.inner.aux_input_storage.borrow_mut();
                    aux_input_storage
                        .resize_with(bus_config.aux_input_busses.num_busses as usize, Vec::new);
                    for bus_storage in aux_input_storage.iter_mut() {
                        bus_storage.resize_with(
                            bus_config.aux_input_busses.num_channels as usize,
                            Vec::new,
                        );
                        for channel_storage in bus_storage {
                            channel_storage.resize(buffer_config.max_buffer_size as usize, 0.0);
                        }
                    }

                    let mut aux_input_buffers = self.inner.aux_input_buffers.borrow_mut();
                    aux_input_buffers.resize_with(
                        bus_config.aux_input_busses.num_busses as usize,
                        Buffer::default,
                    );
                    for buffer in aux_input_buffers.iter_mut() {
                        buffer.set_slices(0, |channel_slices| {
                            channel_slices.resize_with(
                                bus_config.aux_input_busses.num_channels as usize,
                                || &mut [],
                            )
                        });
                    }

                    // And the same thing for the output buffers
                    let mut aux_output_buffers = self.inner.aux_output_buffers.borrow_mut();
                    aux_output_buffers.resize_with(
                        bus_config.aux_output_busses.num_busses as usize,
                        Buffer::default,
                    );
                    for buffer in aux_output_buffers.iter_mut() {
                        buffer.set_slices(0, |channel_slices| {
                            channel_slices.resize_with(
                                bus_config.aux_output_busses.num_channels as usize,
                                || &mut [],
                            )
                        });
                    }

                    kResultOk
                } else {
                    kResultFalse
                }
            }
            (true, None) => kResultFalse,
            (false, _) => {
                self.inner.plugin.lock().deactivate();

                kResultOk
            }
        }
    }

    unsafe fn set_state(&self, state: SharedVstPtr<dyn IBStream>) -> tresult {
        check_null_ptr!(state);

        let state = state.upgrade().unwrap();

        // We need to know how large the state is before we can read it. The current position can be
        // zero, but it can also be something else. Bitwig prepends the preset header in the stream,
        // while some other hosts don't expose that to the plugin.
        let mut current_pos = 0;
        let mut eof_pos = 0;
        if state.tell(&mut current_pos) != kResultOk
            || state.seek(0, vst3_sys::base::kIBSeekEnd, &mut eof_pos) != kResultOk
            || state.seek(current_pos, vst3_sys::base::kIBSeekSet, ptr::null_mut()) != kResultOk
        {
            nih_debug_assert_failure!("Could not get the stream length");
            return kResultFalse;
        }

        let stream_byte_size = (eof_pos - current_pos) as i32;
        let mut num_bytes_read = 0;
        let mut read_buffer: Vec<u8> = Vec::with_capacity(stream_byte_size as usize);
        state.read(
            read_buffer.as_mut_ptr() as *mut c_void,
            read_buffer.capacity() as i32,
            &mut num_bytes_read,
        );
        read_buffer.set_len(num_bytes_read as usize);

        // If the size is zero, some hosts will always return `kResultFalse` even if the read was
        // 'successful', so we can't check the return value but we can check the number of bytes
        // read.
        if read_buffer.len() != stream_byte_size as usize {
            nih_debug_assert_failure!("Unexpected stream length");
            return kResultFalse;
        }

        let success = state::deserialize_json::<P>(
            &read_buffer,
            self.inner.params.clone(),
            state::make_params_getter(&self.inner.param_by_hash, &self.inner.param_id_to_hash),
            self.inner.current_buffer_config.load().as_ref(),
        );
        if !success {
            return kResultFalse;
        }

        if let Some(buffer_config) = self.inner.current_buffer_config.load() {
            // NOTE: This needs to be dropped after the `plugin` lock to avoid deadlocks
            let mut init_context = self.inner.make_init_context();
            let bus_config = self.inner.current_bus_config.load();
            let mut plugin = self.inner.plugin.lock();
            plugin.initialize(&bus_config, &buffer_config, &mut init_context);
            // TODO: This also goes for the CLAP version, but should we call reset here? Won't the
            //       host always restart playback? Check this with a couple of hosts and remove the
            //       duplicate reset if it's not needed.
            process_wrapper(|| plugin.reset());
        }

        // Reinitialize the plugin after loading state so it can respond to the new parameter values
        let task_posted = self.inner.schedule_gui(Task::ParameterValuesChanged);
        nih_debug_assert!(task_posted, "The task queue is full, dropping task...");

        nih_trace!("Loaded state ({} bytes)", read_buffer.len());

        kResultOk
    }

    unsafe fn get_state(&self, state: SharedVstPtr<dyn IBStream>) -> tresult {
        check_null_ptr!(state);

        let state = state.upgrade().unwrap();

        let serialized = state::serialize_json::<P>(
            self.inner.params.clone(),
            state::make_params_iter(&self.inner.param_by_hash, &self.inner.param_id_to_hash),
        );
        match serialized {
            Ok(serialized) => {
                let mut num_bytes_written = 0;
                let result = state.write(
                    serialized.as_ptr() as *const c_void,
                    serialized.len() as i32,
                    &mut num_bytes_written,
                );

                nih_debug_assert_eq!(result, kResultOk);
                nih_debug_assert_eq!(num_bytes_written as usize, serialized.len());

                nih_trace!("Saved state ({} bytes)", serialized.len());

                kResultOk
            }
            Err(err) => {
                nih_debug_assert_failure!("Could not save state: {:#}", err);
                kResultFalse
            }
        }
    }
}

impl<P: Vst3Plugin> IEditController for Wrapper<P> {
    unsafe fn set_component_state(&self, _state: SharedVstPtr<dyn IBStream>) -> tresult {
        // We have a single file component, so we don't need to do anything here
        kResultOk
    }

    unsafe fn set_state(&self, _state: SharedVstPtr<dyn IBStream>) -> tresult {
        // We don't store any separate state here. The plugin's state will have been restored
        // through the component. Calling that same function here will likely lead to duplicate
        // state restores
        kResultOk
    }

    unsafe fn get_state(&self, _state: SharedVstPtr<dyn IBStream>) -> tresult {
        // Same for this function
        kResultOk
    }

    unsafe fn get_parameter_count(&self) -> i32 {
        // We need to add a whole bunch of parameters if the plugin accepts MIDI CCs
        if P::MIDI_INPUT >= MidiConfig::MidiCCs {
            self.inner.param_hashes.len() as i32 + VST3_MIDI_NUM_PARAMS as i32
        } else {
            self.inner.param_hashes.len() as i32
        }
    }

    unsafe fn get_parameter_info(
        &self,
        param_index: i32,
        info: *mut vst3_sys::vst::ParameterInfo,
    ) -> tresult {
        check_null_ptr!(info);

        if param_index < 0 || param_index > self.get_parameter_count() {
            return kInvalidArgument;
        }

        *info = std::mem::zeroed();
        let info = &mut *info;

        // If the parameter is a generated MIDI CC/channel pressure/pitch bend then it needs to be
        // handled separately
        let num_actual_params = self.inner.param_hashes.len() as i32;
        if P::MIDI_INPUT >= MidiConfig::MidiCCs && param_index >= num_actual_params {
            let midi_param_relative_idx = (param_index - num_actual_params) as u32;
            // This goes up to 130 for the 128 CCs followed by channel pressure and pitch bend
            let midi_cc = midi_param_relative_idx % VST3_MIDI_CCS;
            let midi_channel = midi_param_relative_idx / VST3_MIDI_CCS;
            let name = match midi_cc {
                // kAfterTouch
                128 => format!("MIDI Ch. {} Channel Pressure", midi_channel + 1),
                // kPitchBend
                129 => format!("MIDI Ch. {} Pitch Bend", midi_channel + 1),
                n => format!("MIDI Ch. {} CC {}", midi_channel + 1, n),
            };

            info.id = VST3_MIDI_PARAMS_START + midi_param_relative_idx;
            u16strlcpy(&mut info.title, &name);
            u16strlcpy(&mut info.short_title, &name);
            info.flags = ParameterFlags::kIsReadOnly as i32 | (1 << 4); // kIsHidden
        } else {
            let param_hash = &self.inner.param_hashes[param_index as usize];
            let param_unit = &self
                .inner
                .param_units
                .get_vst3_unit_id(*param_hash)
                .expect("Inconsistent parameter data");
            let param_ptr = &self.inner.param_by_hash[param_hash];
            let default_value = param_ptr.default_normalized_value();
            let flags = param_ptr.flags();
            let automatable = !flags.contains(ParamFlags::NON_AUTOMATABLE);
            let hidden = flags.contains(ParamFlags::HIDDEN);
            let is_bypass = flags.contains(ParamFlags::BYPASS);

            info.id = *param_hash;
            u16strlcpy(&mut info.title, param_ptr.name());
            u16strlcpy(&mut info.short_title, param_ptr.name());
            u16strlcpy(&mut info.units, param_ptr.unit());
            info.step_count = param_ptr.step_count().unwrap_or(0) as i32;
            info.default_normalized_value = default_value as f64;
            info.unit_id = *param_unit;
            info.flags = 0;
            if automatable && !hidden {
                info.flags |= ParameterFlags::kCanAutomate as i32;
            }
            if hidden {
                info.flags |= ParameterFlags::kIsReadOnly as i32 | (1 << 4); // kIsHidden
            }
            if is_bypass {
                info.flags |= ParameterFlags::kIsBypass as i32;
            }
        }

        kResultOk
    }

    unsafe fn get_param_string_by_value(
        &self,
        id: u32,
        value_normalized: f64,
        string: *mut TChar,
    ) -> tresult {
        check_null_ptr!(string);

        let dest = &mut *(string as *mut [TChar; 128]);

        // TODO: We don't implement these methods at all for our generated MIDI CC parameters,
        //       should be fine right? They should be hidden anyways.
        match self.inner.param_by_hash.get(&id) {
            Some(param_ptr) => {
                u16strlcpy(
                    dest,
                    &param_ptr.normalized_value_to_string(value_normalized as f32, false),
                );

                kResultOk
            }
            _ => kInvalidArgument,
        }
    }

    unsafe fn get_param_value_by_string(
        &self,
        id: u32,
        string: *const TChar,
        value_normalized: *mut f64,
    ) -> tresult {
        check_null_ptr!(string, value_normalized);

        let string = match U16CStr::from_ptr_str(string as *const u16).to_string() {
            Ok(s) => s,
            Err(_) => return kInvalidArgument,
        };

        match self.inner.param_by_hash.get(&id) {
            Some(param_ptr) => {
                let value = match param_ptr.string_to_normalized_value(&string) {
                    Some(v) => v as f64,
                    None => return kResultFalse,
                };
                *value_normalized = value;

                kResultOk
            }
            _ => kInvalidArgument,
        }
    }

    unsafe fn normalized_param_to_plain(&self, id: u32, value_normalized: f64) -> f64 {
        match self.inner.param_by_hash.get(&id) {
            Some(param_ptr) => param_ptr.preview_plain(value_normalized as f32) as f64,
            _ => value_normalized,
        }
    }

    unsafe fn plain_param_to_normalized(&self, id: u32, plain_value: f64) -> f64 {
        match self.inner.param_by_hash.get(&id) {
            Some(param_ptr) => param_ptr.preview_normalized(plain_value as f32) as f64,
            _ => plain_value,
        }
    }

    unsafe fn get_param_normalized(&self, id: u32) -> f64 {
        match self.inner.param_by_hash.get(&id) {
            Some(param_ptr) => param_ptr.modulated_normalized_value() as f64,
            _ => 0.5,
        }
    }

    unsafe fn set_param_normalized(&self, id: u32, value: f64) -> tresult {
        // If the plugin is currently processing audio, then this parameter change will also be sent
        // to the process function
        if self.inner.is_processing.load(Ordering::SeqCst) {
            return kResultOk;
        }

        let sample_rate = self
            .inner
            .current_buffer_config
            .load()
            .map(|c| c.sample_rate);
        self.inner
            .set_normalized_value_by_hash(id, value as f32, sample_rate)
    }

    unsafe fn set_component_handler(
        &self,
        handler: SharedVstPtr<dyn vst3_sys::vst::IComponentHandler>,
    ) -> tresult {
        *self.inner.component_handler.borrow_mut() = handler.upgrade().map(VstPtr::from);

        kResultOk
    }

    unsafe fn create_view(&self, _name: vst3_sys::base::FIDString) -> *mut c_void {
        // Without specialization this is the least redundant way to check if the plugin has an
        // editor. The default implementation returns a None here.
        match self.inner.editor.borrow().as_ref() {
            Some(editor) => Box::into_raw(WrapperView::new(self.inner.clone(), editor.clone()))
                as *mut vst3_sys::c_void,
            None => ptr::null_mut(),
        }
    }
}

impl<P: Vst3Plugin> IAudioProcessor for Wrapper<P> {
    unsafe fn set_bus_arrangements(
        &self,
        inputs: *mut vst3_sys::vst::SpeakerArrangement,
        num_ins: i32,
        outputs: *mut vst3_sys::vst::SpeakerArrangement,
        num_outs: i32,
    ) -> tresult {
        check_null_ptr!(inputs, outputs);

        // HACK: Bitwig will not call the process function at all if the plugin does not have any
        //       audio IO, so we'll add a zero channel output to work around this if that is the
        //       case
        let no_main_audio_io = P::DEFAULT_INPUT_CHANNELS == 0 && P::DEFAULT_OUTPUT_CHANNELS == 0;

        // Why are these signed integers again?
        if num_ins < 0 || num_outs < 0 {
            return kInvalidArgument;
        }

        // Every auxiliary input or output needs to have the same number of channels. In order to
        // support plugins with no main IO but with auxiliary IO, we'll need to take that into
        // account when asserting this. If that's the case, then the first bus for that direction
        // will have been marked auxiliary.
        let aux_inputs_only = P::DEFAULT_INPUT_CHANNELS == 0 && P::DEFAULT_AUX_INPUTS.is_some();
        let num_input_channels = if aux_inputs_only || num_ins < 1 {
            0
        } else {
            (*inputs).count_ones()
        };

        let aux_input_start_idx = if aux_inputs_only { 0 } else { 1 };
        let num_aux_input_busses = (num_ins as u32).saturating_sub(aux_input_start_idx);
        let num_aux_input_channels = if num_aux_input_busses == 0 {
            0
        } else {
            (*inputs.offset(aux_input_start_idx as isize)).count_ones()
        };
        for i in 1..num_aux_input_busses {
            if (*inputs.offset((aux_input_start_idx + i) as isize)).count_ones()
                != num_aux_input_channels
            {
                nih_debug_assert_failure!("Mismatching auxiliary input bus channels set by host");
                return kResultFalse;
            }
        }

        let aux_outputs_only = P::DEFAULT_OUTPUT_CHANNELS == 0 && P::DEFAULT_AUX_OUTPUTS.is_some();
        let num_output_channels = if (aux_outputs_only && !no_main_audio_io) || num_outs < 1 {
            0
        } else {
            (*outputs).count_ones()
        };

        let aux_output_start_idx = if aux_outputs_only { 0 } else { 1 };
        let num_aux_output_busses = (num_outs as u32).saturating_sub(aux_output_start_idx);
        let num_aux_output_channels = if num_aux_output_busses == 0 {
            0
        } else {
            (*outputs.offset(aux_output_start_idx as isize)).count_ones()
        };
        for i in 1..num_aux_output_busses {
            if (*outputs.offset((aux_output_start_idx + i) as isize)).count_ones()
                != num_aux_output_channels
            {
                nih_debug_assert_failure!("Mismatching auxiliary output bus channels set by host");
                return kResultFalse;
            }
        }

        let proposed_config = BusConfig {
            num_input_channels,
            num_output_channels,
            aux_input_busses: AuxiliaryIOConfig {
                num_busses: num_aux_input_busses,
                num_channels: num_aux_input_channels,
            },
            aux_output_busses: AuxiliaryIOConfig {
                num_busses: num_aux_output_busses,
                num_channels: num_aux_output_channels,
            },
        };
        if self
            .inner
            .plugin
            .lock()
            .accepts_bus_config(&proposed_config)
        {
            self.inner.current_bus_config.store(proposed_config);

            kResultOk
        } else {
            kResultFalse
        }
    }

    unsafe fn get_bus_arrangement(
        &self,
        dir: vst3_sys::vst::BusDirection,
        index: i32,
        arr: *mut vst3_sys::vst::SpeakerArrangement,
    ) -> tresult {
        check_null_ptr!(arr);

        // HACK: Bitwig will not call the process function at all if the plugin does not have any
        //       audio IO, so we'll add a zero channel output to work around this if that is the
        //       case
        let no_main_audio_io = P::DEFAULT_INPUT_CHANNELS == 0 && P::DEFAULT_OUTPUT_CHANNELS == 0;

        let channel_count_to_map = |count| match count {
            0 => vst3_sys::vst::kEmpty,
            1 => vst3_sys::vst::kMono,
            2 => vst3_sys::vst::kStereo,
            5 => vst3_sys::vst::k50,
            6 => vst3_sys::vst::k51,
            7 => vst3_sys::vst::k70Cine,
            8 => vst3_sys::vst::k71Cine,
            n => {
                nih_debug_assert_failure!(
                    "No defined layout for {} channels, making something up on the spot...",
                    n
                );
                (1 << n) - 1
            }
        };

        let bus_config = self.inner.current_bus_config.load();
        let num_channels = if dir == vst3_sys::vst::BusDirections::kInput as i32 {
            let aux_inputs_only = P::DEFAULT_INPUT_CHANNELS == 0 && P::DEFAULT_AUX_INPUTS.is_some();
            let aux_input_start_idx = if aux_inputs_only { 0 } else { 1 };
            if !aux_inputs_only && index == 0 {
                bus_config.num_input_channels
            } else if (aux_input_start_idx
                ..(aux_input_start_idx + bus_config.aux_input_busses.num_busses as i32))
                .contains(&index)
            {
                bus_config.aux_input_busses.num_channels
            } else {
                return kInvalidArgument;
            }
        } else if dir == vst3_sys::vst::BusDirections::kOutput as i32 {
            let aux_outputs_only =
                P::DEFAULT_OUTPUT_CHANNELS == 0 && P::DEFAULT_AUX_OUTPUTS.is_some();
            let aux_output_start_idx = if aux_outputs_only { 0 } else { 1 };
            if (!aux_outputs_only || no_main_audio_io) && index == 0 {
                bus_config.num_output_channels
            } else if (aux_output_start_idx
                ..(aux_output_start_idx + bus_config.aux_output_busses.num_busses as i32))
                .contains(&index)
            {
                bus_config.aux_output_busses.num_channels
            } else {
                return kInvalidArgument;
            }
        } else {
            return kInvalidArgument;
        };
        let channel_map = channel_count_to_map(num_channels);

        nih_debug_assert_eq!(num_channels, channel_map.count_ones());
        *arr = channel_map;

        kResultOk
    }

    unsafe fn can_process_sample_size(&self, symbolic_sample_size: i32) -> tresult {
        if symbolic_sample_size == vst3_sys::vst::SymbolicSampleSizes::kSample32 as i32 {
            kResultOk
        } else {
            kResultFalse
        }
    }

    unsafe fn get_latency_samples(&self) -> u32 {
        self.inner.current_latency.load(Ordering::SeqCst)
    }

    unsafe fn setup_processing(&self, setup: *const vst3_sys::vst::ProcessSetup) -> tresult {
        check_null_ptr!(setup);

        // There's no special handling for offline processing at the moment
        let setup = &*setup;
        nih_debug_assert_eq!(
            setup.symbolic_sample_size,
            vst3_sys::vst::SymbolicSampleSizes::kSample32 as i32
        );

        // This is needed when activating the plugin and when restoring state
        self.inner.current_buffer_config.store(Some(BufferConfig {
            sample_rate: setup.sample_rate as f32,
            min_buffer_size: None,
            max_buffer_size: setup.max_samples_per_block as u32,
            process_mode: self.inner.current_process_mode.load(),
        }));

        let mode = match setup.process_mode {
            n if n == ProcessModes::kRealtime as i32 => ProcessMode::Realtime,
            n if n == ProcessModes::kPrefetch as i32 => ProcessMode::Buffered,
            n if n == ProcessModes::kOffline as i32 => ProcessMode::Offline,
            n => {
                nih_debug_assert_failure!("Unknown rendering mode '{}', defaulting to realtime", n);
                ProcessMode::Realtime
            }
        };
        self.inner.current_process_mode.store(mode);

        // Initializing the plugin happens in `IAudioProcessor::set_active()` because the host may
        // still change the channel layouts at this point

        kResultOk
    }

    unsafe fn set_processing(&self, state: TBool) -> tresult {
        let state = state != 0;

        // Always reset the processing status when the plugin gets activated or deactivated
        self.inner.last_process_status.store(ProcessStatus::Normal);
        self.inner.is_processing.store(state, Ordering::SeqCst);

        // This function is also used to reset buffers on the plugin, so we should do the same
        // thing. We don't call `reset()` in `setup_processing()` for that same reason.
        if state {
            // HACK: See the comment in `IComponent::setActive()`. This is needed to work around
            //       Ardour bugs.
            let mut plugin = match self.inner.plugin.try_lock() {
                Some(plugin) => plugin,
                None => {
                    nih_debug_assert_failure!(
                        "The host tried to call IAudioProcessor::setProcessing(true) during a \
                         reentrent call to IComponent::setActive(true), returning kResultOk. If \
                         this is Ardour then it will still call \
                         IAudioProcessor::setProcessing(true) later and everything will be fine. \
                         Hopefully."
                    );
                    return kResultOk;
                }
            };

            process_wrapper(|| plugin.reset());
        }

        // We don't have any special handling for suspending and resuming plugins, yet
        kResultOk
    }

    // Clippy doesn't understand our `event_start_idx`
    #[allow(clippy::mut_range_bound)]
    unsafe fn process(&self, data: *mut vst3_sys::vst::ProcessData) -> tresult {
        check_null_ptr!(data);

        // Panic on allocations if the `assert_process_allocs` feature has been enabled, and make
        // sure that FTZ is set up correctly
        process_wrapper(|| {
            // We need to handle incoming automation first
            let data = &*data;
            let sample_rate = self
                .inner
                .current_buffer_config
                .load()
                .expect("Process call without prior setup call")
                .sample_rate;

            nih_debug_assert!(data.num_inputs >= 0 && data.num_outputs >= 0);
            nih_debug_assert_eq!(
                data.symbolic_sample_size,
                vst3_sys::vst::SymbolicSampleSizes::kSample32 as i32
            );
            nih_debug_assert!(data.num_samples >= 0);

            let total_buffer_len = data.num_samples as usize;

            // Before doing anything, clear out any auxiliary outputs since they may contain
            // uninitialized data when the host assumes that we'll always write something there
            let current_bus_config = self.inner.current_bus_config.load();
            let has_main_input = current_bus_config.num_input_channels > 0;
            // HACK: Bitwig requires VST3 plugins to always have a main output. We'll however still
            //       use this variable here to maintain consistency between the backends.
            let has_main_output = true;
            if !data.outputs.is_null() {
                for output_idx in if has_main_output { 1 } else { 0 }..data.num_outputs as isize {
                    let host_output = data.outputs.offset(output_idx);
                    if !(*host_output).buffers.is_null() {
                        for channel_idx in 0..(*host_output).num_channels as isize {
                            ptr::write_bytes(
                                *((*host_output).buffers.offset(channel_idx)) as *mut f32,
                                0,
                                total_buffer_len,
                            );
                        }
                    }
                }
            }

            // If `P::SAMPLE_ACCURATE_AUTOMATION` is set, then we'll split up the audio buffer into
            // chunks whenever a parameter change occurs. To do that, we'll store all of those
            // parameter changes in a vector. Otherwise all parameter changes are handled right here
            // and now. We'll also need to store the note events in the same vector because MIDI CC
            // messages are sent through parameter changes. This vector gets sorted at the end so we
            // can treat it as a sort of queue.
            let mut process_events = self.inner.process_events.borrow_mut();
            process_events.clear();

            // First we'll go through the parameter changes. This may also include MIDI CC messages
            // if the plugin supports those
            if let Some(param_changes) = data.input_param_changes.upgrade() {
                let num_param_queues = param_changes.get_parameter_count();
                for change_queue_idx in 0..num_param_queues {
                    if let Some(param_change_queue) =
                        param_changes.get_parameter_data(change_queue_idx).upgrade()
                    {
                        let param_hash = param_change_queue.get_parameter_id();
                        let num_changes = param_change_queue.get_point_count();
                        if num_changes <= 0 {
                            continue;
                        }

                        let mut sample_offset = 0i32;
                        let mut value = 0.0f64;
                        for change_idx in 0..num_changes {
                            if param_change_queue.get_point(
                                change_idx,
                                &mut sample_offset,
                                &mut value,
                            ) == kResultOk
                            {
                                // Later this timing will be compensated for block splits by calling
                                // `event.subtract_timing(block_start)` before it is passed to the
                                // plugin. Out of bounds events are clamped to the buffer>
                                let timing = sample_offset as u32;
                                nih_debug_assert!(
                                    timing < total_buffer_len as u32,
                                    "Input event is out of bounds, will be clamped to the \
                                     buffer's size"
                                );
                                let timing = timing.min(total_buffer_len as u32 - 1);

                                let value = value as f32;

                                // MIDI CC messages, channel pressure, and pitch bend are also sent
                                // as parameter changes
                                if P::MIDI_INPUT >= MidiConfig::MidiCCs
                                    && (VST3_MIDI_PARAMS_START..VST3_MIDI_PARAMS_END)
                                        .contains(&param_hash)
                                {
                                    let midi_param_relative_idx =
                                        param_hash - VST3_MIDI_PARAMS_START;
                                    // This goes up to 130 for the 128 CCs followed by channel pressure and pitch bend
                                    let midi_cc = (midi_param_relative_idx % VST3_MIDI_CCS) as u8;
                                    let midi_channel =
                                        (midi_param_relative_idx / VST3_MIDI_CCS) as u8;
                                    process_events.push(ProcessEvent::NoteEvent(match midi_cc {
                                        // kAfterTouch
                                        128 => NoteEvent::MidiChannelPressure {
                                            timing,
                                            channel: midi_channel,
                                            pressure: value,
                                        },
                                        // kPitchBend
                                        129 => NoteEvent::MidiPitchBend {
                                            timing,
                                            channel: midi_channel,
                                            value,
                                        },
                                        n => NoteEvent::MidiCC {
                                            timing,
                                            channel: midi_channel,
                                            cc: n,
                                            value,
                                        },
                                    }));
                                } else if P::SAMPLE_ACCURATE_AUTOMATION {
                                    process_events.push(ProcessEvent::ParameterChange {
                                        timing,
                                        hash: param_hash,
                                        normalized_value: value,
                                    });
                                } else {
                                    self.inner.set_normalized_value_by_hash(
                                        param_hash,
                                        value,
                                        Some(sample_rate),
                                    );
                                }
                            }
                        }
                    }
                }
            }

            // Then we'll add all of our input events
            if P::MIDI_INPUT >= MidiConfig::Basic {
                let mut note_expression_controller =
                    self.inner.note_expression_controller.borrow_mut();
                if let Some(events) = data.input_events.upgrade() {
                    let num_events = events.get_event_count();

                    let mut event: MaybeUninit<_> = MaybeUninit::uninit();
                    for i in 0..num_events {
                        let result = events.get_event(i, event.as_mut_ptr());
                        nih_debug_assert_eq!(result, kResultOk);

                        let event = event.assume_init();
                        let timing = event.sample_offset as u32;
                        nih_debug_assert!(
                            timing < total_buffer_len as u32,
                            "Input event is out of bounds, will be clamped to the buffer's size"
                        );
                        let timing = timing.min(total_buffer_len as u32 - 1);

                        if event.type_ == EventTypes::kNoteOnEvent as u16 {
                            let event = event.event.note_on;

                            // We need to keep track of note IDs to be able to handle not
                            // expression value events
                            note_expression_controller.register_note(&event);

                            process_events.push(ProcessEvent::NoteEvent(NoteEvent::NoteOn {
                                timing,
                                voice_id: if event.note_id != -1 {
                                    Some(event.note_id)
                                } else {
                                    None
                                },
                                channel: event.channel as u8,
                                note: event.pitch as u8,
                                velocity: event.velocity,
                            }));
                        } else if event.type_ == EventTypes::kNoteOffEvent as u16 {
                            let event = event.event.note_off;
                            process_events.push(ProcessEvent::NoteEvent(NoteEvent::NoteOff {
                                timing,
                                voice_id: if event.note_id != -1 {
                                    Some(event.note_id)
                                } else {
                                    None
                                },
                                channel: event.channel as u8,
                                note: event.pitch as u8,
                                velocity: event.velocity,
                            }));
                        } else if event.type_ == EventTypes::kPolyPressureEvent as u16 {
                            let event = event.event.poly_pressure;
                            process_events.push(ProcessEvent::NoteEvent(NoteEvent::PolyPressure {
                                timing,
                                voice_id: if event.note_id != -1 {
                                    Some(event.note_id)
                                } else {
                                    None
                                },
                                channel: event.channel as u8,
                                note: event.pitch as u8,
                                pressure: event.pressure,
                            }));
                        } else if event.type_ == EventTypes::kNoteExpressionValueEvent as u16 {
                            let event = event.event.note_expression_value;
                            match note_expression_controller.translate_event(timing, &event) {
                                Some(translated_event) => {
                                    process_events.push(ProcessEvent::NoteEvent(translated_event))
                                }
                                None => nih_debug_assert_failure!(
                                    "Unhandled note expression type: {}",
                                    event.type_id
                                ),
                            }
                        } else if event.type_ == EventTypes::kDataEvent as u16
                            && event.event.data.type_ == 0
                        {
                            // 0 = kMidiSysEx
                            let event = event.event.data;

                            // `NoteEvent::from_midi` prints some tracing if parsing fails, which is
                            // not necessarily an error
                            assert!(!event.bytes.is_null());
                            let sysex_buffer =
                                std::slice::from_raw_parts(event.bytes, event.size as usize);
                            if let Ok(note_event) = NoteEvent::from_midi(timing, sysex_buffer) {
                                process_events.push(ProcessEvent::NoteEvent(note_event));
                            };
                        }
                    }
                }
            }

            // And then we'll make sure everything is in the right order
            // NOTE: It's important that this sort is stable, because parameter changes need to be
            //       processed before note events. Otherwise you'll get out of bounds note events
            //       with block splitting when the note event occurs at one index after the end (or
            //       on the exclusive end index) of the block.
            // FIXME: Apparently stable sort allcoates if the slice is large enough. This should be
            //        fixed at some point.
            permit_alloc(|| {
                process_events.sort_by_key(|event| match event {
                    ProcessEvent::ParameterChange { timing, .. } => *timing,
                    ProcessEvent::NoteEvent(event) => event.timing(),
                })
            });

            let mut block_start = 0usize;
            let mut block_end;
            let mut event_start_idx = 0;
            let result = loop {
                // In sample-accurate automation mode we'll handle all parameter changes from the
                // sorted process event array until we run into for the current sample, and then
                // process the block between the current sample and the sample containing the next
                // parameter change, if any. All timings also need to be compensated for this. As
                // mentioned above, for this to work correctly parameter changes need to be ordered
                // before note events at the same index.
                // The extra scope is here to make sure we release the borrow on input_events
                {
                    let mut input_events = self.inner.input_events.borrow_mut();
                    input_events.clear();

                    block_end = total_buffer_len;
                    for event_idx in event_start_idx..process_events.len() {
                        match &process_events[event_idx] {
                            ProcessEvent::ParameterChange {
                                timing,
                                hash,
                                normalized_value,
                            } => {
                                // If this parameter change happens after the start of this block, then
                                // we'll split the block here and handle this parameter change after
                                // we've processed this block
                                if *timing != block_start as u32 {
                                    event_start_idx = event_idx;
                                    block_end = *timing as usize;
                                    break;
                                }

                                self.inner.set_normalized_value_by_hash(
                                    *hash,
                                    *normalized_value,
                                    Some(sample_rate),
                                );
                            }
                            ProcessEvent::NoteEvent(event) => {
                                // We need to make sure to compensate the event for any block splitting,
                                // since we had to create the event object beforehand
                                let mut event = event.clone();
                                event.subtract_timing(block_start as u32);
                                input_events.push_back(event);
                            }
                        }
                    }
                }

                // This vector has been preallocated to contain enough slices as there are output
                // channels. In case the does does not provide an output or if they don't provide
                // all of the channels (this should not happen, but Ableton Live might do it) then
                // we'll skip the process function.
                let block_len = block_end - block_start;
                let mut output_buffer = self.inner.output_buffer.borrow_mut();
                let mut buffer_is_valid = false;
                output_buffer.set_slices(block_len, |output_slices| {
                    // Buffers for zero-channel plugins like note effects should always be allowed
                    buffer_is_valid = output_slices.is_empty();

                    if !data.outputs.is_null() {
                        let num_output_channels = (*data.outputs).num_channels as usize;
                        buffer_is_valid = num_output_channels == output_slices.len();
                        nih_debug_assert_eq!(num_output_channels, output_slices.len());

                        // In case the host does provide fewer output channels than we expect, we
                        // should still try to handle that gracefully. This happens when the plugin
                        // is bypassed in Ableton Live and a parameter is modified. In that case the
                        // above assertion will still trigger.
                        for (output_channel_idx, output_channel_slice) in output_slices
                            .iter_mut()
                            .take(num_output_channels)
                            .enumerate()
                        {
                            // If `P::SAMPLE_ACCURATE_AUTOMATION` is set, then we may be iterating
                            // over the buffer in smaller sections.
                            // SAFETY: These pointers may not be valid outside of this function even
                            // though their lifetime is equal to this structs. This is still safe
                            // because they are only dereferenced here later as part of this process
                            // function.
                            let channel_ptr =
                                *((*data.outputs).buffers as *mut *mut f32).add(output_channel_idx);
                            *output_channel_slice = std::slice::from_raw_parts_mut(
                                channel_ptr.add(block_start),
                                block_len,
                            );
                        }
                    }
                });

                // Some hosts process data in place, in which case we don't need to do any copying
                // ourselves. If the pointers do not alias, then we'll do the copy here and then the
                // plugin can just do normal in place processing.
                if !data.outputs.is_null() && !data.inputs.is_null() {
                    let num_output_channels = (*data.outputs).num_channels as usize;
                    let num_input_channels = (*data.inputs).num_channels as usize;
                    nih_debug_assert!(
                        num_input_channels <= num_output_channels,
                        "Stereo to mono and similar configurations are not supported"
                    );
                    for input_channel_idx in 0..cmp::min(num_input_channels, num_output_channels) {
                        let output_channel_ptr =
                            *((*data.outputs).buffers as *mut *mut f32).add(input_channel_idx);
                        let input_channel_ptr =
                            *((*data.inputs).buffers as *const *const f32).add(input_channel_idx);
                        if input_channel_ptr != output_channel_ptr {
                            ptr::copy_nonoverlapping(
                                input_channel_ptr.add(block_start),
                                output_channel_ptr.add(block_start),
                                block_len,
                            );
                        }
                    }
                }

                // We'll need to do the same thing for auxiliary input sidechain buffers. Since we
                // don't know whether overwriting the host's buffers is safe here or not, we'll copy
                // the data to our own buffers instead. These buffers are only accessible through
                // the `aux` parameter on the `process()` function.
                let mut aux_input_storage = self.inner.aux_input_storage.borrow_mut();
                let mut aux_input_buffers = self.inner.aux_input_buffers.borrow_mut();
                for (auxiliary_input_idx, (storage, buffer)) in aux_input_storage
                    .iter_mut()
                    .zip(aux_input_buffers.iter_mut())
                    .enumerate()
                {
                    let host_input_idx = if has_main_input {
                        auxiliary_input_idx as isize + 1
                    } else {
                        auxiliary_input_idx as isize
                    };
                    let host_input = data.inputs.offset(host_input_idx);
                    if host_input_idx >= data.num_inputs as isize
                             || data.inputs.is_null()
                             || (*host_input).buffers.is_null()
                             // Would only happen if the user configured zero channels for the
                             // auxiliary buffers
                             || storage.is_empty()
                             || (*host_input).num_channels != buffer.channels() as i32
                    {
                        nih_debug_assert!(host_input_idx < data.num_inputs as isize);
                        nih_debug_assert!(!data.inputs.is_null());
                        nih_debug_assert!(!(*host_input).buffers.is_null());
                        nih_debug_assert!(!storage.is_empty());
                        nih_debug_assert_eq!((*host_input).num_channels, buffer.channels() as i32);

                        // If the host passes weird data then we need to be very sure that there are
                        // no dangling references to previous data
                        buffer.set_slices(0, |slices| slices.fill_with(|| &mut []));
                        continue;
                    }

                    // We'll always reuse the start of the buffer even of the current block is
                    // shorter for cache locality reasons
                    for (channel_idx, channel_storage) in storage.iter_mut().enumerate() {
                        // The `set_len()` avoids having to unnecessarily fill the buffer with
                        // zeroes when sizing up
                        assert!(block_len <= channel_storage.capacity());
                        channel_storage.set_len(block_len);
                        channel_storage.copy_from_slice(std::slice::from_raw_parts(
                            (*(*host_input).buffers.add(channel_idx)).add(block_start)
                                as *const f32,
                            block_len,
                        ));
                    }

                    buffer.set_slices(block_len, |slices| {
                        for (channel_slice, channel_storage) in
                            slices.iter_mut().zip(storage.iter_mut())
                        {
                            // SAFETY: The 'static cast is required because Rust does not allow you
                            //         to store references to a field in another field.  Because
                            //         these slices are set here before the process function is
                            //         called, we ensure that there are no dangling slices. These
                            //         buffers/slices are only ever read from in the second part of
                            //         this block process loop.
                            *channel_slice = &mut *(channel_storage.as_mut_slice() as *mut [f32]);
                        }
                    });
                }

                // And the same thing for auxiliary output buffers
                let mut aux_output_buffers = self.inner.aux_output_buffers.borrow_mut();
                for (auxiliary_output_idx, buffer) in aux_output_buffers.iter_mut().enumerate() {
                    let host_output_idx = if has_main_output {
                        auxiliary_output_idx as isize + 1
                    } else {
                        auxiliary_output_idx as isize
                    };
                    let host_output = data.outputs.offset(host_output_idx);
                    if host_output_idx >= data.num_outputs as isize
                        || data.outputs.is_null()
                        || (*host_output).buffers.is_null()
                        || buffer.channels() == 0
                    {
                        nih_debug_assert!(host_output_idx < data.num_outputs as isize);
                        nih_debug_assert!(!data.outputs.is_null());
                        nih_debug_assert!(!(*host_output).buffers.is_null());

                        // If the host passes weird data then we need to be very sure that there are
                        // no dangling references to previous data
                        buffer.set_slices(0, |slices| slices.fill_with(|| &mut []));
                        continue;
                    }

                    buffer.set_slices(block_len, |slices| {
                        for (channel_idx, channel_slice) in slices.iter_mut().enumerate() {
                            *channel_slice = std::slice::from_raw_parts_mut(
                                (*(*host_output).buffers.add(channel_idx)).add(block_start)
                                    as *mut f32,
                                block_len,
                            );
                        }
                    });
                }

                // Some of the fields are left empty because VST3 does not provide this
                // information, but the methods on [`Transport`] can reconstruct these values
                // from the other fields
                let mut transport = Transport::new(sample_rate);
                if !data.context.is_null() {
                    let context = &*data.context;

                    // These constants are missing from vst3-sys, see:
                    // https://steinbergmedia.github.io/vst3_doc/vstinterfaces/structSteinberg_1_1Vst_1_1ProcessContext.html
                    transport.playing = context.state & (1 << 1) != 0; // kPlaying
                    transport.recording = context.state & (1 << 3) != 0; // kRecording
                    if context.state & (1 << 10) != 0 {
                        // kTempoValid
                        transport.tempo = Some(context.tempo);
                    }
                    if context.state & (1 << 13) != 0 {
                        // kTimeSigValid
                        transport.time_sig_numerator = Some(context.time_sig_num);
                        transport.time_sig_denominator = Some(context.time_sig_den);
                    }

                    // We need to compensate for the block splitting here
                    transport.pos_samples = Some(context.project_time_samples + block_start as i64);
                    if context.state & (1 << 9) != 0 {
                        // kProjectTimeMusicValid
                        if P::SAMPLE_ACCURATE_AUTOMATION
                            && block_start > 0
                            && (context.state & (1 << 10) != 0)
                        {
                            // kTempoValid
                            transport.pos_beats = Some(
                                context.project_time_music
                                    + (block_start as f64 / sample_rate as f64 / 60.0
                                        * context.tempo),
                            );
                        } else {
                            transport.pos_beats = Some(context.project_time_music);
                        }
                    }

                    if context.state & (1 << 11) != 0 {
                        // kBarPositionValid
                        if P::SAMPLE_ACCURATE_AUTOMATION && block_start > 0 {
                            // The transport object knows how to recompute this from the other information
                            transport.bar_start_pos_beats = match transport.bar_start_pos_beats() {
                                Some(updated) => Some(updated),
                                None => Some(context.bar_position_music),
                            };
                        } else {
                            transport.bar_start_pos_beats = Some(context.bar_position_music);
                        }
                    }
                    if context.state & (1 << 2) != 0 && context.state & (1 << 12) != 0 {
                        // kCycleActive && kCycleValid
                        transport.loop_range_beats =
                            Some((context.cycle_start_music, context.cycle_end_music));
                    }
                }

                let result = if buffer_is_valid {
                    // NOTE: `parking_lot`'s mutexes sometimes allocate because of their use of
                    //       thread locals
                    let mut plugin = permit_alloc(|| self.inner.plugin.lock());
                    // SAFETY: Shortening these borrows is safe as even if the plugin overwrites the
                    //         slices (which it cannot do without using unsafe code), then they
                    //         would still be reset on the next iteration
                    let mut aux = AuxiliaryBuffers {
                        inputs: &mut *(aux_input_buffers.as_mut_slice() as *mut [Buffer]),
                        outputs: &mut *(aux_output_buffers.as_mut_slice() as *mut [Buffer]),
                    };
                    let mut context = self.inner.make_process_context(transport);
                    let result = plugin.process(&mut output_buffer, &mut aux, &mut context);
                    self.inner.last_process_status.store(result);
                    result
                } else {
                    ProcessStatus::Normal
                };

                // Send any events output by the plugin during the process cycle
                if let Some(events) = data.output_events.upgrade() {
                    let mut output_events = self.inner.output_events.borrow_mut();
                    while let Some(event) = output_events.pop_front() {
                        // We'll set the correct variant on this struct, or skip to the next
                        // loop iteration if we don't handle the event type
                        let mut vst3_event: Event = mem::zeroed();
                        vst3_event.bus_index = 0;
                        // There's also a ppqPos field, but uh how about no
                        vst3_event.sample_offset = event.timing() as i32 + block_start as i32;

                        // Out of bounds events are clamped to the buffer
                        nih_debug_assert!(
                            vst3_event.sample_offset < total_buffer_len as i32,
                            "Output event is out of bounds, will be clamped to the buffer's size"
                        );
                        vst3_event.sample_offset =
                            vst3_event.sample_offset.min(total_buffer_len as i32 - 1);

                        // `voice_id.unwrap_or(|| ...)` triggers
                        // https://github.com/rust-lang/rust-clippy/issues/8522
                        #[allow(clippy::unnecessary_lazy_evaluations)]
                        match event {
                            NoteEvent::NoteOn {
                                timing: _,
                                voice_id,
                                channel,
                                note,
                                velocity,
                            } if P::MIDI_OUTPUT >= MidiConfig::Basic => {
                                vst3_event.type_ = EventTypes::kNoteOnEvent as u16;
                                vst3_event.event.note_on = NoteOnEvent {
                                    channel: channel as i16,
                                    pitch: note as i16,
                                    tuning: 0.0,
                                    velocity,
                                    length: 0, // What?
                                    // We'll use this for our note IDs, that way we don't have
                                    // to do anything complicated here
                                    note_id: voice_id
                                        .unwrap_or_else(|| ((channel as i32) << 8) | note as i32),
                                };
                            }
                            NoteEvent::NoteOff {
                                timing: _,
                                voice_id,
                                channel,
                                note,
                                velocity,
                            } if P::MIDI_OUTPUT >= MidiConfig::Basic => {
                                vst3_event.type_ = EventTypes::kNoteOffEvent as u16;
                                vst3_event.event.note_off = NoteOffEvent {
                                    channel: channel as i16,
                                    pitch: note as i16,
                                    velocity,
                                    note_id: voice_id
                                        .unwrap_or_else(|| ((channel as i32) << 8) | note as i32),
                                    tuning: 0.0,
                                };
                            }
                            // VST3 does not support or need these events, but they should also not
                            // trigger a debug assertion failure in NIH-plug. Also notes how this is
                            // gated by `P::MIDI_INPUT`.
                            NoteEvent::VoiceTerminated { .. }
                                if P::MIDI_INPUT >= MidiConfig::Basic =>
                            {
                                continue;
                            }
                            NoteEvent::PolyPressure {
                                timing: _,
                                voice_id,
                                channel,
                                note,
                                pressure,
                            } if P::MIDI_OUTPUT >= MidiConfig::Basic => {
                                vst3_event.type_ = EventTypes::kPolyPressureEvent as u16;
                                vst3_event.event.poly_pressure = PolyPressureEvent {
                                    channel: channel as i16,
                                    pitch: note as i16,
                                    note_id: voice_id
                                        .unwrap_or_else(|| ((channel as i32) << 8) | note as i32),
                                    pressure,
                                };
                            }
                            ref event @ (NoteEvent::PolyVolume {
                                voice_id,
                                channel,
                                note,
                                ..
                            }
                            | NoteEvent::PolyPan {
                                voice_id,
                                channel,
                                note,
                                ..
                            }
                            | NoteEvent::PolyTuning {
                                voice_id,
                                channel,
                                note,
                                ..
                            }
                            | NoteEvent::PolyVibrato {
                                voice_id,
                                channel,
                                note,
                                ..
                            }
                            | NoteEvent::PolyExpression {
                                voice_id,
                                channel,
                                note,
                                ..
                            }
                            | NoteEvent::PolyBrightness {
                                voice_id,
                                channel,
                                note,
                                ..
                            }) if P::MIDI_OUTPUT >= MidiConfig::Basic => {
                                match NoteExpressionController::translate_event_reverse(
                                    voice_id
                                        .unwrap_or_else(|| ((channel as i32) << 8) | note as i32),
                                    event,
                                ) {
                                    Some(translated_event) => {
                                        vst3_event.type_ =
                                            EventTypes::kNoteExpressionValueEvent as u16;
                                        vst3_event.event.note_expression_value = translated_event;
                                    }
                                    None => {
                                        nih_debug_assert_failure!(
                                            "Mishandled note expression value event"
                                        );
                                    }
                                }
                            }
                            NoteEvent::MidiChannelPressure {
                                timing: _,
                                channel,
                                pressure,
                            } if P::MIDI_OUTPUT >= MidiConfig::MidiCCs => {
                                vst3_event.type_ = EventTypes::kLegacyMIDICCOutEvent as u16;
                                vst3_event.event.legacy_midi_cc_out = LegacyMidiCCOutEvent {
                                    control_number: 128, // kAfterTouch
                                    channel: channel as i8,
                                    value: (pressure * 127.0).round() as i8,
                                    value2: 0,
                                };
                            }
                            NoteEvent::MidiPitchBend {
                                timing: _,
                                channel,
                                value,
                            } if P::MIDI_OUTPUT >= MidiConfig::MidiCCs => {
                                let scaled = (value * ((1 << 14) - 1) as f32).round() as i32;

                                vst3_event.type_ = EventTypes::kLegacyMIDICCOutEvent as u16;
                                vst3_event.event.legacy_midi_cc_out = LegacyMidiCCOutEvent {
                                    control_number: 129, // kPitchBend
                                    channel: channel as i8,
                                    value: (scaled & 0b01111111) as i8,
                                    value2: ((scaled >> 7) & 0b01111111) as i8,
                                };
                            }
                            NoteEvent::MidiCC {
                                timing: _,
                                channel,
                                cc,
                                value,
                            } if P::MIDI_OUTPUT >= MidiConfig::MidiCCs => {
                                vst3_event.type_ = EventTypes::kLegacyMIDICCOutEvent as u16;
                                vst3_event.event.legacy_midi_cc_out = LegacyMidiCCOutEvent {
                                    control_number: cc,
                                    channel: channel as i8,
                                    value: (value * 127.0).round() as i8,
                                    value2: 0,
                                };
                            }
                            NoteEvent::MidiProgramChange {
                                timing: _,
                                channel,
                                program,
                            } if P::MIDI_OUTPUT >= MidiConfig::MidiCCs => {
                                vst3_event.type_ = EventTypes::kLegacyMIDICCOutEvent as u16;
                                vst3_event.event.legacy_midi_cc_out = LegacyMidiCCOutEvent {
                                    control_number: 130, // kCtrlProgramChange
                                    channel: channel as i8,
                                    value: program as i8,
                                    value2: 0,
                                };
                            }
                            NoteEvent::MidiSysEx { timing: _, message }
                                if P::MIDI_OUTPUT >= MidiConfig::Basic =>
                            {
                                let (padded_sysex_buffer, length) = message.to_buffer();
                                let padded_sysex_buffer = padded_sysex_buffer.borrow();
                                nih_debug_assert!(padded_sysex_buffer.len() >= length);
                                let sysex_buffer = &padded_sysex_buffer[..length];

                                vst3_event.type_ = EventTypes::kDataEvent as u16;
                                vst3_event.event.data = DataEvent {
                                    size: sysex_buffer.len() as u32,
                                    type_: 0, // kMidiSysEx
                                    bytes: sysex_buffer.as_ptr(),
                                };

                                // NOTE: We need to have this call here while `sysex_buffer` is
                                //       still in scope since the event contains pointers to it
                                let result = events.add_event(&mut vst3_event);
                                nih_debug_assert_eq!(result, kResultOk);
                                continue;
                            }
                            _ => {
                                nih_debug_assert_failure!(
                                    "Invalid output event for the current MIDI_OUTPUT setting"
                                );
                                continue;
                            }
                        };

                        let result = events.add_event(&mut vst3_event);
                        nih_debug_assert_eq!(result, kResultOk);
                    }
                }

                let result = match result {
                    ProcessStatus::Error(err) => {
                        nih_debug_assert_failure!("Process error: {}", err);

                        return kResultFalse;
                    }
                    _ => kResultOk,
                };

                // If our block ends at the end of the buffer then that means there are no more
                // unprocessed (parameter) events. If there are more events, we'll just keep going
                // through this process until we've processed the entire buffer.
                if block_end == total_buffer_len {
                    break result;
                } else {
                    block_start = block_end;
                }
            };

            // After processing audio, we'll check if the editor has sent us updated plugin state.
            // We'll restore that here on the audio thread to prevent changing the values during the
            // process call and also to prevent inconsistent state when the host also wants to load
            // plugin state.
            // FIXME: Zero capacity channels allocate on receiving, find a better alternative that
            //        doesn't do that
            let updated_state = permit_alloc(|| self.inner.updated_state_receiver.try_recv());
            if let Ok(mut state) = updated_state {
                state::deserialize_object::<P>(
                    &mut state,
                    self.inner.params.clone(),
                    state::make_params_getter(
                        &self.inner.param_by_hash,
                        &self.inner.param_id_to_hash,
                    ),
                    self.inner.current_buffer_config.load().as_ref(),
                );

                // NOTE: This needs to be dropped after the `plugin` lock to avoid deadlocks
                let mut init_context = self.inner.make_init_context();
                let bus_config = self.inner.current_bus_config.load();
                let buffer_config = self.inner.current_buffer_config.load().unwrap();
                let mut plugin = self.inner.plugin.lock();
                // FIXME: This is obviously not realtime-safe, but loading presets without doing
                //         this could lead to inconsistencies. It's the plugin's responsibility to
                //         not perform any realtime-unsafe work when the initialize function is
                //         called a second time if it supports runtime preset loading.
                permit_alloc(|| plugin.initialize(&bus_config, &buffer_config, &mut init_context));
                plugin.reset();

                let task_posted = self.inner.schedule_gui(Task::ParameterValuesChanged);
                nih_debug_assert!(task_posted, "The task queue is full, dropping task...");

                // We'll pass the state object back to the GUI thread so deallocation can happen
                // there without potentially blocking the audio thread
                if let Err(err) = self.inner.updated_state_sender.send(state) {
                    nih_debug_assert_failure!(
                        "Failed to send state object back to GUI thread: {}",
                        err
                    );
                };
            }

            result
        })
    }

    unsafe fn get_tail_samples(&self) -> u32 {
        // https://github.com/steinbergmedia/vst3_pluginterfaces/blob/2ad397ade5b51007860bedb3b01b8afd2c5f6fba/vst/ivstaudioprocessor.h#L145-L159
        match self.inner.last_process_status.load() {
            ProcessStatus::Tail(samples) => samples,
            ProcessStatus::KeepAlive => u32::MAX, // kInfiniteTail
            _ => 0,                               // kNoTail
        }
    }
}

impl<P: Vst3Plugin> IMidiMapping for Wrapper<P> {
    unsafe fn get_midi_controller_assignment(
        &self,
        bus_index: i32,
        channel: i16,
        midi_cc_number: vst3_com::vst::CtrlNumber,
        param_id: *mut vst3_com::vst::ParamID,
    ) -> tresult {
        if P::MIDI_INPUT < MidiConfig::MidiCCs
            || bus_index != 0
            || !(0..VST3_MIDI_CHANNELS as i16).contains(&channel)
            || !(0..VST3_MIDI_CCS as i16).contains(&midi_cc_number)
        {
            return kResultFalse;
        }

        check_null_ptr!(param_id);

        // We reserve a contiguous parameter range right at the end of the allowed parameter indices
        // for these MIDI CC parameters
        *param_id =
            VST3_MIDI_PARAMS_START + midi_cc_number as u32 + (channel as u32 * VST3_MIDI_CCS);

        kResultOk
    }
}

impl<P: Vst3Plugin> INoteExpressionController for Wrapper<P> {
    unsafe fn get_note_expression_count(&self, bus_idx: i32, _channel: i16) -> i32 {
        // Apparently you need to define the predefined note expressions. Thanks VST3.
        if P::MIDI_INPUT >= MidiConfig::Basic && bus_idx == 0 {
            note_expressions::KNOWN_NOTE_EXPRESSIONS.len() as i32
        } else {
            0
        }
    }

    unsafe fn get_note_expression_info(
        &self,
        bus_idx: i32,
        _channel: i16,
        note_expression_idx: i32,
        info: *mut NoteExpressionTypeInfo,
    ) -> tresult {
        if P::MIDI_INPUT < MidiConfig::Basic
            || bus_idx != 0
            || !(0..note_expressions::KNOWN_NOTE_EXPRESSIONS.len() as i32)
                .contains(&note_expression_idx)
        {
            return kInvalidArgument;
        }

        check_null_ptr!(info);

        *info = mem::zeroed();

        let info = &mut *info;
        let note_expression_info =
            &note_expressions::KNOWN_NOTE_EXPRESSIONS[note_expression_idx as usize];
        info.type_id = note_expression_info.type_id;
        u16strlcpy(&mut info.title, note_expression_info.title);
        u16strlcpy(&mut info.short_title, note_expression_info.title);
        u16strlcpy(&mut info.units, note_expression_info.unit);
        info.unit_id = kNoParentUnitId;
        // This should not be needed since they're predefined, but then again you'd think you also
        // wouldn't need to define predefined note expressions now do you?
        info.value_desc = NoteExpressionValueDescription {
            default_value: 0.5,
            min: 0.0,
            max: 1.0,
            step_count: 0,
        };
        info.id = kNoParamId;
        info.flags = 1 << 2; // kIsAbsolute

        kResultOk
    }

    unsafe fn get_note_expression_string_by_value(
        &self,
        _bus_idx: i32,
        _channel: i16,
        _id: u32,
        _value: f64,
        _string: *mut TChar,
    ) -> tresult {
        kResultFalse
    }

    unsafe fn get_note_expression_value_by_string(
        &self,
        _bus_idx: i32,
        _channel: i16,
        _id: u32,
        _string: *const TChar,
        _value: *mut f64,
    ) -> tresult {
        kResultFalse
    }
}

impl<P: Vst3Plugin> IProcessContextRequirements for Wrapper<P> {
    unsafe fn get_process_context_requirements(&self) -> u32 {
        IProcessContextRequirementsFlags::kNeedProjectTimeMusic
            | IProcessContextRequirementsFlags::kNeedBarPositionMusic
            | IProcessContextRequirementsFlags::kNeedCycleMusic
            | IProcessContextRequirementsFlags::kNeedTimeSignature
            | IProcessContextRequirementsFlags::kNeedTempo
            | IProcessContextRequirementsFlags::kNeedTransportState
    }
}

impl<P: Vst3Plugin> IUnitInfo for Wrapper<P> {
    unsafe fn get_unit_count(&self) -> i32 {
        self.inner.param_units.len() as i32
    }

    unsafe fn get_unit_info(&self, unit_index: i32, info: *mut UnitInfo) -> tresult {
        check_null_ptr!(info);

        match self.inner.param_units.info(unit_index as usize) {
            Some((unit_id, unit_info)) => {
                *info = mem::zeroed();

                let info = &mut *info;
                info.id = unit_id;
                info.parent_unit_id = unit_info.parent_id;
                u16strlcpy(&mut info.name, &unit_info.name);
                info.program_list_id = kNoProgramListId;

                kResultOk
            }
            None => kInvalidArgument,
        }
    }

    unsafe fn get_program_list_count(&self) -> i32 {
        // TODO: Do we want program lists? Probably not, CLAP doesn't even support them.
        0
    }

    unsafe fn get_program_list_info(
        &self,
        _list_index: i32,
        _info: *mut ProgramListInfo,
    ) -> tresult {
        kInvalidArgument
    }

    unsafe fn get_program_name(
        &self,
        _list_id: i32,
        _program_index: i32,
        _name: *mut u16,
    ) -> tresult {
        kInvalidArgument
    }

    unsafe fn get_program_info(
        &self,
        _list_id: i32,
        _program_index: i32,
        _attribute_id: *const u8,
        _attribute_value: *mut u16,
    ) -> tresult {
        kInvalidArgument
    }

    unsafe fn has_program_pitch_names(&self, _id: i32, _index: i32) -> tresult {
        // TODO: Support note names once someone requests it
        kInvalidArgument
    }

    unsafe fn get_program_pitch_name(
        &self,
        _id: i32,
        _index: i32,
        _pitch: i16,
        _name: *mut u16,
    ) -> tresult {
        kInvalidArgument
    }

    unsafe fn get_selected_unit(&self) -> i32 {
        // No! Steinberg! I don't want any of this! I just want to group parameters!
        kRootUnitId
    }

    unsafe fn select_unit(&self, _id: i32) -> tresult {
        kResultFalse
    }

    unsafe fn get_unit_by_bus(
        &self,
        _type_: i32,
        _dir: i32,
        _bus_index: i32,
        _channel: i32,
        _unit_id: *mut i32,
    ) -> tresult {
        // Stahp it!
        kResultFalse
    }

    unsafe fn set_unit_program_data(
        &self,
        _list_or_unit: i32,
        _program_idx: i32,
        _data: SharedVstPtr<dyn IBStream>,
    ) -> tresult {
        kInvalidArgument
    }
}
