use crossbeam::channel::{unbounded, Receiver, Sender};
use nih_plug::prelude::*;
use std::{
    sync::{Arc, Mutex},
    thread::sleep,
    time::Duration,
};

// Name the plugin's tasks
enum Task {
    HelloWorld,
}

struct MyPlugin {
    params: Arc<MyParams>,
    greeting: Arc<Mutex<String>>,
    channel: (Sender<String>, Receiver<String>),
}

impl Default for MyPlugin {
    fn default() -> Self {
        Self {
            params: Arc::new(MyParams::default()),
            greeting: Arc::new(Mutex::new(String::from("hello world"))),
            channel: unbounded(),
        }
    }
}

impl Plugin for MyPlugin {
    // Identify the enum as the type the plugin uses for task names
    type BackgroundTask = Task;

    // Implement the plugin's task runner by switching on task name.
    //   - Called after the plugin instance is created
    //   - Send result back over a channel or triple buffer
    fn task_executor(&mut self) -> TaskExecutor<Self> {
        let greeting = self.greeting.clone();
        let s = self.channel.0.clone();
        Box::new(move |task| match task {
            Task::HelloWorld => {
                sleep(Duration::from_secs(2));
                nih_log!("task: {}", greeting.lock().unwrap());
                s.send(String::from("sent from task")).unwrap();
            }
        })
    }

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        _buffer_config: &BufferConfig,
        context: &mut impl InitContext<Self>,
    ) -> bool {
        nih_log!("initialize: initializing the plugin");
        *(self.greeting.lock().unwrap()) = String::from("task run from initialize method");

        // Run synchronously
        context.execute(Self::BackgroundTask::HelloWorld);

        // log messages from background task
        while let Ok(message) = self.channel.1.try_recv() {
            nih_log!("initialize: {message}");
        }

        true
    }

    fn reset(&mut self) {}

    fn process(
        &mut self,
        _buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        *(self.greeting.lock().unwrap()) = String::from("task run from process method");
        let transport = context.transport();

        if transport.playing && 0 == transport.pos_samples().unwrap_or_default() {
            nih_log!("process: processing first buffer after play pressed");

            // Run on a background thread
            context.execute_background(Self::BackgroundTask::HelloWorld);

            // Waits for previous run of same task to complete
            context.execute_background(Self::BackgroundTask::HelloWorld);
        }

        // log messages from background task
        while let Ok(message) = self.channel.1.try_recv() {
            nih_log!("process: {message}");
        }

        ProcessStatus::Normal
    }

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    type SysExMessage = ();

    const NAME: &'static str = "nih-plug task example";
    const VENDOR: &'static str = "Brian Edwards";
    const URL: &'static str = env!("CARGO_PKG_HOMEPAGE");
    const EMAIL: &'static str = "brian.edwards@jalopymusic.com";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[];
    const MIDI_INPUT: MidiConfig = MidiConfig::None;
    const MIDI_OUTPUT: MidiConfig = MidiConfig::None;
    const SAMPLE_ACCURATE_AUTOMATION: bool = true;
}

#[derive(Default, Params)]
struct MyParams {}

impl ClapPlugin for MyPlugin {
    const CLAP_ID: &'static str = "com.jalopymusic.nih-plug-task";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("nih-plug background task example");
    const CLAP_MANUAL_URL: Option<&'static str> = Some(Self::URL);
    const CLAP_SUPPORT_URL: Option<&'static str> = Some(Self::URL);
    const CLAP_FEATURES: &'static [ClapFeature] = &[];
}

impl Vst3Plugin for MyPlugin {
    const VST3_CLASS_ID: [u8; 16] = *b"NihPlugTaskExamp";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[];
}

nih_export_clap!(MyPlugin);
nih_export_vst3!(MyPlugin);
