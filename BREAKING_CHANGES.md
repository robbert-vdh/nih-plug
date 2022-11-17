# Breaking changes

Since there is no stable release yet, there is also no proper changelog yet. But
since not everyone might want to dive through commit messages to find out what's
new and what's changed, this document lists all breaking changes in reverse
chronological order. If a new feature did not require any changes to existing
code then it will not be listed here.

## [2022-11-17]

- The order of `#[nested]` parameters in the parameter list now always follows
  the declaration order instead of nested parameters being ordered below regular
  parameters.

## [2022-11-08]

- The `Param::{next_previous}{_step,_normalized_step}()` functions now take an
  additional boolean argument to indicate that the range must be finer. This is
  used for floating point parameters to chop the range up into smaller segments
  when using Shift+scroll.

## [2022-11-07]

- `Param::plain_value()` and `Param::normalized_value()` have been renamed to
  `Param::modulated_plain_value()` and `Param::modulated_normalized_value()`.
  These functions are only used when creating GUIs, so this shouldn't break any
  other plugin code. This change was made to make it extra clear that these
  values do include monophonic modulation, as it's very easy to mistakenly use
  the wrong value when handling user input in GUI widgets.

## [2022-11-06]

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

- `nih_plug_vizia` has been updated. Widgets with custom drawing code will need
  to be updated because of changes in Vizia itself.

## [2022-10-22]

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

- The `#[nested]` parameter attribute has gained super powers and has its syntax
  changed. It can now automatically handle many situations that previously
  required custom `Params` implementations to have multiple almost identical
  copies of a parameter struct. The current version supports both fields with
  unique parameter ID prefixes, and arrays of parameter objects. See the
  [`Params`](https://nih-plug.robbertvanderhelm.nl/nih_plug/param/internals/trait.Params.html)
  trait for more information on the new syntax.

## [2022-09-22]

- `nih_plug_vizia` has been updated. Custom widgets will need to be updated
  because of changes Vizia itself.
- `nih_plug_egui` has been updated from egui 0.17 to egui 0.19.

## [2022-09-06]

- Parameter values are now accessed using `param.value()` instead of
  `param.value`, with `param.value()` being an alias for the existing
  `param.plain_value()` function. The old approach, while perfectly safe in
  practice, was technically unsound because it used mutable pointers to
  parameters that may also be simultaneously read from in an editor GUI. With
  this change the parameters now use actual relaxed atomic stores and loads to
  avoid mutable aliasing, which means the value fields are now no longer
  directly accessible.

## [2022-09-04]

- `Smoother::next_block_mapped()` and `Smoother::next_block_exact_mapped()` have
  been redesigned. They now take an index of the element being generated and the
  float representation of the smoothed value. This makes it easier to use them
  for modulation, and it makes it possible to smoothly modulate integers and
  other stepped parameters. Additionally, the mapping functions are now also
  called for every produced value, even if the smoother has already finished
  smoothing and is always producing the same value.

## [2022-08-19]

- Standalones now use the plugin's default input and output channel counts
  instead of always defaulting to two inputs and two outputs.
- `Plugin::DEFAULT_NUM_INPUTS` and `Plugin::DEFAULT_NUM_OUTPUTS` have been
  renamed to `Plugin::DEFAULT_INPUT_CHANNELS` and
  `Plugin::DEFAULT_OUTPUT_CHANNELS` respectively to avoid confusion as these
  constants only affect the main input and output.

## [2022-07-18]

- `IntRange` and `FloatRange` no longer have min/max methods and instead have
  next/previous step methods. This is for better compatibility with the new
  reversed ranges.

## [2022-07-06]

- There are new `NoteEvent::PolyModulation` and `NoteEvent::MonoAutomation`
  events as part of polyphonic modulation support for CLAP plugins.
- The block smoothing API has been reworked. Instead of `Smoother`s having their
  own built-in block buffer, you now need to provide your own mutable slice for
  the smoother to fill. This makes the API easier to understand, more flexible,
  and it allows cloning smoothers without worrying about allocations.In
  addition, the new implementation is much more efficient when the smoothing
  period has ended before or during the block.

## [2022-07-05]

- The `ClapPlugin::CLAP_HARD_REALTIME` constant was moved to the general
  `Plugin` trait as `Plugin::HARD_REALTIME_ONLY`, and best-effort support for
  VST3 has been added.

## [2022-07-04]

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

- The `Params::serialize_fields()` and `Params::deserialize_fields()` methods
  and the `State` struct now use `BTreeMap`s instead of `HashMap`s so the order
  is consistent the plugin's state to JSON multiple times. These things are part
  of NIH-plug's internals, so unless you're implementing the `Params` trait by
  hand you will not notice any changes.

## [2022-06-01]

- The `ClapPlugin::CLAP_FEATURES` field now uses an array of `ClapFeature`
  values instead of `&'static str`s. CLAP 0.26 contains many new predefined
  features, and the existing ones now use dashes instead of underscores. Custom
  features are still possible using `ClapFeature::Custom`.

## [2022-05-27]

- `Plugin::process()` now takes a new `aux: &mut AuxiliaryBuffers` parameter.
  This was needed to allow auxiliary (sidechain) inputs and outputs.
- The `Plugin::initialize()` method now takes a `&mut impl InitContext` instead
  of a `&mut impl ProcessContext`.

## [2022-05-22]

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
