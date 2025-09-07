# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic
Versioning](https://semver.org/spec/v2.0.0.html).

Since there is no stable release yet, the changes are organized per day in
reverse chronological order. The main purpose of this document in its current
state is to list breaking changes.

## [2025-02-23]

### Breaking changes

- `nih_plug_egui` now uses egui 0.31.

### Added

- `nih_plug_egui` has a new `ResizableWindow` widget that can be used to resize
  the plugin's editor.

### Changed

- The CLAP bindings were updated to 1.2.2. The only noticeable difference is
  that the remote controls exposed through `ClapPlugin::remote_controls()` now
  use the non-draft extension.

### Fixed

- Fixed a warning about future name clashes when compiling `nih_plug`.

## [2024-12-23]

### Added

- `nih_plug_vizia`'s `ParamSlider` has a new style that always shows the offset
  relative to the center of the slider.

## [2024-08-18]

### Breaking changes

- The minimum supported Rust version has been bumped to 1.80 to replace the last
  uses of `lazy_static` with `std::sync::LazyLock`.

## [2024-05-05]

### Breaking changes

- `nih_plug_egui` has been updated from egui 0.26.1 to egui 0.27.2.
- `nih_plug_vizia` has been updated to the latest version with some a additional
  patches. This includes a workaround for the problem where opening multiple
  instances of a plugin's GUI on Windows or macOS would result in crashes.

### Changed

- Two byte slices are now accepted in `NoteEvent::from_midi()` if the event
  doesn't use the third byte.

### Fixed

- Fixed a race condition in the VST3 GUI event loop on Linux. This could cause
  panics with certain versions of Carla.
- The CPAL backend now correctly handles situations where it receives fewer
  samples than configured.
- Fixed the handling of multichannel audio in the CPAL backend.

## [2024-05-04]

### Fixed

- Fixed a soundness issue in the buffer management where in-place input/output
  buffers may not have been recognized properly before.

## [2024-03-23]

### Added

- `nih_plug_xtask` now detects and uses non-standard `target` directory
  locations if overridden through Cargo's settings.

## [2024-03-18]

### Changed

- (Keyboard) input events sent by the host through VST3's `IPlugView` interface
  are now explicitly ignored. This may allow a couple more keyboard events to
  reach through to plugin windows in hosts that use these.

## [2024-02-23]

### Fixed

- Fixed `nih_plug_egui` panicking due to cursor icons not yet being implemented in baseview for MacOS and Windows.

## [2024-02-22]

### Breaking changes

- `nih_plug_egui` has been updated from egui 0.22.0 to using egui 0.26.1.

## [2023-12-30]

### Breaking changes

- `nih_plug_vizia` has been updated to the latest Vizia version. Vizia's styling
  system has changed a lot since the last update, so plugin GUIs and stylesheets
  may require small changes before they behave the same again. A summary of the
  most important changes can be found in Vizia PR
  [#291](https://github.com/vizia/vizia/pull/291). Some notable breaking changes
  include:

  - Font handling and choosing between different variations of the same font
    (e.g. `Noto Sans` versus `Noto Sans Light` versus `Noto Sans Light Italic`)
    works very differently now.
  - `ResizeHandle` now needs to be the last element in a GUI because of changes
    to Vizia's event targetting mechanism.

- The `raw_window_handle` version used by NIH-plug has been updated to version
  0.5.x.

### Added

- Added initial RISC-V support to `nih_plug_xtask`.
  ([#95](https://github.com/robbert-vdh/nih-plug/pull/95)).

### Changed

- `ParentWindowHandle` has changed to be a sum type of different parent window
  handle types, similar to `RawWindowHandle`. This makes it easier to use GUI
  libraries that link against a different version of `raw_window_handle` than
  the one used by NIH-plug itself by simply wrapping around
  `ParentWindowHandle`.
- `nih_debug_assert*!()` failures are now promoted to a warning instead of a
  debug message. This makes the non-fatal debug assertion failures easier to
  spot.
- The minimum scale factor in `nih_plug_vizia` has changed from 0.25 to 0.5.
  Vizia rounds things to single pixels, and below 0.5 scaling single pixel
  borders would disappear when not using a HiDPI setup.

### Fixed

- Various `baseview` dependencies now have their versions pinned.

## [2023-12-06]

### Fixed

- `nih_export_vst3!()` no longer requires `nih_debug_assert` to be in scope.

## [2023-11-05]

### Changed

- `FloatParam` and `IntParam` ranges can now be accessed using methods on the
  parameters ([#89](https://github.com/robbert-vdh/nih-plug/pull/89)).

## [2023-09-21]

### Fixed

- Fixed null pointers assertions in the low level buffer management code not
  working correctly.

## [2023-09-03]

### Added

- `nih_export_vst3!()` now also supports more than one plugin type argument,
  just like `nih_export_clap!()`.

### Fixed

- The `nih_export_*!()` macros now use `$crate` to refer to NIH-plug itself,
  which makes it possible to use the NIH-plug crate under a different name.

## [2023-08-05]

### Breaking changes

- The minimum supported Rust version has been bumped to 1.70 so we can start
  using `OnceCell` and `OnceLock` to phase out uses of `lazy_static`.

### Added

- `nih_export_clap!()` can now take more than one plugin type argument to allow
  exporting more than one plugin from a single plugin library.

## [2023-05-13]

### Fixed

- Removed the `Default` bound from the `SysExMessage::Buffer` type. This was a
  leftover from an older design.

## [2023-04-30]

### Changed

- Added debug assertions to make sure parameter ranges are valid. The minimum
  value must always be lower than the maximum value and they cannot be equal.

## [2023-04-27]

### Changed

- The `v2s_f32_rounded()` formatter now avoids returning negative zero values
  for roundtripping reasons since -0.0 and 0.0 correspond to the same normalized
  value.

## [2023-04-24]

### Breaking changes

- `Plugin::editor()` and `Plugin::task_executor()` now take `&mut self` instead
  of `&self` to make it easier to move data into these functions without
  involving interior mutability.

### Changed

- The `Plugin` trait's documentation has been updated to better clarify the
  structure and to more explicitly mention that the non-lifecycle methods are
  called once immediately after creating the plugin object.

### Fixed

- The logger now uses the correct local time offset on Linux instead of
  defaulting to UTC due to some implementation details of the underlying `time`
  crate.
- The buffer changes from March 31st broke the sample accurate automation
  feature. This has now been fixed.

## [2023-04-22]

### Added

- CLAP plugins can optionally declare pages of [remote
  controls](https://github.com/free-audio/clap/blob/main/include/clap/ext/draft/remote-controls.h)
  so DAWs can more automatically map pages of the plugin's parameters to
  hardware controllers. This is currently a draft extension, so until the
  extension is finalized host support may break at any moment.

### Changed

- The CLAP version has been updated to 1.1.8.
- The prelude module now also re-exports the following:
  - The `PluginApi` num.
  - The `Transport` struct.

### Fixed

- The upgrade to CLAP 1.1.8 caused NIH-plug to switch from the draft version of
  the voice info extension to the final version, fixing voice stacking with
  recent versions of Bitwig.

## [2023-04-05]

### Breaking changes

- The `nih_debug_assert*!()` macros are now upgraded to regular panicking
  `debug_assert!()` macros during tests.
- `SmoothingStyle::for_oversampling_factor()` has been removed in favor of a new
  mechanism that allows the smmoothers to be aware of oversampling. A new
  `Smoothingstyle::OversamplingAware(oversampling_times, style)` can be used to
  wrap another `Smoothingstyle` to make it aware of an oversampling amount that
  can change at runtime. The `oversampling_times` is an `Arc<AtomicF32>` that
  indicates the current oversampling amount. This makes it possible to link
  multiple parameters to the same oversampling amount, have different sets of
  parameters run at different effective sample rates, and automatically update
  those oversampling amounts/sample rate multipliers from a parameter callback.
- As a consequence of the above change, `Smoothingstyle` is no longer `Copy`
  since the `OversamplingAware` smoothing style contain an
  `Arc<Smoothingstyle>`. It can still be `Clone`d.

### Changed

- The prelude module now also re-exports the `AtomicF32` type since it's needed
  to use the new `Smoothingstyle::OversamplingAware`.

## [2023-04-01]

### Fixed

- Auxiliary output buffers are now always zeroed out in case the host didn't do
  this for us. This was a regression from before 2023-03-31.

## [2023-03-31]

### Changed

- Buffer management has been completely rewritten so it can be shared among all
  of NIH-plug's backends. This should not result in any noticeable changes, but
  it should reduce the chances of backend-specific bugs when it comes to
  interacting with audio buffers and it will make it simpler to implement buffer
  management for new plugin APIs.

### Fixed

- When a main IO audio buffers has more output channels than input channels, the
  excess output channels are now correctly filled with zeroes instead of
  containing whatever data was left in the host's output buffers. As part of
  this change NIH-plug's buffer management has been refactored to reuse the same
  logic in all of its wrappers.
- Any outstanding VST3 output events are now sent to the host during a parameter
  flush.

## [2023-03-21]

### Changed

- The logger now always shows the module in debug builds to make it easier to
  know where logging messages are sent from. Previously this was only done for
  the debug and trace message levels.
- The logger now filters out the `Mapped XXXX font faces in YYYms.` messages
  from cosmic text in release builds as this is unnecessary noise for end users.
- `nih_plug_vizia`: `ParamButton`'s active color was made much lighter to make
  the text more readable, and the hover state has been fixed.

## [2023-03-18]

### Added

- `nih_plug_vizia`: Added a `GuiContextEvent::Resize` event. The plugin can emit
  this to trigger a resize to its current size, as specified by its
  `ViziaState`'s size callback. This can be used to declaratively resize a
  plugin GUI and it removes some potential surface for making mistakes in the
  process. See `GuiContextEvent::Resize`'s documentation for an example.

## [2023-03-17]

### Added

- Added a `NoteEvent::channel()` method to get an event's channel, if it has
  any. ([#62](https://github.com/robbert-vdh/nih-plug/pull/62))

## [2023-03-07]

This document is now also used to keep track of non-breaking changes.

### Breaking changes

- The way window sizes work in `ViziaState` has been reworked to be more
  predictable and reliable. Instead of creating a `ViziaState` with a predefined
  size and then tracking the window's current size in that object, `ViziaState`
  now takes a callback that returns the window's current logical size. This can
  be used to compute the window's current size based on the plugin's state. The
  result is that window sizes always match the plugin's current state and
  recalling an old incorrect size is no longer possible.

### Added

- Debug builds now include debug assertions that detect incorrect use of the
  `GuiContext`'s parameter setting methods.

## [2023-02-28]

### Breaking changes

- `ViziaState::from_size()` now takes a third boolean argument to control
  whether the window's size is persisted or not. This avoids a potential bug
  where an old window size is recalled after the plugin's GUI's size has changed
  in an update to the plugin.

## [2023-02-20]

### Breaking changes

- The way audio IO layouts are configured has changed completely to align better
  with NIH-plug's current and future supported plugin API backends. Rather than
  defining a default layout and allowing the host/backend to change the channel
  counts by polling the `Plugin::accepts_bus_config()` function, the plugin now
  explicitly enumerates all supported audio IO layouts in a declarative fashion.
  This change gives the plugin more options for defining alternative audio port
  layouts including layouts with variable numbers of channels and ports, while
  simultaneously removing ambiguities and behavior that was previously governed
  by heuristics.

  All types surrounding bus layouts and port names have changed slightly to
  accommodate this change. Take a look at the updated examples for more details
  on how this works. The `Plugin::AUDIO_IO_LAYOUTS` field's documentation also
  contains an example for how to initialize the layouts slice.

- As a result of the above change, NIH-plug's standalones no longer have
  `--input` and `--output` command line arguments to change the number of input
  and output channels. Instead, they now have an `--audio-layout` option that
  lets the user select an audio layout from the list of available layouts by
  index. `--audio-layout=help` can be used to list those layouts.

## [2023-02-01]

### Breaking changes

- The `Vst3Plugin::VST3_CATEGORIES` string constant has been replaced by a
  `Vst3Plugin::VST3_SUBCATEGORIES` constant of type `&[Vst3SubCategory]`.
  `Vst3SubCategory` is an enum containing all of VST3's predefined categories,
  and it behaves similarly to the `ClapFeature` enum used for CLAP plugins. This
  makes defining subcategories for VST3 plugins easier and less error prone.

## [2023-01-31]

### Breaking changes

- NIH-plug has gained support MIDI SysEx in a simple, type-safe, and
  realtime-safe way. This sadly does mean that every `Plugin` instance now needs
  to define a `SysExMessage` type definition and constructor function as Rust
  does not yet support defaults for associated types (Rust issue
  [#29661](https://github.com/rust-lang/rust/issues/29661)):

  ```rust
  type SysExMessage = ();
  ```

- As the result of the above change, `NoteEvent` is now parameterized by a
  `SysExMessage` type. There is a new `PluginNoteEvent<P>` type synonym that can
  be parameterized by a `Plugin` to make using this slightly less verbose.

## [2023-01-12]

### Breaking changes

- The Vizia dependency has been updated. This updated version uses a new text
  rendering engine, so there are a couple breaking changes:
  - The names for some of Vizia's fonts have changed. The constants and font
    registration functions in `nih_plug_vizia::assets` and
    `nih_plug_vizia::vizia_assets` still have the same name, but all uses of the
    `font` CSS property and `.font()` view modifier will have to be changed.
  - Metrics for rendered text have change slightly. Most notably the height and
    vertical positioning of text is slightly different, so you may have to
    adjust your layout slightly accordingly.

## [2023-01-11]

### Breaking changes

- `Editor::param_values_changes()` is no longer called from the audio thread and
  thus no longer needs to be realtime safe.
- A new `Editor::param_value_changed(id, normalized_value)` method has been
  added. This is used to notify the plugin of changes to individual parameters.
- A similar new `Editor::param_modulation_changed(id, modulation_offset)` is
  used to inform the plugin of a parameter's new monophonic modulation offset.

## [2023-01-06]

### Breaking changes

- The threads used for the `.schedule_gui()` and `.schedule_background()`
  methods are now shared between all instances of a plugin. This makes
  `.schedule_gui()` on Linux behave more like it does on Windows and macOS, and
  there is now only a single background thread instead of each instance spawning
  their own thread.

## [2023-01-05]

### Breaking changes

- `Buffer::len()` has been renamed to `Buffer::samples()` to make this less
  ambiguous.
- `Block::len()` has been renamed to `Block::samples()`.

## [2022-11-17]

### Breaking changes

- The `Params` derive macro now also properly supports persistent fields in
  `#[nested]` parameter structs. This takes `#[nested(id_prefix = "...")]` and
  `#[nested(array)]` into account to allow multiple copies of a persistent
  field. This may break existing usages as serialized field data without a
  matching preffix or suffix is no longer passed to the child object.

## [2022-11-17]

### Breaking changes

- The order of `#[nested]` parameters in the parameter list now always follows
  the declaration order instead of nested parameters being ordered below regular
  parameters.

## [2022-11-08]

### Breaking changes

- The `Param::{next_previous}{_step,_normalized_step}()` functions now take an
  additional boolean argument to indicate that the range must be finer. This is
  used for floating point parameters to chop the range up into smaller segments
  when using Shift+scroll.

## [2022-11-07]

### Breaking changes

- `Param::plain_value()` and `Param::normalized_value()` have been renamed to
  `Param::modulated_plain_value()` and `Param::modulated_normalized_value()`.
  These functions are only used when creating GUIs, so this shouldn't break any
  other plugin code. This change was made to make it extra clear that these
  values do include monophonic modulation, as it's very easy to mistakenly use
  the wrong value when handling user input in GUI widgets.

## [2022-11-06]

### Breaking changes

- `nih_plug_vizia::create_vizia_editor_without_theme()` has been removed, and
  `nih_plug_vizia::create_vizia_editor()` has gained a new argument to specify
  what amount of theming to apply. This can now also be used to completely
  disable all theming include Vizia's built-in theme.
- `nih_plug_vizia::create_vizia_editor()` no longer registers any fonts by
  default. Even when those fonts are not used, they will still be embedded in
  the binary, increasing its size by several megabytes. Instead, you can now
  register individual fonts by calling the
  `nih_plug_vizia::assets::register_*()` functions. This means that you _must_
  call `nih_plug_vizia::assets::register_noto_sans_light()` for the default
  theming to work. All of the plugins in this repo also use
  `nih_plug_vizia::assets::register_noto_sans_thin()` as a title font.
- Additionally, the Vizia fork has been updated to not register _any_ default
  fonts for the same reason. If you previously relied on Vizia's default Roboto
  font, then you must now call `nih_plug_vizia::vizia_assets::register_roboto()`
  at the start of your process function.

## [2022-10-23]

### Breaking changes

- `nih_plug_vizia` has been updated. Widgets with custom drawing code will need
  to be updated because of changes in Vizia itself.

## [2022-10-22]

### Breaking changes

- The `Editor` trait and the `ParentWindowHandle` struct have been moved from
  `nih_plug::plugin` to a new `nih_plug::editor` module. If you only use the
  prelude module then you won't need to change anything.
- The `nih_plug::context` module has been split up into
  `nih_plug::context::init`, `nih_plug::context::process`, and
  `nih_plug::context::gui` to make it clearer which structs go with which
  context. You again don't have to change anything if you use the prelude.
- NIH-plug has gained support for asynchronously running background tasks in a
  simple, type-safe, and realtime-safe way. This sadly does mean that every
  `Plugin` instance now needs to define a `BackgroundTask` type definition and
  constructor function as Rust does not yet support defaults for associated
  types (Rust issue [#29661](https://github.com/rust-lang/rust/issues/29661)):

  ```rust
  type BackgroundTask = ();
  ```

- The `&mut impl InitContext` argument to `Plugin::initialize()` needs to be
  changed to `&mut impl InitContext<Self>`.
- The `&mut impl ProcessContext` argument to `Plugin::process()` needs to be
  changed to `&mut impl ProcessContext<Self>`.
- The `Plugin::editor()` method now also takes a
  `_async_executor: AsyncExecutor<Self>` parameter.

## [2022-10-20]

### Breaking changes

- Some items have been moved out of `nih_plug::param::internals`. The main
  `Params` trait is now located under `nih_plug::param`, and the
  `PersistentTrait` trait, implementations, and helper functions are now part of
  a new `nih_plug::param::persist` module. Code importing the `Params` trait
  through the prelude module doesn't need to be changed.
- The `nih_plug::param` module has been renamed to `nih_plug::params`. Code that
  only uses the prelude module doesn't need to be changed.
- The `create_egui_editor()` function from `nih_plug_egui` now also takes a
  build closure to apply initialization logic to the egui context.
- `Editor` and the editor handle returned by `Editor::spawn` now only require
  `Send` and no longer need `Sync`. This is not a breaking change, but it might
  be worth being aware of.
- Similar to the above change, `Plugin` also no longer requires `Sync`.

## [2022-10-13]

### Breaking changes

- The `#[nested]` parameter attribute has gained super powers and has its syntax
  changed. It can now automatically handle many situations that previously
  required custom `Params` implementations to have multiple almost identical
  copies of a parameter struct. The current version supports both fields with
  unique parameter ID prefixes, and arrays of parameter objects. See the
  [`Params`](https://nih-plug.robbertvanderhelm.nl/nih_plug/param/internals/trait.Params.html)
  trait for more information on the new syntax.

## [2022-09-22]

### Breaking changes

- `nih_plug_vizia` has been updated. Custom widgets will need to be updated
  because of changes Vizia itself.
- `nih_plug_egui` has been updated from egui 0.17 to egui 0.19.

## [2022-09-06]

### Breaking changes

- Parameter values are now accessed using `param.value()` instead of
  `param.value`, with `param.value()` being an alias for the existing
  `param.plain_value()` function. The old approach, while perfectly safe in
  practice, was technically unsound because it used mutable pointers to
  parameters that may also be simultaneously read from in an editor GUI. With
  this change the parameters now use actual relaxed atomic stores and loads to
  avoid mutable aliasing, which means the value fields are now no longer
  directly accessible.

## [2022-09-04]

### Breaking changes

- `Smoother::next_block_mapped()` and `Smoother::next_block_exact_mapped()` have
  been redesigned. They now take an index of the element being generated and the
  float representation of the smoothed value. This makes it easier to use them
  for modulation, and it makes it possible to smoothly modulate integers and
  other stepped parameters. Additionally, the mapping functions are now also
  called for every produced value, even if the smoother has already finished
  smoothing and is always producing the same value.

## [2022-08-19]

### Breaking changes

- Standalones now use the plugin's default input and output channel counts
  instead of always defaulting to two inputs and two outputs.
- `Plugin::DEFAULT_NUM_INPUTS` and `Plugin::DEFAULT_NUM_OUTPUTS` have been
  renamed to `Plugin::DEFAULT_INPUT_CHANNELS` and
  `Plugin::DEFAULT_OUTPUT_CHANNELS` respectively to avoid confusion as these
  constants only affect the main input and output.

## [2022-07-18]

### Breaking changes

- `IntRange` and `FloatRange` no longer have min/max methods and instead have
  next/previous step methods. This is for better compatibility with the new
  reversed ranges.

## [2022-07-06]

### Breaking changes

- There are new `NoteEvent::PolyModulation` and `NoteEvent::MonoAutomation`
  events as part of polyphonic modulation support for CLAP plugins.
- The block smoothing API has been reworked. Instead of `Smoother`s having their
  own built-in block buffer, you now need to provide your own mutable slice for
  the smoother to fill. This makes the API easier to understand, more flexible,
  and it allows cloning smoothers without worrying about allocations.In
  addition, the new implementation is much more efficient when the smoothing
  period has ended before or during the block.

## [2022-07-05]

### Breaking changes

- The `ClapPlugin::CLAP_HARD_REALTIME` constant was moved to the general
  `Plugin` trait as `Plugin::HARD_REALTIME_ONLY`, and best-effort support for
  VST3 has been added.

## [2022-07-04]

### Breaking changes

- The `CLAP_DESCRIPTION`, `CLAP_MANUAL_URL`, and `CLAP_SUPPORT_URL` associated
  constants from the `ClapPlugin` are now optional and have the type
  `Option<&'static str>` instead of `&'static str`.
- Most `NoteEvent` variants now have an additional `voice_id` field.
- There is a new `NoteEvent::VoiceTerminated` event a plugin can send to let the
  host know a voice has been terminated. This needs to be output by CLAP plugins
  that support polyphonic modulation.
- There is a new `NoteEvent::Choke` event the host can send to a plugin to let
  it know that it should immediately terminate all sound associated with a voice
  or a key.

## [2022-07-02]

### Breaking changes

- The `Params::serialize_fields()` and `Params::deserialize_fields()` methods
  and the `State` struct now use `BTreeMap`s instead of `HashMap`s so the order
  is consistent the plugin's state to JSON multiple times. These things are part
  of NIH-plug's internals, so unless you're implementing the `Params` trait by
  hand you will not notice any changes.

## [2022-06-01]

### Breaking changes

- The `ClapPlugin::CLAP_FEATURES` field now uses an array of `ClapFeature`
  values instead of `&'static str`s. CLAP 0.26 contains many new predefined
  features, and the existing ones now use dashes instead of underscores. Custom
  features are still possible using `ClapFeature::Custom`.

## [2022-05-27]

### Breaking changes

- `Plugin::process()` now takes a new `aux: &mut AuxiliaryBuffers` parameter.
  This was needed to allow auxiliary (sidechain) inputs and outputs.
- The `Plugin::initialize()` method now takes a `&mut impl InitContext` instead
  of a `&mut impl ProcessContext`.

## [2022-05-22]

### Breaking changes

- The current processing mode is now stored in `BufferConfig`. Previously this
  could be fetched through a function on the `ProcessContext`, but this makes
  more sense as it remains constant until a plugin is deactivated. The
  `BufferConfig` now contains a field for the minimum buffer size that may or
  may not be set depending on the plugin API.
- Previously calling `param.non_automatable()` when constructing a parameter
  also made the parameter hidden. Hiding a parameter is now done through
  `param.hide()`, while `param.non_automatable()` simply makes it so that the
  parameter can only be changed manually and not through automation or
  modulation.

## ...

Who knows what happened at this point!
