use atomic_refcell::AtomicRefMut;
use clap_sys::ext::draft::remote_controls::{
    clap_remote_controls_page, CLAP_REMOTE_CONTROLS_COUNT,
};
use clap_sys::id::{clap_id, CLAP_INVALID_ID};
use clap_sys::string_sizes::CLAP_NAME_SIZE;
use std::cell::Cell;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use super::wrapper::{OutputParamEvent, Task, Wrapper};
use crate::event_loop::EventLoop;
use crate::prelude::{
    ClapPlugin, GuiContext, InitContext, ParamPtr, PluginApi, PluginNoteEvent, ProcessContext,
    RemoteControlsContext, RemoteControlsPage, RemoteControlsSection, Transport,
};
use crate::wrapper::util::strlcpy;

/// An [`InitContext`] implementation for the wrapper.
///
/// # Note
///
/// See the VST3 `WrapperInitContext` for an explanation of why we need this `pending_requests`
/// field.
pub(crate) struct WrapperInitContext<'a, P: ClapPlugin> {
    pub(super) wrapper: &'a Wrapper<P>,
    pub(super) pending_requests: PendingInitContextRequests,
}

/// Any requests that should be sent out when the [`WrapperInitContext`] is dropped. See that
/// struct's docstring for mroe information.
#[derive(Debug, Default)]
pub(crate) struct PendingInitContextRequests {
    /// The value of the last `.set_latency_samples()` call.
    latency_changed: Cell<Option<u32>>,
}

/// A [`ProcessContext`] implementation for the wrapper. This is a separate object so it can hold on
/// to lock guards for event queues. Otherwise reading these events would require constant
/// unnecessary atomic operations to lock the uncontested `RwLock`s.
pub(crate) struct WrapperProcessContext<'a, P: ClapPlugin> {
    pub(super) wrapper: &'a Wrapper<P>,
    pub(super) input_events_guard: AtomicRefMut<'a, VecDeque<PluginNoteEvent<P>>>,
    pub(super) output_events_guard: AtomicRefMut<'a, VecDeque<PluginNoteEvent<P>>>,
    pub(super) transport: Transport,
}

/// A [`GuiContext`] implementation for the wrapper. This is passed to the plugin in
/// [`Editor::spawn()`][crate::prelude::Editor::spawn()] so it can interact with the rest of the plugin and
/// with the host for things like setting parameters.
pub(crate) struct WrapperGuiContext<P: ClapPlugin> {
    pub(super) wrapper: Arc<Wrapper<P>>,
    #[cfg(debug_assertions)]
    pub(super) param_gesture_checker:
        atomic_refcell::AtomicRefCell<crate::wrapper::util::context_checks::ParamGestureChecker>,
}

/// A [`RemoteControlsContext`] implementation for the wrapper. This is used during initialization
/// to allow the plugin to declare remote control pages. This struct defines the pages in the
/// correct format.
pub(crate) struct RemoteControlPages<'a> {
    param_ptr_to_hash: &'a HashMap<ParamPtr, u32>,
    /// The remote control pages, as defined by the plugin. These don't reference any heap data so
    /// we can store them directly.
    pages: &'a mut Vec<clap_remote_controls_page>,
}

impl<P: ClapPlugin> Drop for WrapperInitContext<'_, P> {
    fn drop(&mut self) {
        if let Some(samples) = self.pending_requests.latency_changed.take() {
            self.wrapper.set_latency_samples(samples)
        }
    }
}

impl<P: ClapPlugin> InitContext<P> for WrapperInitContext<'_, P> {
    fn plugin_api(&self) -> PluginApi {
        PluginApi::Clap
    }

    fn execute(&self, task: P::BackgroundTask) {
        (self.wrapper.task_executor.lock())(task);
    }

    fn set_latency_samples(&self, samples: u32) {
        // See this struct's docstring
        self.pending_requests.latency_changed.set(Some(samples));
    }

    fn set_current_voice_capacity(&self, capacity: u32) {
        self.wrapper.set_current_voice_capacity(capacity)
    }
}

impl<P: ClapPlugin> ProcessContext<P> for WrapperProcessContext<'_, P> {
    fn plugin_api(&self) -> PluginApi {
        PluginApi::Clap
    }

    fn execute_background(&self, task: P::BackgroundTask) {
        let task_posted = self.wrapper.schedule_background(Task::PluginTask(task));
        nih_debug_assert!(task_posted, "The task queue is full, dropping task...");
    }

    fn execute_gui(&self, task: P::BackgroundTask) {
        let task_posted = self.wrapper.schedule_gui(Task::PluginTask(task));
        nih_debug_assert!(task_posted, "The task queue is full, dropping task...");
    }

    #[inline]
    fn transport(&self) -> &Transport {
        &self.transport
    }

    fn next_event(&mut self) -> Option<PluginNoteEvent<P>> {
        self.input_events_guard.pop_front()
    }

    fn send_event(&mut self, event: PluginNoteEvent<P>) {
        self.output_events_guard.push_back(event);
    }

    fn set_latency_samples(&self, samples: u32) {
        self.wrapper.set_latency_samples(samples)
    }

    fn set_current_voice_capacity(&self, capacity: u32) {
        self.wrapper.set_current_voice_capacity(capacity)
    }
}

impl<P: ClapPlugin> GuiContext for WrapperGuiContext<P> {
    fn plugin_api(&self) -> PluginApi {
        PluginApi::Clap
    }

    fn request_resize(&self) -> bool {
        self.wrapper.request_resize()
    }

    // All of these functions are supposed to be called from the main thread, so we'll put some
    // trust in the caller and assume that this is indeed the case
    unsafe fn raw_begin_set_parameter(&self, param: ParamPtr) {
        match self.wrapper.param_ptr_to_hash.get(&param) {
            Some(hash) => {
                let success = self
                    .wrapper
                    .queue_parameter_event(OutputParamEvent::BeginGesture { param_hash: *hash });

                nih_debug_assert!(
                    success,
                    "Parameter output event queue was full, parameter change will not be sent to \
                     the host"
                );
            }
            None => nih_debug_assert_failure!("Unknown parameter: {:?}", param),
        }

        #[cfg(debug_assertions)]
        match self.wrapper.param_id_from_ptr(param) {
            Some(param_id) => self
                .param_gesture_checker
                .borrow_mut()
                .begin_set_parameter(param_id),
            None => nih_debug_assert_failure!(
                "raw_begin_set_parameter() called with an unknown ParamPtr"
            ),
        }
    }

    unsafe fn raw_set_parameter_normalized(&self, param: ParamPtr, normalized: f32) {
        match self.wrapper.param_ptr_to_hash.get(&param) {
            Some(hash) => {
                // We queue the parameter change event here, and it will be sent to the host either
                // at the end of the current processing cycle or after requesting an explicit flush
                // (when the plugin isn't processing audio). The parameter's actual value will only
                // be changed when the output event is written to prevent changing parameter values
                // in the middle of processing audio.
                let clap_plain_value = normalized as f64 * param.step_count().unwrap_or(1) as f64;
                let success = self
                    .wrapper
                    .queue_parameter_event(OutputParamEvent::SetValue {
                        param_hash: *hash,
                        clap_plain_value,
                    });

                nih_debug_assert!(
                    success,
                    "Parameter output event queue was full, parameter change will not be sent to \
                     the host"
                );
            }
            None => nih_debug_assert_failure!("Unknown parameter: {:?}", param),
        }

        #[cfg(debug_assertions)]
        match self.wrapper.param_id_from_ptr(param) {
            Some(param_id) => self
                .param_gesture_checker
                .borrow_mut()
                .set_parameter(param_id),
            None => {
                nih_debug_assert_failure!("raw_set_parameter() called with an unknown ParamPtr")
            }
        }
    }

    unsafe fn raw_end_set_parameter(&self, param: ParamPtr) {
        match self.wrapper.param_ptr_to_hash.get(&param) {
            Some(hash) => {
                let success = self
                    .wrapper
                    .queue_parameter_event(OutputParamEvent::EndGesture { param_hash: *hash });

                nih_debug_assert!(
                    success,
                    "Parameter output event queue was full, parameter change will not be sent to \
                     the host"
                );
            }
            None => nih_debug_assert_failure!("Unknown parameter: {:?}", param),
        }

        #[cfg(debug_assertions)]
        match self.wrapper.param_id_from_ptr(param) {
            Some(param_id) => self
                .param_gesture_checker
                .borrow_mut()
                .end_set_parameter(param_id),
            None => {
                nih_debug_assert_failure!("raw_end_set_parameter() called with an unknown ParamPtr")
            }
        }
    }

    fn get_state(&self) -> crate::wrapper::state::PluginState {
        self.wrapper.get_state_object()
    }

    fn set_state(&self, state: crate::wrapper::state::PluginState) {
        self.wrapper.set_state_object_from_gui(state)
    }
}

/// A remote control section. The plugin can fill this with information for one or more pages.
pub(crate) struct Section {
    pages: Vec<Page>,
}

/// A remote control page. These are automatically split into multiple pages if the number of
/// controls exceeds 8.
pub(crate) struct Page {
    name: String,
    params: Vec<Option<ParamPtr>>,
}

impl<'a> RemoteControlPages<'a> {
    /// Allow the plugin to define remote control pages and add them to `pages`. This does not clear
    /// `pages` first.
    pub fn define_remote_control_pages<P: ClapPlugin>(
        plugin: &P,
        pages: &'a mut Vec<clap_remote_controls_page>,
        param_ptr_to_hash: &'a HashMap<ParamPtr, u32>,
    ) {
        // The magic happens in the `add_section()` function defined below
        plugin.remote_controls(&mut Self {
            pages,
            param_ptr_to_hash,
        });
    }

    /// Perform the boilerplate needed for creating and adding a new [`clap_remote_controls_page`].
    /// If `params` contains more than eight parameters then any further parameters will be lost.
    fn add_clap_page(
        &mut self,
        section: &str,
        page_name: &str,
        params: impl IntoIterator<Item = Option<ParamPtr>>,
    ) {
        let mut page = clap_remote_controls_page {
            section_name: [0; CLAP_NAME_SIZE],
            // Pages are numbered sequentially
            page_id: self.pages.len() as clap_id,
            page_name: [0; CLAP_NAME_SIZE],
            param_ids: [CLAP_INVALID_ID; CLAP_REMOTE_CONTROLS_COUNT],
            is_for_preset: false,
        };
        strlcpy(&mut page.section_name, section);
        strlcpy(&mut page.page_name, page_name);

        let mut params = params.into_iter();
        for (param_id, param_ptr) in page.param_ids.iter_mut().zip(&mut params) {
            // `param_id` already has the correct value if `param_ptr` is empty/a spacer
            if let Some(param_ptr) = param_ptr {
                *param_id = self.param_ptr_to_id(param_ptr);
            }
        }

        nih_debug_assert!(
            params.next().is_none(),
            "More than eight parameters were passed to 'RemoteControlPages::add_page()', this is \
             a NIH-plug bug."
        );

        self.pages.push(page);
    }

    /// Transform a `ParamPtr` to the associated CLAP parameter ID/hash. Returns -1/invalid
    /// parameter and triggers a debug assertion when the parameter is not known.
    fn param_ptr_to_id(&self, ptr: ParamPtr) -> clap_id {
        match self.param_ptr_to_hash.get(&ptr) {
            Some(id) => *id,
            None => {
                nih_debug_assert_failure!(
                    "An unknown parameter was added to a remote control page, ignoring..."
                );

                CLAP_INVALID_ID
            }
        }
    }
}

impl RemoteControlsContext for RemoteControlPages<'_> {
    type Section = Section;

    fn add_section(&mut self, name: impl Into<String>, f: impl FnOnce(&mut Self::Section)) {
        let section_name = name.into();
        let mut section = Section {
            pages: Vec::with_capacity(1),
        };
        f(&mut section);

        // The pages in the section may need to be split up into multiple pages if it defines more
        // than eight parameters. This keeps the interface flexible for potential future expansion
        // and makes manual paging unnecessary in some situations.
        for page in section.pages {
            if page.params.len() > CLAP_REMOTE_CONTROLS_COUNT {
                for (subpage_idx, subpage_params) in
                    page.params.chunks(CLAP_REMOTE_CONTROLS_COUNT).enumerate()
                {
                    let subpage_name = format!("{} {}", page.name, subpage_idx + 1);
                    self.add_clap_page(
                        &section_name,
                        &subpage_name,
                        subpage_params.iter().copied(),
                    );
                }
            } else {
                self.add_clap_page(&section_name, &page.name, page.params);
            }
        }
    }
}

impl RemoteControlsSection for Section {
    type Page = Page;

    fn add_page(&mut self, name: impl Into<String>, f: impl FnOnce(&mut Self::Page)) {
        let mut page = Page {
            name: name.into(),
            params: Vec::with_capacity(CLAP_REMOTE_CONTROLS_COUNT),
        };
        f(&mut page);

        self.pages.push(page);
    }
}

impl RemoteControlsPage for Page {
    fn add_param(&mut self, param: &impl crate::prelude::Param) {
        self.params.push(Some(param.as_ptr()));
    }

    fn add_spacer(&mut self) {
        self.params.push(None);
    }
}
