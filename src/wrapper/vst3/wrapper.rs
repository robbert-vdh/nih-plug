use std::cmp::{self, Reverse};
use std::ffi::c_void;
use std::mem::{self, MaybeUninit};
use std::ptr;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use vst3_sys::base::{kInvalidArgument, kNoInterface, kResultFalse, kResultOk, tresult, TBool};
use vst3_sys::base::{IBStream, IPluginBase};
use vst3_sys::utils::SharedVstPtr;
use vst3_sys::vst::{
    kNoProgramListId, kRootUnitId, IAudioProcessor, IComponent, IEditController, IEventList,
    IParamValueQueue, IParameterChanges, IUnitInfo, ProgramListInfo, TChar, UnitInfo,
};
use vst3_sys::VST3;
use widestring::U16CStr;

use super::inner::WrapperInner;
use super::util::VstPtr;
use super::view::WrapperView;
use crate::context::Transport;
use crate::param::ParamFlags;
use crate::plugin::{BufferConfig, BusConfig, NoteEvent, ProcessStatus, Vst3Plugin};
use crate::wrapper::state;
use crate::wrapper::util::{process_wrapper, u16strlcpy};
use crate::wrapper::vst3::inner::ParameterChange;

// Alias needed for the VST3 attribute macro
use vst3_sys as vst3_com;

#[VST3(implements(IComponent, IEditController, IAudioProcessor, IUnitInfo))]
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
        // We won't separate the edit controller to keep the implemetnation a bit smaller
        kNoInterface
    }

    unsafe fn set_io_mode(&self, _mode: vst3_sys::vst::IoMode) -> tresult {
        // This would need to integrate with the GUI, which we currently don't have
        kResultOk
    }

    unsafe fn get_bus_count(
        &self,
        type_: vst3_sys::vst::MediaType,
        dir: vst3_sys::vst::BusDirection,
    ) -> i32 {
        // All plugins currently only have a single input and a single output bus
        match type_ {
            x if x == vst3_sys::vst::MediaTypes::kAudio as i32 => 1,
            x if x == vst3_sys::vst::MediaTypes::kEvent as i32
                && dir == vst3_sys::vst::BusDirections::kInput as i32
                && P::ACCEPTS_MIDI =>
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

        match (type_, dir, index) {
            (t, _, _) if t == vst3_sys::vst::MediaTypes::kAudio as i32 => {
                *info = mem::zeroed();

                let info = &mut *info;
                info.media_type = vst3_sys::vst::MediaTypes::kAudio as i32;
                info.bus_type = vst3_sys::vst::BusTypes::kMain as i32;
                info.flags = vst3_sys::vst::BusFlags::kDefaultActive as u32;
                match (dir, index) {
                    (d, 0) if d == vst3_sys::vst::BusDirections::kInput as i32 => {
                        info.direction = vst3_sys::vst::BusDirections::kInput as i32;
                        info.channel_count =
                            self.inner.current_bus_config.load().num_input_channels as i32;
                        u16strlcpy(&mut info.name, "Input");

                        kResultOk
                    }
                    (d, 0) if d == vst3_sys::vst::BusDirections::kOutput as i32 => {
                        info.direction = vst3_sys::vst::BusDirections::kOutput as i32;
                        info.channel_count =
                            self.inner.current_bus_config.load().num_output_channels as i32;
                        u16strlcpy(&mut info.name, "Output");

                        kResultOk
                    }
                    _ => kInvalidArgument,
                }
            }
            (t, d, 0)
                if t == vst3_sys::vst::MediaTypes::kEvent as i32
                    && d == vst3_sys::vst::BusDirections::kInput as i32
                    && P::ACCEPTS_MIDI =>
            {
                *info = mem::zeroed();

                let info = &mut *info;
                info.media_type = vst3_sys::vst::MediaTypes::kEvent as i32;
                info.direction = vst3_sys::vst::BusDirections::kInput as i32;
                info.channel_count = 16;
                u16strlcpy(&mut info.name, "MIDI");
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
            (t, 0) if t == vst3_sys::vst::MediaTypes::kAudio as i32 => {
                out_info.media_type = vst3_sys::vst::MediaTypes::kAudio as i32;
                out_info.bus_index = in_info.bus_index;
                out_info.channel = in_info.channel;

                kResultOk
            }
            _ => kInvalidArgument,
        }
    }

    unsafe fn activate_bus(
        &self,
        type_: vst3_sys::vst::MediaType,
        dir: vst3_sys::vst::BusDirection,
        index: i32,
        _state: vst3_sys::base::TBool,
    ) -> tresult {
        // We don't need any special handling here
        match (type_, dir, index) {
            (t, _, 0) if t == vst3_sys::vst::MediaTypes::kAudio as i32 => kResultOk,
            (t, d, 0)
                if t == vst3_sys::vst::MediaTypes::kEvent as i32
                    && d == vst3_sys::vst::BusDirections::kInput as i32
                    && P::ACCEPTS_MIDI =>
            {
                kResultOk
            }
            _ => kInvalidArgument,
        }
    }

    unsafe fn set_active(&self, _state: TBool) -> tresult {
        // We don't need any special handling here
        kResultOk
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

        let success = state::deserialize(
            &read_buffer,
            self.inner.params.clone(),
            &self.inner.param_by_hash,
            &self.inner.param_id_to_hash,
            self.inner.current_buffer_config.load().as_ref(),
        );
        if !success {
            return kResultFalse;
        }

        // Reinitialize the plugin after loading state so it can respond to the new parameter values
        self.inner.notify_param_values_changed();

        let bus_config = self.inner.current_bus_config.load();
        if let Some(buffer_config) = self.inner.current_buffer_config.load() {
            let mut plugin = self.inner.plugin.write();
            plugin.initialize(
                &bus_config,
                &buffer_config,
                &mut self
                    .inner
                    .make_process_context(Transport::new(buffer_config.sample_rate)),
            );
            process_wrapper(|| plugin.reset());
        }

        kResultOk
    }

    unsafe fn get_state(&self, state: SharedVstPtr<dyn IBStream>) -> tresult {
        check_null_ptr!(state);

        let state = state.upgrade().unwrap();

        let serialized = state::serialize(
            self.inner.params.clone(),
            &self.inner.param_by_hash,
            &self.inner.param_id_to_hash,
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
                kResultOk
            }
            Err(err) => {
                nih_debug_assert_failure!("Could not save state: {}", err);
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
        self.inner.param_hashes.len() as i32
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
        let is_bypass = flags.contains(ParamFlags::BYPASS);

        *info = std::mem::zeroed();

        let info = &mut *info;
        info.id = *param_hash;
        u16strlcpy(&mut info.title, param_ptr.name());
        u16strlcpy(&mut info.short_title, param_ptr.name());
        u16strlcpy(&mut info.units, param_ptr.unit());
        info.step_count = param_ptr.step_count().unwrap_or(0) as i32;
        info.default_normalized_value = default_value as f64;
        info.unit_id = *param_unit;
        info.flags = if automatable {
            vst3_sys::vst::ParameterFlags::kCanAutomate as i32
        } else {
            vst3_sys::vst::ParameterFlags::kIsReadOnly as i32 | (1 << 4) // kIsHidden
        };
        if is_bypass {
            info.flags |= vst3_sys::vst::ParameterFlags::kIsBypass as i32;
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
            _ => 0.5,
        }
    }

    unsafe fn plain_param_to_normalized(&self, id: u32, plain_value: f64) -> f64 {
        match self.inner.param_by_hash.get(&id) {
            Some(param_ptr) => param_ptr.preview_normalized(plain_value as f32) as f64,
            _ => 0.5,
        }
    }

    unsafe fn get_param_normalized(&self, id: u32) -> f64 {
        match self.inner.param_by_hash.get(&id) {
            Some(param_ptr) => param_ptr.normalized_value() as f64,
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
        let result = self
            .inner
            .set_normalized_value_by_hash(id, value as f32, sample_rate);
        self.inner.notify_param_values_changed();

        result
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
        match &self.inner.editor {
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

        // We currently only do single audio bus IO configurations
        if num_ins != 1 || num_outs != 1 {
            return kInvalidArgument;
        }

        let input_channel_map = &*inputs;
        let output_channel_map = &*outputs;
        let proposed_config = BusConfig {
            num_input_channels: input_channel_map.count_ones(),
            num_output_channels: output_channel_map.count_ones(),
        };
        if self
            .inner
            .plugin
            .read()
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

        let config = self.inner.current_bus_config.load();
        let num_channels = match (dir, index) {
            (d, 0) if d == vst3_sys::vst::BusDirections::kInput as i32 => config.num_input_channels,
            (d, 0) if d == vst3_sys::vst::BusDirections::kOutput as i32 => {
                config.num_output_channels
            }
            _ => return kInvalidArgument,
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

        let bus_config = self.inner.current_bus_config.load();
        let buffer_config = BufferConfig {
            sample_rate: setup.sample_rate as f32,
            max_buffer_size: setup.max_samples_per_block as u32,
        };

        // Befure initializing the plugin, make sure all smoothers are set the the default values
        for param in self.inner.param_by_hash.values() {
            param.update_smoother(buffer_config.sample_rate, true);
        }

        let mut plugin = self.inner.plugin.write();
        if plugin.initialize(
            &bus_config,
            &buffer_config,
            &mut self
                .inner
                .make_process_context(Transport::new(buffer_config.sample_rate)),
        ) {
            // As per-the trait docs we'll always call this after the initialization function
            process_wrapper(|| plugin.reset());

            // Preallocate enough room in the output slices vector so we can convert a `*mut *mut
            // f32` to a `&mut [&mut f32]` in the process call
            self.inner
                .output_buffer
                .borrow_mut()
                .with_raw_vec(|output_slices| {
                    output_slices.resize_with(bus_config.num_output_channels as usize, || &mut [])
                });

            // Also store this for later, so we can reinitialize the plugin after restoring state
            self.inner.current_buffer_config.store(Some(buffer_config));

            kResultOk
        } else {
            kResultFalse
        }
    }

    unsafe fn set_processing(&self, state: TBool) -> tresult {
        // Always reset the processing status when the plugin gets activated or deactivated
        self.inner.last_process_status.store(ProcessStatus::Normal);
        self.inner.is_processing.store(state != 0, Ordering::SeqCst);

        // We don't have any special handling for suspending and resuming plugins, yet
        kResultOk
    }

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

            // It's possible the host only wanted to send new parameter values
            let is_parameter_flush = data.num_outputs == 0;
            if is_parameter_flush {
                nih_log!("VST3 parameter flush");
            } else {
                check_null_ptr_msg!(
                    "Process output pointer is null",
                    data.outputs,
                    (*data.outputs).buffers,
                );
            }

            // The setups we suppport are:
            // - 1 input bus
            // - 1 output bus
            // - 1 input bus and 1 output bus
            nih_debug_assert!(
                data.num_inputs >= 0
                    && data.num_inputs <= 1
                    && data.num_outputs >= 0
                    && data.num_outputs <= 1,
                "The host provides more than one input or output bus"
            );
            nih_debug_assert_eq!(
                data.symbolic_sample_size,
                vst3_sys::vst::SymbolicSampleSizes::kSample32 as i32
            );
            nih_debug_assert!(data.num_samples >= 0);

            // If `P::SAMPLE_ACCURATE_AUTOMATION` is set, then we'll split up the audio buffer into
            // chunks whenever a parameter change occurs. Otherwise all parameter changes are
            // handled right here and now.
            let mut input_param_changes = self.inner.input_param_changes.borrow_mut();
            let mut parameter_values_changed = false;
            input_param_changes.clear();
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
                        #[allow(clippy::collapsible_else_if)]
                        if P::SAMPLE_ACCURATE_AUTOMATION {
                            for change_idx in 0..num_changes {
                                if param_change_queue.get_point(
                                    change_idx,
                                    &mut sample_offset,
                                    &mut value,
                                ) == kResultOk
                                {
                                    input_param_changes.push(Reverse((
                                        sample_offset as usize,
                                        ParameterChange {
                                            hash: param_hash,
                                            normalized_value: value as f32,
                                        },
                                    )));
                                }
                            }
                        } else {
                            if param_change_queue.get_point(
                                num_changes - 1,
                                &mut sample_offset,
                                &mut value,
                            ) == kResultOk
                            {
                                self.inner.set_normalized_value_by_hash(
                                    param_hash,
                                    value as f32,
                                    Some(sample_rate),
                                );
                                parameter_values_changed = true;
                            }
                        }
                    }
                }
            }

            let mut block_start = 0;
            let mut block_end = data.num_samples as usize;
            let mut event_start_idx = 0;
            loop {
                // In sample-accurate automation mode we'll handle any parameter changes for the
                // current sample, and then process the block between the current sample and the
                // sample containing the next parameter change, if any. All timings also need to be
                // compensated for this.
                if P::SAMPLE_ACCURATE_AUTOMATION {
                    if input_param_changes.is_empty() {
                        block_end = data.num_samples as usize;
                    } else {
                        while let Some(Reverse((sample_idx, _))) = input_param_changes.peek() {
                            if *sample_idx != block_start {
                                block_end = *sample_idx;
                                break;
                            }

                            let Reverse((_, change)) = input_param_changes.pop().unwrap();
                            self.inner.set_normalized_value_by_hash(
                                change.hash,
                                change.normalized_value,
                                Some(sample_rate),
                            );
                            parameter_values_changed = true;
                        }
                    }
                }

                // This allows the GUI to react to incoming parameter changes
                if parameter_values_changed {
                    self.inner.notify_param_values_changed();
                    parameter_values_changed = false;
                }

                if P::ACCEPTS_MIDI {
                    let mut input_events = self.inner.input_events.borrow_mut();
                    if let Some(events) = data.input_events.upgrade() {
                        let num_events = events.get_event_count();

                        input_events.clear();
                        let mut event: MaybeUninit<_> = MaybeUninit::uninit();
                        for i in event_start_idx..num_events {
                            assert_eq!(events.get_event(i, event.as_mut_ptr()), kResultOk);
                            let event = event.assume_init();

                            // Make sure to only process the events for this block if we're
                            // splitting the buffer
                            if P::SAMPLE_ACCURATE_AUTOMATION
                                && event.sample_offset as u32 >= block_end as u32
                            {
                                event_start_idx = i;
                                break;
                            }

                            let timing = event.sample_offset as u32 - block_start as u32;
                            if event.type_ == vst3_sys::vst::EventTypes::kNoteOnEvent as u16 {
                                let event = event.event.note_on;
                                input_events.push_back(NoteEvent::NoteOn {
                                    timing,
                                    channel: event.channel as u8,
                                    note: event.pitch as u8,
                                    velocity: (event.velocity * 127.0).round() as u8,
                                });
                            } else if event.type_ == vst3_sys::vst::EventTypes::kNoteOffEvent as u16
                            {
                                let event = event.event.note_off;
                                input_events.push_back(NoteEvent::NoteOff {
                                    timing,
                                    channel: event.channel as u8,
                                    note: event.pitch as u8,
                                    velocity: (event.velocity * 127.0).round() as u8,
                                });
                            }
                        }
                    }
                }

                let result = if is_parameter_flush {
                    kResultOk
                } else {
                    let num_output_channels = (*data.outputs).num_channels as usize;

                    // This vector has been preallocated to contain enough slices as there are
                    // output channels
                    let mut output_buffer = self.inner.output_buffer.borrow_mut();
                    output_buffer.with_raw_vec(|output_slices| {
                        nih_debug_assert_eq!(num_output_channels, output_slices.len());
                        for (output_channel_idx, output_channel_slice) in
                            output_slices.iter_mut().enumerate()
                        {
                            // If `P::SAMPLE_ACCURATE_AUTOMATION` is set, then we may be iterating over
                            // the buffer in smaller sections.
                            // SAFETY: These pointers may not be valid outside of this function even though
                            // their lifetime is equal to this structs. This is still safe because they are
                            // only dereferenced here later as part of this process function.
                            let channel_ptr =
                                *((*data.outputs).buffers as *mut *mut f32).add(output_channel_idx);
                            *output_channel_slice = std::slice::from_raw_parts_mut(
                                channel_ptr.add(block_start),
                                block_end - block_start,
                            );
                        }
                    });

                    // Some hosts process data in place, in which case we don't need to do any
                    // copying ourselves. If the pointers do not alias, then we'll do the copy here
                    // and then the plugin can just do normal in place processing.
                    if !data.inputs.is_null() {
                        let num_input_channels = (*data.inputs).num_channels as usize;
                        nih_debug_assert!(
                            num_input_channels <= num_output_channels,
                            "Stereo to mono and similar configurations are not supported"
                        );
                        for input_channel_idx in
                            0..cmp::min(num_input_channels, num_output_channels)
                        {
                            let output_channel_ptr =
                                *((*data.outputs).buffers as *mut *mut f32).add(input_channel_idx);
                            let input_channel_ptr = *((*data.inputs).buffers as *const *const f32)
                                .add(input_channel_idx);
                            if input_channel_ptr != output_channel_ptr {
                                ptr::copy_nonoverlapping(
                                    input_channel_ptr.add(block_start),
                                    output_channel_ptr.add(block_start),
                                    block_end - block_start,
                                );
                            }
                        }
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
                        transport.pos_samples =
                            Some(context.project_time_samples + block_start as i64);
                        if context.state & (1 << 9) != 0 {
                            // kProjectTimeMusicValid
                            if P::SAMPLE_ACCURATE_AUTOMATION && (context.state & (1 << 10) != 0) {
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
                            transport.bar_start_pos_beats = Some(context.bar_position_music);
                        }
                        if context.state & (1 << 2) != 0 && context.state & (1 << 12) != 0 {
                            // kCycleActive && kCycleValid
                            transport.loop_range_beats =
                                Some((context.cycle_start_music, context.cycle_end_music));
                        }
                    }

                    let mut plugin = self.inner.plugin.write();
                    let mut context = self.inner.make_process_context(transport);

                    let result = plugin.process(&mut output_buffer, &mut context);
                    self.inner.last_process_status.store(result);
                    match result {
                        ProcessStatus::Error(err) => {
                            nih_debug_assert_failure!("Process error: {}", err);

                            return kResultFalse;
                        }
                        _ => kResultOk,
                    }
                };

                // If our block ends at the end of the buffer then that means there are no more
                // unprocessed (parameter) events. If there are more events, we'll just keep going
                // through this process until we've processed the entire buffer.
                if block_end as i32 == data.num_samples {
                    break result;
                } else {
                    block_start = block_end;
                }
            }
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
