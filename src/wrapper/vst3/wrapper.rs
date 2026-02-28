use std::borrow::Borrow;
use std::ffi::c_void;
use std::mem::{self, MaybeUninit};
use std::num::NonZeroU32;
use std::ptr::NonNull;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use vst3::Steinberg::Vst::ProcessContext_::StatesAndFlags_::{
    kBarPositionValid, kCycleActive, kCycleValid, kPlaying, kProjectTimeMusicValid, kRecording,
    kTempoValid, kTimeSigValid,
};
use vst3::Steinberg::Vst::{
    kNoParamId, kNoParentUnitId, kNoProgramListId, kRootUnitId, BusDirection, CString, CtrlNumber,
    DataEvent, Event, Event_::EventTypes, Event_::EventTypes_, IAudioProcessor,
    IAudioProcessorTrait, IComponent, IComponentHandler, IComponentTrait, IEditController,
    IEditControllerTrait, IEventList, IEventListTrait, IMidiMapping, IMidiMappingTrait,
    INoteExpressionController, INoteExpressionControllerTrait, IParamValueQueue,
    IParamValueQueueTrait, IParameterChanges, IParameterChangesTrait, IProcessContextRequirements,
    IProcessContextRequirementsTrait, IProcessContextRequirements_, IUnitInfo, IUnitInfoTrait,
    IoMode, LegacyMIDICCOutEvent, MediaType, NoteExpressionTypeID, NoteExpressionTypeInfo,
    NoteExpressionValue, NoteExpressionValueDescription, NoteOffEvent, NoteOnEvent, ParamID,
    ParamValue, ParameterInfo, ParameterInfo_::ParameterFlags, ParameterInfo_::ParameterFlags_,
    PolyPressureEvent, ProcessData, ProcessModes, ProcessModes_, ProcessSetup, ProgramListID,
    ProgramListInfo, SpeakerArrangement, String128, TChar, UnitID, UnitInfo,
};
use vst3::Steinberg::{
    int16, int32, kInvalidArgument, kNoInterface, kResultFalse, kResultOk, tresult, uint32,
    FIDString, FUnknown, IBStream, IBStreamTrait, IPlugView, IPluginBase, IPluginBaseTrait, TBool,
    TUID,
};
use vst3::{Class, ComRef, ComWrapper};
use widestring::U16CStr;

use super::inner::{ProcessEvent, WrapperInner};
use super::note_expressions::{self, NoteExpressionController};
use super::util::{u16strlcpy, VST3_MIDI_CCS, VST3_MIDI_NUM_PARAMS, VST3_MIDI_PARAMS_START};
use super::util::{VST3_MIDI_CHANNELS, VST3_MIDI_PARAMS_END};
use super::view::WrapperView;
use crate::prelude::{
    AuxiliaryBuffers, BufferConfig, MidiConfig, NoteEvent, ParamFlags, ProcessMode, ProcessStatus,
    SysExMessage, Transport, Vst3Plugin,
};
use crate::util::permit_alloc;
use crate::wrapper::state;
use crate::wrapper::util::buffer_management::{BufferManager, ChannelPointers};
use crate::wrapper::util::{clamp_input_event_timing, clamp_output_event_timing, process_wrapper};

pub struct Wrapper<P: Vst3Plugin> {
    inner: Arc<WrapperInner<P>>,
}

impl<P: Vst3Plugin> Class for Wrapper<P> {
    type Interfaces = (
        IComponent,
        IEditController,
        IAudioProcessor,
        IMidiMapping,
        INoteExpressionController,
        IProcessContextRequirements,
        IUnitInfo,
    );
}

impl<P: Vst3Plugin> Wrapper<P> {
    pub fn new() -> Self {
        Self {
            inner: WrapperInner::new(),
        }
    }
}

impl<P: Vst3Plugin> Drop for Wrapper<P> {
    fn drop(&mut self) {
        nih_debug_assert_eq!(Arc::strong_count(&self.inner), 1);
    }
}

impl<P: Vst3Plugin> IPluginBaseTrait for Wrapper<P> {
    unsafe fn initialize(&self, _context: *mut FUnknown) -> tresult {
        // We currently don't need or allow any initialization logic
        kResultOk
    }

    unsafe fn terminate(&self) -> tresult {
        kResultOk
    }
}

impl<P: Vst3Plugin> IComponentTrait for Wrapper<P> {
    unsafe fn getControllerClassId(&self, _classId: *mut TUID) -> tresult {
        // We won't separate the edit controller to keep the implementation a bit smaller
        kNoInterface
    }

    unsafe fn setIoMode(&self, _mode: IoMode) -> tresult {
        // Not quite sure what the point of this is when the processing setup also receives similar
        // information
        kResultOk
    }

    unsafe fn getBusCount(
        &self,
        type_: vst3::Steinberg::Vst::MediaType,
        dir: vst3::Steinberg::Vst::BusDirection,
    ) -> int32 {
        let current_audio_io_layout = self.inner.current_audio_io_layout.load();

        // A plugin has a main input and output bus if the default number of channels is non-zero,
        // and a plugin can also have auxiliary input and output busses
        match type_ {
            x if x == vst3::Steinberg::Vst::MediaTypes_::kAudio as i32
                && dir == vst3::Steinberg::Vst::BusDirections_::kInput as i32 =>
            {
                let main_busses = if current_audio_io_layout.main_input_channels.is_some() {
                    1
                } else {
                    0
                };
                let aux_busses = current_audio_io_layout.aux_input_ports.len() as i32;

                main_busses + aux_busses
            }
            x if x == vst3::Steinberg::Vst::MediaTypes_::kAudio as i32
                && dir == vst3::Steinberg::Vst::BusDirections_::kOutput as i32 =>
            {
                let main_busses = if current_audio_io_layout.main_output_channels.is_some() {
                    1
                } else {
                    0
                };
                let aux_busses = current_audio_io_layout.aux_output_ports.len() as i32;

                main_busses + aux_busses
            }
            x if x == vst3::Steinberg::Vst::MediaTypes_::kEvent as i32
                && dir == vst3::Steinberg::Vst::BusDirections_::kInput as i32
                && P::MIDI_INPUT >= MidiConfig::Basic =>
            {
                1
            }
            x if x == vst3::Steinberg::Vst::MediaTypes_::kEvent as i32
                && dir == vst3::Steinberg::Vst::BusDirections_::kOutput as i32
                && P::MIDI_OUTPUT >= MidiConfig::Basic =>
            {
                1
            }
            _ => 0,
        }
    }

    unsafe fn getBusInfo(
        &self,
        type_: vst3::Steinberg::Vst::MediaType,
        dir: vst3::Steinberg::Vst::BusDirection,
        index: int32,
        info: *mut vst3::Steinberg::Vst::BusInfo,
    ) -> tresult {
        check_null_ptr!(info);

        let current_audio_io_layout = self.inner.current_audio_io_layout.load();

        match (type_, dir, index) {
            (t, d, _)
                if t == vst3::Steinberg::Vst::MediaTypes_::kAudio as i32
                    && d == vst3::Steinberg::Vst::BusDirections_::kInput as i32 =>
            {
                *info = mem::zeroed();

                let info = &mut *info;
                info.mediaType = vst3::Steinberg::Vst::MediaTypes_::kAudio as i32;
                info.direction = dir;
                info.flags = vst3::Steinberg::Vst::BusInfo_::BusFlags_::kDefaultActive as u32;

                let has_main_input = current_audio_io_layout.main_input_channels.is_some();
                let aux_input_start_idx = if has_main_input { 1 } else { 0 };
                let aux_input_idx = (index - aux_input_start_idx).max(0) as usize;
                if index == 0 && has_main_input {
                    info.busType = vst3::Steinberg::Vst::BusTypes_::kMain as i32;
                    info.channelCount =
                        current_audio_io_layout.main_input_channels.unwrap().get() as i32;
                    u16strlcpy(&mut info.name, &current_audio_io_layout.main_input_name());

                    kResultOk
                } else if aux_input_idx < current_audio_io_layout.aux_input_ports.len() {
                    info.busType = vst3::Steinberg::Vst::BusTypes_::kAux as i32;
                    info.channelCount =
                        current_audio_io_layout.aux_input_ports[aux_input_idx].get() as i32;
                    u16strlcpy(
                        &mut info.name,
                        &current_audio_io_layout
                            .aux_input_name(aux_input_idx)
                            .expect("Out of bounds auxiliary input port"),
                    );

                    kResultOk
                } else {
                    kInvalidArgument
                }
            }
            (t, d, _)
                if t == vst3::Steinberg::Vst::MediaTypes_::kAudio as i32
                    && d == vst3::Steinberg::Vst::BusDirections_::kOutput as i32 =>
            {
                *info = mem::zeroed();

                let info = &mut *info;
                info.mediaType = vst3::Steinberg::Vst::MediaTypes_::kAudio as i32;
                info.direction = dir;
                info.flags = vst3::Steinberg::Vst::BusInfo_::BusFlags_::kDefaultActive as u32;

                let has_main_output = current_audio_io_layout.main_output_channels.is_some();
                let aux_output_start_idx = if has_main_output { 1 } else { 0 };
                let aux_output_idx = (index - aux_output_start_idx).max(0) as usize;
                if index == 0 && has_main_output {
                    info.busType = vst3::Steinberg::Vst::BusTypes_::kMain as i32;
                    // NOTE: See above, this becomes a 0 channel output if the plugin doesn't have a
                    //       main output
                    info.channelCount = current_audio_io_layout
                        .main_output_channels
                        .map(NonZeroU32::get)
                        .unwrap_or_default() as i32;
                    u16strlcpy(&mut info.name, &current_audio_io_layout.main_output_name());

                    kResultOk
                } else if aux_output_idx < current_audio_io_layout.aux_output_ports.len() {
                    info.busType = vst3::Steinberg::Vst::BusTypes_::kAux as i32;
                    info.channelCount =
                        current_audio_io_layout.aux_output_ports[aux_output_idx].get() as i32;
                    u16strlcpy(
                        &mut info.name,
                        &current_audio_io_layout
                            .aux_output_name(aux_output_idx)
                            .expect("Out of bounds auxiliary output port"),
                    );

                    kResultOk
                } else {
                    kInvalidArgument
                }
            }
            (t, d, 0)
                if t == vst3::Steinberg::Vst::MediaTypes_::kEvent as i32
                    && d == vst3::Steinberg::Vst::BusDirections_::kInput as i32
                    && P::MIDI_INPUT >= MidiConfig::Basic =>
            {
                *info = mem::zeroed();

                let info = &mut *info;
                info.mediaType = vst3::Steinberg::Vst::MediaTypes_::kEvent as i32;
                info.direction = vst3::Steinberg::Vst::BusDirections_::kInput as i32;
                info.channelCount = 16;
                u16strlcpy(&mut info.name, "Note Input");
                info.busType = vst3::Steinberg::Vst::BusTypes_::kMain as i32;
                info.flags = vst3::Steinberg::Vst::BusInfo_::BusFlags_::kDefaultActive as u32;
                kResultOk
            }
            (t, d, 0)
                if t == vst3::Steinberg::Vst::MediaTypes_::kEvent as i32
                    && d == vst3::Steinberg::Vst::BusDirections_::kOutput as i32
                    && P::MIDI_OUTPUT >= MidiConfig::Basic =>
            {
                *info = mem::zeroed();

                let info = &mut *info;
                info.mediaType = vst3::Steinberg::Vst::MediaTypes_::kEvent as i32;
                info.direction = vst3::Steinberg::Vst::BusDirections_::kOutput as i32;
                info.channelCount = 16;
                u16strlcpy(&mut info.name, "Note Output");
                info.busType = vst3::Steinberg::Vst::BusTypes_::kMain as i32;
                info.flags = vst3::Steinberg::Vst::BusInfo_::BusFlags_::kDefaultActive as u32;
                kResultOk
            }
            _ => kInvalidArgument,
        }
    }

    unsafe fn getRoutingInfo(
        &self,
        in_info: *mut vst3::Steinberg::Vst::RoutingInfo,
        out_info: *mut vst3::Steinberg::Vst::RoutingInfo,
    ) -> tresult {
        check_null_ptr!(in_info, out_info);

        let current_audio_io_layout = self.inner.current_audio_io_layout.load();

        *out_info = mem::zeroed();

        let in_info = &*in_info;
        let out_info = &mut *out_info;
        match (in_info.mediaType, in_info.busIndex) {
            (t, 0)
                if t == vst3::Steinberg::Vst::MediaTypes_::kAudio as i32
                    // We only have an IO pair when the plugin has both a main input and a main output
                    && current_audio_io_layout.main_input_channels.is_some()
                    && current_audio_io_layout.main_output_channels.is_some() =>
            {
                out_info.mediaType = vst3::Steinberg::Vst::MediaTypes_::kAudio as i32;
                out_info.busIndex = in_info.busIndex;
                out_info.channel = in_info.channel;

                kResultOk
            }
            (t, 0)
                if t == vst3::Steinberg::Vst::MediaTypes_::kEvent as i32
                    && P::MIDI_INPUT >= MidiConfig::Basic
                    && P::MIDI_OUTPUT >= MidiConfig::Basic =>
            {
                out_info.mediaType = vst3::Steinberg::Vst::MediaTypes_::kEvent as i32;
                out_info.busIndex = in_info.busIndex;
                out_info.channel = in_info.channel;

                kResultOk
            }
            _ => kResultFalse,
        }
    }

    unsafe fn activateBus(
        &self,
        type_: vst3::Steinberg::Vst::MediaType,
        dir: vst3::Steinberg::Vst::BusDirection,
        index: int32,
        _state: TBool,
    ) -> tresult {
        let current_audio_io_layout = self.inner.current_audio_io_layout.load();

        // We don't support this, but the validator will get very angry with us if we let it know
        // that
        match (type_, dir, index) {
            (t, d, _)
                if t == vst3::Steinberg::Vst::MediaTypes_::kAudio as i32
                    && d == vst3::Steinberg::Vst::BusDirections_::kInput as i32 =>
            {
                let main_busses = if current_audio_io_layout.main_input_channels.is_some() {
                    1
                } else {
                    0
                };
                let aux_busses = current_audio_io_layout.aux_input_ports.len() as i32;

                if (0..main_busses + aux_busses).contains(&index) {
                    kResultOk
                } else {
                    kInvalidArgument
                }
            }
            (t, d, _)
                if t == vst3::Steinberg::Vst::MediaTypes_::kAudio as i32
                    && d == vst3::Steinberg::Vst::BusDirections_::kOutput as i32 =>
            {
                let main_busses = if current_audio_io_layout.main_output_channels.is_some() {
                    1
                } else {
                    0
                };
                let aux_busses = current_audio_io_layout.aux_output_ports.len() as i32;

                if (0..main_busses + aux_busses).contains(&index) {
                    kResultOk
                } else {
                    kInvalidArgument
                }
            }
            (t, d, 0)
                if t == vst3::Steinberg::Vst::MediaTypes_::kEvent as i32
                    && d == vst3::Steinberg::Vst::BusDirections_::kInput as i32
                    && P::MIDI_INPUT >= MidiConfig::Basic =>
            {
                kResultOk
            }
            (t, d, 0)
                if t == vst3::Steinberg::Vst::MediaTypes_::kEvent as i32
                    && d == vst3::Steinberg::Vst::BusDirections_::kOutput as i32
                    && P::MIDI_OUTPUT >= MidiConfig::Basic =>
            {
                kResultOk
            }
            _ => kInvalidArgument,
        }
    }

    unsafe fn setActive(&self, state: TBool) -> tresult {
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
                let audio_io_layout = self.inner.current_audio_io_layout.load();
                let mut plugin = self.inner.plugin.lock();
                if plugin.initialize(&audio_io_layout, &buffer_config, &mut init_context) {
                    // NOTE: We don't call `Plugin::reset()` here. The call is done in `set_process()`
                    //       instead. Otherwise we would call the function twice, and `set_process()` needs
                    //       to be called after this function before the plugin may process audio again.

                    // This preallocates enough space so we can transform all of the host's raw
                    // channel pointers into a set of `Buffer` objects for the plugin's main and
                    // auxiliary IO
                    *self.inner.buffer_manager.borrow_mut() = BufferManager::for_audio_io_layout(
                        buffer_config.max_buffer_size as usize,
                        audio_io_layout,
                    );

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

    unsafe fn setState(&self, state: *mut IBStream) -> tresult {
        use vst3::Steinberg::IBStream_::IStreamSeekMode_::*;

        check_null_ptr!(state);

        let state = ComRef::from_raw(state).unwrap();

        // We need to know how large the state is before we can read it. The current position can be
        // zero, but it can also be something else. Bitwig prepends the preset header in the stream,
        // while some other hosts don't expose that to the plugin.
        let mut current_pos = 0;
        let mut eof_pos = 0;
        if state.tell(&mut current_pos) != kResultOk
            || state.seek(0, kIBSeekEnd as int32, &mut eof_pos) != kResultOk
            || state.seek(current_pos, kIBSeekSet as int32, std::ptr::null_mut()) != kResultOk
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

        match state::deserialize_json(&read_buffer) {
            Some(mut state) => {
                if self.inner.set_state_inner(&mut state) {
                    nih_trace!("Loaded state ({} bytes)", read_buffer.len());
                    kResultOk
                } else {
                    kResultFalse
                }
            }
            None => kResultFalse,
        }
    }

    unsafe fn getState(&self, state: *mut IBStream) -> tresult {
        check_null_ptr!(state);

        let state = ComRef::from_raw(state).unwrap();

        let serialized = state::serialize_json::<P>(
            self.inner.params.clone(),
            state::make_params_iter(&self.inner.param_by_hash, &self.inner.param_id_to_hash),
        );
        match serialized {
            Ok(serialized) => {
                let mut num_bytes_written = 0;
                let result = state.write(
                    serialized.as_ptr() as *mut c_void,
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

impl<P: Vst3Plugin> IEditControllerTrait for Wrapper<P> {
    unsafe fn setComponentState(&self, _state: *mut IBStream) -> tresult {
        // We have a single file component, so we don't need to do anything here
        kResultOk
    }

    unsafe fn setState(&self, _state: *mut IBStream) -> tresult {
        // We don't store any separate state here. The plugin's state will have been restored
        // through the component. Calling that same function here will likely lead to duplicate
        // state restores
        kResultOk
    }

    unsafe fn getState(&self, _state: *mut IBStream) -> tresult {
        // Same for this function
        kResultOk
    }

    unsafe fn getParameterCount(&self) -> int32 {
        // We need to add a whole bunch of parameters if the plugin accepts MIDI CCs
        if P::MIDI_INPUT >= MidiConfig::MidiCCs {
            self.inner.param_hashes.len() as i32 + VST3_MIDI_NUM_PARAMS as i32
        } else {
            self.inner.param_hashes.len() as i32
        }
    }

    unsafe fn getParameterInfo(&self, param_index: int32, info: *mut ParameterInfo) -> tresult {
        check_null_ptr!(info);

        if param_index < 0 || param_index > self.getParameterCount() {
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
            u16strlcpy(&mut info.shortTitle, &name);
            info.flags = ParameterFlags_::kIsReadOnly as i32 | (1 << 4); // kIsHidden
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
            u16strlcpy(&mut info.shortTitle, param_ptr.name());
            u16strlcpy(&mut info.units, param_ptr.unit());
            info.stepCount = param_ptr.step_count().unwrap_or(0) as i32;
            info.defaultNormalizedValue = default_value as f64;
            info.unitId = *param_unit;
            info.flags = 0;
            if automatable && !hidden {
                info.flags |= ParameterFlags_::kCanAutomate as i32;
            }
            if hidden {
                info.flags |= ParameterFlags_::kIsReadOnly as i32 | (1 << 4); // kIsHidden
            }
            if is_bypass {
                info.flags |= ParameterFlags_::kIsBypass as i32;
            }
        }

        kResultOk
    }

    unsafe fn getParamStringByValue(
        &self,
        id: ParamID,
        value_normalized: ParamValue,
        string: *mut String128,
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

    unsafe fn getParamValueByString(
        &self,
        id: ParamID,
        string: *mut TChar,
        value_normalized: *mut ParamValue,
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

    unsafe fn normalizedParamToPlain(
        &self,
        id: ParamID,
        value_normalized: ParamValue,
    ) -> ParamValue {
        match self.inner.param_by_hash.get(&id) {
            Some(param_ptr) => param_ptr.preview_plain(value_normalized as f32) as f64,
            _ => value_normalized,
        }
    }

    unsafe fn plainParamToNormalized(&self, id: ParamID, plain_value: ParamValue) -> ParamValue {
        match self.inner.param_by_hash.get(&id) {
            Some(param_ptr) => param_ptr.preview_normalized(plain_value as f32) as f64,
            _ => plain_value,
        }
    }

    unsafe fn getParamNormalized(&self, id: ParamID) -> ParamValue {
        match self.inner.param_by_hash.get(&id) {
            Some(param_ptr) => param_ptr.modulated_normalized_value() as f64,
            _ => 0.5,
        }
    }

    unsafe fn setParamNormalized(&self, id: ParamID, value: ParamValue) -> tresult {
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

    unsafe fn setComponentHandler(&self, handler: *mut IComponentHandler) -> tresult {
        *self.inner.component_handler.borrow_mut() =
            ComRef::from_raw(handler).map(|r| r.to_com_ptr());

        kResultOk
    }

    unsafe fn createView(&self, _name: FIDString) -> *mut IPlugView {
        // Without specialization this is the least redundant way to check if the plugin has an
        // editor. The default implementation returns a None here.
        match self.inner.editor.borrow().as_ref() {
            Some(editor) => {
                let view = ComWrapper::new(WrapperView::new(self.inner.clone(), editor.clone()));
                view.to_com_ptr::<IPlugView>().unwrap().into_raw()
            }
            None => std::ptr::null_mut(),
        }
    }
}

impl<P: Vst3Plugin> IAudioProcessorTrait for Wrapper<P> {
    unsafe fn setBusArrangements(
        &self,
        inputs: *mut SpeakerArrangement,
        num_ins: int32,
        outputs: *mut SpeakerArrangement,
        num_outs: int32,
    ) -> tresult {
        check_null_ptr!(inputs, outputs);

        // Why are these signed integers again?
        if num_ins < 0 || num_outs < 0 {
            return kInvalidArgument;
        }

        // NIH-plug no longer supports flexible IO layouts. Instead we'll try to find an audio IO
        // layout that matches the host's requested layout.
        let matching_layout = P::AUDIO_IO_LAYOUTS
            .iter()
            .find(|layout| {
                // If the number of ports/busses doesn't match then we can immediately discard the
                // layout. VST3 doesn't allow for optional switchable ports like CLAP does. Only the
                // channel counts can change.
                let num_layout_ins = if layout.main_input_channels.is_some() {
                    1
                } else {
                    0
                } + layout.aux_input_ports.len();
                let num_layout_outs = if layout.main_output_channels.is_some() {
                    1
                } else {
                    0
                } + layout.aux_output_ports.len();
                if num_ins as usize != num_layout_ins || num_outs as usize != num_layout_outs {
                    return false;
                }

                // NOTE: We completely ignore the speaker arrangements and only look at the channel
                //       counts here. This may cause issues at some point, but it works for now.
                let has_main_input = layout.main_input_channels.is_some();
                let aux_input_start_idx = if has_main_input { 0 } else { 1 };
                if has_main_input
                    && (*inputs).count_ones() != layout.main_input_channels.unwrap().get()
                {
                    return false;
                }
                for (aux_input_idx, channel_count) in layout.aux_input_ports.iter().enumerate() {
                    if (*inputs.add(aux_input_idx + aux_input_start_idx)).count_ones()
                        != channel_count.get()
                    {
                        return false;
                    }
                }

                let has_main_output = layout.main_output_channels.is_some();
                let aux_output_start_idx = if has_main_output { 0 } else { 1 };
                if (*outputs).count_ones()
                    != layout
                        .main_output_channels
                        .map(NonZeroU32::get)
                        .unwrap_or_default()
                {
                    return false;
                }
                for (aux_output_idx, channel_count) in layout.aux_output_ports.iter().enumerate() {
                    if (*outputs.add(aux_output_idx + aux_output_start_idx)).count_ones()
                        != channel_count.get()
                    {
                        return false;
                    }
                }

                true
            })
            .copied();

        match matching_layout {
            Some(layout) => {
                // This layout is used from hereon onwards, at least until this function is called
                // again
                self.inner.current_audio_io_layout.store(layout);

                kResultOk
            }
            None => kResultFalse,
        }
    }

    unsafe fn getBusArrangement(
        &self,
        dir: BusDirection,
        index: i32,
        arr: *mut SpeakerArrangement,
    ) -> tresult {
        check_null_ptr!(arr);

        let channel_count_to_map = |count| match count {
            0 => vst3::Steinberg::Vst::SpeakerArr::kEmpty,
            1 => vst3::Steinberg::Vst::SpeakerArr::kMono,
            2 => vst3::Steinberg::Vst::SpeakerArr::kStereo,
            5 => vst3::Steinberg::Vst::SpeakerArr::k50,
            6 => vst3::Steinberg::Vst::SpeakerArr::k51,
            7 => vst3::Steinberg::Vst::SpeakerArr::k70Cine,
            8 => vst3::Steinberg::Vst::SpeakerArr::k71Cine,
            n => {
                nih_debug_assert_failure!(
                    "No defined layout for {} channels, making something up on the spot...",
                    n
                );
                (1 << n) - 1
            }
        };

        let current_audio_io_layout = self.inner.current_audio_io_layout.load();
        let num_channels = if dir == vst3::Steinberg::Vst::BusDirections_::kInput as i32 {
            let has_main_input = current_audio_io_layout.main_input_channels.is_some();
            let aux_input_start_idx = if has_main_input { 1 } else { 0 };
            let aux_input_idx = (index - aux_input_start_idx).max(0) as usize;
            if index == 0 && has_main_input {
                current_audio_io_layout.main_input_channels.unwrap().get()
            } else if aux_input_idx < current_audio_io_layout.aux_input_ports.len() {
                current_audio_io_layout.aux_input_ports[aux_input_idx].get()
            } else {
                return kInvalidArgument;
            }
        } else if dir == vst3::Steinberg::Vst::BusDirections_::kOutput as i32 {
            let has_main_output = current_audio_io_layout.main_output_channels.is_some();
            let aux_output_start_idx = if has_main_output { 1 } else { 0 };
            let aux_output_idx = (index - aux_output_start_idx).max(0) as usize;
            if index == 0 && has_main_output {
                current_audio_io_layout.main_output_channels.unwrap().get()
            } else if aux_output_idx < current_audio_io_layout.aux_output_ports.len() {
                current_audio_io_layout.aux_output_ports[aux_output_idx].get()
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

    unsafe fn canProcessSampleSize(&self, symbolic_sample_size: int32) -> tresult {
        if symbolic_sample_size == vst3::Steinberg::Vst::SymbolicSampleSizes_::kSample32 as i32 {
            kResultOk
        } else {
            kResultFalse
        }
    }

    unsafe fn getLatencySamples(&self) -> uint32 {
        self.inner.current_latency.load(Ordering::SeqCst)
    }

    unsafe fn setupProcessing(&self, setup: *mut ProcessSetup) -> tresult {
        check_null_ptr!(setup);

        // There's no special handling for offline processing at the moment
        let setup = &*setup;
        nih_debug_assert_eq!(
            setup.symbolicSampleSize,
            vst3::Steinberg::Vst::SymbolicSampleSizes_::kSample32 as i32
        );

        // This is needed when activating the plugin and when restoring state
        self.inner.current_buffer_config.store(Some(BufferConfig {
            sample_rate: setup.sampleRate as f32,
            min_buffer_size: None,
            max_buffer_size: setup.maxSamplesPerBlock as u32,
            process_mode: self.inner.current_process_mode.load(),
        }));

        let mode = match setup.processMode {
            n if n == ProcessModes_::kRealtime as i32 => ProcessMode::Realtime,
            n if n == ProcessModes_::kPrefetch as i32 => ProcessMode::Buffered,
            n if n == ProcessModes_::kOffline as i32 => ProcessMode::Offline,
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

    unsafe fn setProcessing(&self, state: TBool) -> tresult {
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
    unsafe fn process(&self, data: *mut ProcessData) -> tresult {
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

            nih_debug_assert!(data.numInputs >= 0 && data.numOutputs >= 0);
            nih_debug_assert_eq!(
                data.symbolicSampleSize,
                vst3::Steinberg::Vst::SymbolicSampleSizes_::kSample32 as i32
            );
            nih_debug_assert!(data.numSamples >= 0);

            let total_buffer_len = data.numSamples as usize;

            let current_audio_io_layout = self.inner.current_audio_io_layout.load();
            let has_main_input = current_audio_io_layout.main_input_channels.is_some();
            let has_main_output = current_audio_io_layout.main_output_channels.is_some();
            let aux_input_start_idx = if has_main_input { 1 } else { 0 };
            let aux_output_start_idx = if has_main_output { 1 } else { 0 };

            // NOTE: VST3 hosts may trigger a 'parameter flush' by calling the process function for
            //       0 input samples. If this is the case then we'll only handle events and skip all
            //       audio processing. Some hosts, like Ableton Live, implement this in a broken way
            //       and instead only set the number of channels to 0. In that case the
            //       'buffer_is_valid' check from below should still prevent audio processing.
            let mut is_param_flush = total_buffer_len == 0;
            if (data.numOutputs == 0 || data.outputs.is_null())
                && (has_main_output || !current_audio_io_layout.aux_output_ports.is_empty())
            {
                is_param_flush = true;
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
            if let Some(param_changes) = ComRef::from_raw(data.inputParameterChanges) {
                let num_param_queues = param_changes.getParameterCount();
                for change_queue_idx in 0..num_param_queues {
                    if let Some(param_change_queue) =
                        ComRef::from_raw(param_changes.getParameterData(change_queue_idx))
                    {
                        let param_hash = param_change_queue.getParameterId();
                        let num_changes = param_change_queue.getPointCount();
                        if num_changes <= 0 {
                            continue;
                        }

                        let mut sample_offset = 0i32;
                        let mut value = 0.0f64;
                        for change_idx in 0..num_changes {
                            if param_change_queue.getPoint(
                                change_idx,
                                &mut sample_offset,
                                &mut value,
                            ) == kResultOk
                            {
                                // Later this timing will be compensated for block splits by calling
                                // `event.subtract_timing(block_start)` before it is passed to the
                                // plugin. Out of bounds events are clamped to the buffer>
                                let timing = clamp_input_event_timing(
                                    sample_offset as u32,
                                    total_buffer_len as u32,
                                );
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
                if let Some(events) = ComRef::from_raw(data.inputEvents) {
                    let num_events = events.getEventCount();

                    let mut event: MaybeUninit<_> = MaybeUninit::uninit();
                    for i in 0..num_events {
                        let result = events.getEvent(i, event.as_mut_ptr());
                        nih_debug_assert_eq!(result, kResultOk);

                        let event = event.assume_init();
                        let timing = clamp_input_event_timing(
                            event.sampleOffset as u32,
                            total_buffer_len as u32,
                        );

                        if event.r#type == EventTypes_::kNoteOnEvent as u16 {
                            let event = event.__field0.noteOn;

                            // We need to keep track of note IDs to be able to handle not
                            // expression value events
                            note_expression_controller.register_note(&event);

                            process_events.push(ProcessEvent::NoteEvent(NoteEvent::NoteOn {
                                timing,
                                voice_id: if event.noteId != -1 {
                                    Some(event.noteId)
                                } else {
                                    None
                                },
                                channel: event.channel as u8,
                                note: event.pitch as u8,
                                velocity: event.velocity,
                            }));
                        } else if event.r#type == EventTypes_::kNoteOffEvent as u16 {
                            let event = event.__field0.noteOff;
                            process_events.push(ProcessEvent::NoteEvent(NoteEvent::NoteOff {
                                timing,
                                voice_id: if event.noteId != -1 {
                                    Some(event.noteId)
                                } else {
                                    None
                                },
                                channel: event.channel as u8,
                                note: event.pitch as u8,
                                velocity: event.velocity,
                            }));
                        } else if event.r#type == EventTypes_::kPolyPressureEvent as u16 {
                            let event = event.__field0.polyPressure;
                            process_events.push(ProcessEvent::NoteEvent(NoteEvent::PolyPressure {
                                timing,
                                voice_id: if event.noteId != -1 {
                                    Some(event.noteId)
                                } else {
                                    None
                                },
                                channel: event.channel as u8,
                                note: event.pitch as u8,
                                pressure: event.pressure,
                            }));
                        } else if event.r#type == EventTypes_::kNoteExpressionValueEvent as u16 {
                            let event = event.__field0.noteExpressionValue;
                            match note_expression_controller.translate_event(timing, &event) {
                                Some(translated_event) => {
                                    process_events.push(ProcessEvent::NoteEvent(translated_event))
                                }
                                None => nih_debug_assert_failure!(
                                    "Unhandled note expression type: {}",
                                    event.typeId
                                ),
                            }
                        } else if event.r#type == EventTypes_::kDataEvent as u16
                            && event.__field0.data.r#type == 0
                        {
                            // 0 = kMidiSysEx
                            let event = event.__field0.data;

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

                let result = if is_param_flush {
                    kResultOk
                } else {
                    // After processing the events we now know where/if the block should be split,
                    // and we can start preparing audio processing
                    let block_len = block_end - block_start;

                    // The buffer manager preallocated buffer slices for all the IO and storage for
                    // any axuiliary inputs.
                    let mut buffer_manager = self.inner.buffer_manager.borrow_mut();
                    let buffers =
                        buffer_manager.create_buffers(block_start, block_len, |buffer_source| {
                            if data.numOutputs > 0
                                && !data.outputs.is_null()
                                && !(*data.outputs).__field0.channelBuffers32.is_null()
                                && has_main_output
                            {
                                let audio_output = &*data.outputs;
                                let ptrs = NonNull::new(
                                    audio_output.__field0.channelBuffers32 as *mut *mut f32,
                                )
                                .unwrap();
                                let num_channels = audio_output.numChannels as usize;

                                *buffer_source.main_output_channel_pointers =
                                    Some(ChannelPointers { ptrs, num_channels });
                            }

                            if data.numInputs > 0
                                && !data.inputs.is_null()
                                && !(*data.inputs).__field0.channelBuffers32.is_null()
                                && has_main_input
                            {
                                let audio_input = &*data.inputs;
                                let ptrs = NonNull::new(
                                    audio_input.__field0.channelBuffers32 as *mut *mut f32,
                                )
                                .unwrap();
                                let num_channels = audio_input.numChannels as usize;

                                *buffer_source.main_input_channel_pointers =
                                    Some(ChannelPointers { ptrs, num_channels });
                            }

                            if !data.inputs.is_null() {
                                for (aux_input_no, aux_input_channel_pointers) in buffer_source
                                    .aux_input_channel_pointers
                                    .iter_mut()
                                    .enumerate()
                                {
                                    let aux_input_idx = aux_input_no + aux_input_start_idx;
                                    if aux_input_idx > data.numOutputs as usize {
                                        break;
                                    }

                                    let audio_input = &*data.inputs.add(aux_input_idx);
                                    match NonNull::new(
                                        audio_input.__field0.channelBuffers32 as *mut *mut f32,
                                    ) {
                                        Some(ptrs) => {
                                            let num_channels = audio_input.numChannels as usize;

                                            *aux_input_channel_pointers =
                                                Some(ChannelPointers { ptrs, num_channels });
                                        }
                                        None => continue,
                                    }
                                }
                            }

                            if !data.outputs.is_null() {
                                for (aux_output_no, aux_output_channel_pointers) in buffer_source
                                    .aux_output_channel_pointers
                                    .iter_mut()
                                    .enumerate()
                                {
                                    let aux_output_idx = aux_output_no + aux_output_start_idx;
                                    if aux_output_idx > data.numOutputs as usize {
                                        break;
                                    }

                                    let audio_output = &*data.outputs.add(aux_output_idx);
                                    match NonNull::new(
                                        audio_output.__field0.channelBuffers32 as *mut *mut f32,
                                    ) {
                                        Some(ptrs) => {
                                            let num_channels = audio_output.numChannels as usize;

                                            *aux_output_channel_pointers =
                                                Some(ChannelPointers { ptrs, num_channels });
                                        }
                                        None => continue,
                                    }
                                }
                            }
                        });

                    // We already checked whether the host has initiated a parameter flush, but in
                    // case it still did something unexpected that we did not catch we'll still try
                    // to prevent processing audio when the slices don't contain the values we
                    // expect.
                    let mut buffer_is_valid = true;
                    for output_buffer_slice in
                        buffers.main_buffer.as_slice_immutable().iter().chain(
                            buffers
                                .aux_outputs
                                .iter()
                                .flat_map(|buffer| buffer.as_slice_immutable().iter()),
                        )
                    {
                        if output_buffer_slice.is_empty() {
                            buffer_is_valid = false;
                            break;
                        }
                    }
                    nih_debug_assert!(buffer_is_valid);

                    // Some of the fields are left empty because VST3 does not provide this
                    // information, but the methods on [`Transport`] can reconstruct these values
                    // from the other fields
                    let mut transport = Transport::new(sample_rate);
                    if !data.processContext.is_null() {
                        let context = &*data.processContext;

                        transport.playing = context.state & kPlaying != 0;
                        transport.recording = context.state & kRecording != 0;
                        if context.state & kTempoValid != 0 {
                            transport.tempo = Some(context.tempo);
                        }
                        if context.state & kTimeSigValid != 0 {
                            transport.time_sig_numerator = Some(context.timeSigNumerator);
                            transport.time_sig_denominator = Some(context.timeSigDenominator);
                        }

                        // We need to compensate for the block splitting here
                        transport.pos_samples =
                            Some(context.projectTimeSamples + block_start as i64);
                        if context.state & kProjectTimeMusicValid != 0 {
                            if P::SAMPLE_ACCURATE_AUTOMATION
                                && block_start > 0
                                && (context.state & kTempoValid != 0)
                            {
                                transport.pos_beats = Some(
                                    context.projectTimeMusic
                                        + (block_start as f64 / sample_rate as f64 / 60.0
                                            * context.tempo),
                                );
                            } else {
                                transport.pos_beats = Some(context.projectTimeMusic);
                            }
                        }

                        if context.state & kBarPositionValid != 0 {
                            if P::SAMPLE_ACCURATE_AUTOMATION && block_start > 0 {
                                // The transport object knows how to recompute this from the other information
                                transport.bar_start_pos_beats =
                                    match transport.bar_start_pos_beats() {
                                        Some(updated) => Some(updated),
                                        None => Some(context.barPositionMusic),
                                    };
                            } else {
                                transport.bar_start_pos_beats = Some(context.barPositionMusic);
                            }
                        }
                        if context.state & kCycleActive != 0 && context.state & kCycleValid != 0 {
                            transport.loop_range_beats =
                                Some((context.cycleStartMusic, context.cycleEndMusic));
                        }
                    }

                    let result = if buffer_is_valid {
                        // NOTE: `parking_lot`'s mutexes sometimes allocate because of their use of
                        //       thread locals
                        let mut plugin = permit_alloc(|| self.inner.plugin.lock());
                        let mut aux = AuxiliaryBuffers {
                            inputs: buffers.aux_inputs,
                            outputs: buffers.aux_outputs,
                        };
                        let mut context = self.inner.make_process_context(transport);
                        let result = plugin.process(buffers.main_buffer, &mut aux, &mut context);
                        self.inner.last_process_status.store(result);
                        result
                    } else {
                        ProcessStatus::Normal
                    };

                    match result {
                        ProcessStatus::Error(err) => {
                            nih_debug_assert_failure!("Process error: {}", err);

                            return kResultFalse;
                        }
                        _ => kResultOk,
                    }
                };

                // Send any events output by the plugin during the process cycle
                if let Some(events) = ComRef::from_raw(data.outputEvents) {
                    let mut output_events = self.inner.output_events.borrow_mut();
                    while let Some(event) = output_events.pop_front() {
                        // We'll set the correct variant on this struct, or skip to the next loop
                        // iteration if we don't handle the event type
                        let mut vst3_event: Event = mem::zeroed();
                        vst3_event.busIndex = 0;
                        // There's also a ppqPos field, but uh how about no
                        vst3_event.sampleOffset = clamp_output_event_timing(
                            event.timing() + block_start as u32,
                            total_buffer_len as u32,
                        ) as i32;

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
                                vst3_event.r#type = EventTypes_::kNoteOnEvent as u16;
                                vst3_event.__field0.noteOn = NoteOnEvent {
                                    channel: channel as i16,
                                    pitch: note as i16,
                                    tuning: 0.0,
                                    velocity,
                                    length: 0, // What?
                                    // We'll use this for our note IDs, that way we don't have to do
                                    // anything complicated here
                                    noteId: voice_id
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
                                vst3_event.r#type = EventTypes_::kNoteOffEvent as u16;
                                vst3_event.__field0.noteOff = NoteOffEvent {
                                    channel: channel as i16,
                                    pitch: note as i16,
                                    velocity,
                                    noteId: voice_id
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
                                vst3_event.r#type = EventTypes_::kPolyPressureEvent as u16;
                                vst3_event.__field0.polyPressure = PolyPressureEvent {
                                    channel: channel as i16,
                                    pitch: note as i16,
                                    noteId: voice_id
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
                                        vst3_event.r#type =
                                            EventTypes_::kNoteExpressionValueEvent as u16;
                                        vst3_event.__field0.noteExpressionValue = translated_event;
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
                                vst3_event.r#type = EventTypes_::kLegacyMIDICCOutEvent as u16;
                                vst3_event.__field0.midiCCOut = LegacyMIDICCOutEvent {
                                    controlNumber: 128, // kAfterTouch
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

                                vst3_event.r#type = EventTypes_::kLegacyMIDICCOutEvent as u16;
                                vst3_event.__field0.midiCCOut = LegacyMIDICCOutEvent {
                                    controlNumber: 129, // kPitchBend
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
                                vst3_event.r#type = EventTypes_::kLegacyMIDICCOutEvent as u16;
                                vst3_event.__field0.midiCCOut = LegacyMIDICCOutEvent {
                                    controlNumber: cc,
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
                                vst3_event.r#type = EventTypes_::kLegacyMIDICCOutEvent as u16;
                                vst3_event.__field0.midiCCOut = LegacyMIDICCOutEvent {
                                    controlNumber: 130, // kCtrlProgramChange
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

                                vst3_event.r#type = EventTypes_::kDataEvent as u16;
                                vst3_event.__field0.data = DataEvent {
                                    size: sysex_buffer.len() as u32,
                                    r#type: 0, // kMidiSysEx
                                    bytes: sysex_buffer.as_ptr(),
                                };

                                // NOTE: We need to have this call here while `sysex_buffer` is
                                //       still in scope since the event contains pointers to it
                                let result = events.addEvent(&mut vst3_event);
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

                        let result = events.addEvent(&mut vst3_event);
                        nih_debug_assert_eq!(result, kResultOk);
                    }
                }

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
                self.inner.set_state_inner(&mut state);

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

    unsafe fn getTailSamples(&self) -> uint32 {
        // https://github.com/steinbergmedia/vst3_pluginterfaces/blob/2ad397ade5b51007860bedb3b01b8afd2c5f6fba/vst/ivstaudioprocessor.h#L145-L159
        match self.inner.last_process_status.load() {
            ProcessStatus::Tail(samples) => samples,
            ProcessStatus::KeepAlive => u32::MAX, // kInfiniteTail
            _ => 0,                               // kNoTail
        }
    }
}

impl<P: Vst3Plugin> IMidiMappingTrait for Wrapper<P> {
    unsafe fn getMidiControllerAssignment(
        &self,
        bus_index: int32,
        channel: int16,
        midi_cc_number: CtrlNumber,
        param_id: *mut ParamID,
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

impl<P: Vst3Plugin> INoteExpressionControllerTrait for Wrapper<P> {
    unsafe fn getNoteExpressionCount(&self, bus_idx: int32, _channel: int16) -> int32 {
        // Apparently you need to define the predefined note expressions. Thanks VST3.
        if P::MIDI_INPUT >= MidiConfig::Basic && bus_idx == 0 {
            note_expressions::KNOWN_NOTE_EXPRESSIONS.len() as i32
        } else {
            0
        }
    }

    unsafe fn getNoteExpressionInfo(
        &self,
        bus_idx: int32,
        _channel: int16,
        note_expression_idx: int32,
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
        info.typeId = note_expression_info.type_id;
        u16strlcpy(&mut info.title, note_expression_info.title);
        u16strlcpy(&mut info.shortTitle, note_expression_info.title);
        u16strlcpy(&mut info.units, note_expression_info.unit);
        info.unitId = kNoParentUnitId;
        // This should not be needed since they're predefined, but then again you'd think you also
        // wouldn't need to define predefined note expressions now do you?
        info.valueDesc = NoteExpressionValueDescription {
            defaultValue: 0.5,
            minimum: 0.0,
            maximum: 1.0,
            stepCount: 0,
        };
        info.associatedParameterId = kNoParamId;
        info.flags = 1 << 2; // kIsAbsolute

        kResultOk
    }

    unsafe fn getNoteExpressionStringByValue(
        &self,
        _bus_idx: int32,
        _channel: int16,
        _id: NoteExpressionTypeID,
        _value: NoteExpressionValue,
        _string: *mut String128,
    ) -> tresult {
        kResultFalse
    }

    unsafe fn getNoteExpressionValueByString(
        &self,
        _bus_idx: int32,
        _channel: int16,
        _id: NoteExpressionTypeID,
        _string: *const TChar,
        _value: *mut NoteExpressionValue,
    ) -> tresult {
        kResultFalse
    }
}

impl<P: Vst3Plugin> IProcessContextRequirementsTrait for Wrapper<P> {
    unsafe fn getProcessContextRequirements(&self) -> uint32 {
        IProcessContextRequirements_::Flags_::kNeedProjectTimeMusic
            | IProcessContextRequirements_::Flags_::kNeedBarPositionMusic
            | IProcessContextRequirements_::Flags_::kNeedCycleMusic
            | IProcessContextRequirements_::Flags_::kNeedTimeSignature
            | IProcessContextRequirements_::Flags_::kNeedTempo
            | IProcessContextRequirements_::Flags_::kNeedTransportState
    }
}

impl<P: Vst3Plugin> IUnitInfoTrait for Wrapper<P> {
    unsafe fn getUnitCount(&self) -> int32 {
        self.inner.param_units.len() as i32
    }

    unsafe fn getUnitInfo(&self, unit_index: int32, info: *mut UnitInfo) -> tresult {
        check_null_ptr!(info);

        match self.inner.param_units.info(unit_index as usize) {
            Some((unit_id, unit_info)) => {
                *info = mem::zeroed();

                let info = &mut *info;
                info.id = unit_id;
                info.parentUnitId = unit_info.parent_id;
                u16strlcpy(&mut info.name, &unit_info.name);
                info.programListId = kNoProgramListId;

                kResultOk
            }
            None => kInvalidArgument,
        }
    }

    unsafe fn getProgramListCount(&self) -> int32 {
        // TODO: Do we want program lists? Probably not, CLAP doesn't even support them.
        0
    }

    unsafe fn getProgramListInfo(
        &self,
        _list_index: int32,
        _info: *mut ProgramListInfo,
    ) -> tresult {
        kInvalidArgument
    }

    unsafe fn getProgramName(
        &self,
        _list_id: ProgramListID,
        _program_index: int32,
        _name: *mut String128,
    ) -> tresult {
        kInvalidArgument
    }

    unsafe fn getProgramInfo(
        &self,
        _list_id: ProgramListID,
        _program_index: int32,
        _attribute_id: CString,
        _attribute_value: *mut String128,
    ) -> tresult {
        kInvalidArgument
    }

    unsafe fn hasProgramPitchNames(&self, _id: ProgramListID, _index: int32) -> tresult {
        // TODO: Support note names once someone requests it
        kInvalidArgument
    }

    unsafe fn getProgramPitchName(
        &self,
        _id: ProgramListID,
        _index: int32,
        _pitch: int16,
        _name: *mut String128,
    ) -> tresult {
        kInvalidArgument
    }

    unsafe fn getSelectedUnit(&self) -> UnitID {
        // No! Steinberg! I don't want any of this! I just want to group parameters!
        kRootUnitId
    }

    unsafe fn selectUnit(&self, _id: UnitID) -> tresult {
        kResultFalse
    }

    unsafe fn getUnitByBus(
        &self,
        _type_: MediaType,
        _dir: BusDirection,
        _bus_index: int32,
        _channel: int32,
        _unit_id: *mut UnitID,
    ) -> tresult {
        // Stahp it!
        kResultFalse
    }

    unsafe fn setUnitProgramData(
        &self,
        _list_or_unit: int32,
        _program_idx: int32,
        _data: *mut IBStream,
    ) -> tresult {
        kInvalidArgument
    }
}
