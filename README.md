# NIH-plug

[![Automated builds](https://github.com/robbert-vdh/nih-plug/actions/workflows/build.yml/badge.svg?branch=master)](https://github.com/robbert-vdh/nih-plug/actions/workflows/build.yml?query=branch%3Amaster)
[![Tests](https://github.com/robbert-vdh/nih-plug/actions/workflows/test.yml/badge.svg?branch=master)](https://github.com/robbert-vdh/nih-plug/actions/workflows/test.yml?query=branch%3Amaster)
[![Docs](https://github.com/robbert-vdh/nih-plug/actions/workflows/docs.yml/badge.svg?branch=master)](https://robbert-vdh.github.io/nih-plug)

This is a work in progress API-agnostic audio plugin framework written in Rust
to do some experiments with, as well as a small collection of plugins. The idea
is to have a statefull but simple plugin API that gets rid of as much
unnecessary ceremony wherever possible, while also keeping the amount of magic
to minimum. Since this is not quite meant for general use just yet, the plugin
API surface is currently limited to the functionality that I either needed
myself or that was requested by others. See the [current
features](#current-features) section for more information on the project's
current status.

Come join us on the [Rust Audio Discord](https://discord.gg/ykxU3rt4Cb).

### Table of contents

- [Plugins](#plugins)
- [Framework](#framework)
  - [Current features](#current-features)
  - [Building](#building)
  - [Plugin formats](#plugin-formats)
  - [Example plugins](#example-plugins)
- [Licensing](#licensing)

## Plugins

Check each plugin's readme for more details on what the plugin actually does and
for download links.

- [**Crisp**](plugins/crisp) adds a bright crispy top end to any low bass sound.
  Inspired by Polarity's [Fake Distortion](https://youtu.be/MKfFn4L1zeg) video.
- [**Diopser**](plugins/diopser) is a totally original phase rotation plugin.
  Useful for oomphing up kickdrums and basses, transforming synths into their
  evil phase-y cousin, and making everything sound like a cheap Sci-Fi laser
  beam.
- [**Puberty Simulator**](plugins/puberty_simulator) is that patent pending One
  Weird Plugin that simulates the male voice change during puberty! If it was
  not already obvious from that sentence, this plugin is a joke, but it might
  actually be useful (or at least interesting) in some situations. This plugin
  pitches the signal down an octave, but it also has the side effect of causing
  things to sound like a cracking voice or to make them sound slightly out of
  tune.

## Framework

### Current features

- Supports both VST3 and [CLAP](https://github.com/free-audio/clap) by simply
  adding the corresponding `nih_export_<api>!(Foo)` macro to your plugin's
  library.
- Declarative parameter handling without any boilerplate.
  - Define parameters for your plugin by adding `FloatParam`, `IntParam`,
    `BoolParam`, and `EnumParam<T>` fields to your parameter struct, assign
    stable IDs to them with the `#[id = "foobar"]`, and a `#[derive(Params)]`
    does all of the boring work for you.
  - Parameters can have complex value distributions and the parameter objects
    come with built-in smoothers and callbacks.
  - Use simple enums deriving the `Enum` trait with the `EnumParam<T>` parameter
    type for parameters that allow the user to choose between multiple discrete
    options. That way you can use regular Rust pattern matching when working
    with these values without having to do any conversions yourself.
  - Store additional non-parameter state for your plugin by adding any field
    that can be serialized with [Serde](https://serde.rs/) to your plugin's
    `Params` object and annotating them with `#[persist = "key"]`.
  - Group your parameters into logical groups by nesting `Params` objects using
    the `#[nested = "Group Name"]`attribute.
  - When needed, you can also provide your own implementation for the `Params`
    trait to enable dynamically generated parameters and arrays of if mostly
    identical parameter objects.
- Stateful. Behaves mostly like JUCE, just without all of the boilerplate.
- Does not make any assumptions on how you want to process audio, but does come
  with utilities and adapters to help with common access patterns.
  - Efficiently iterate over an audio buffer either per-sample per-channel,
    per-block per-channel, or even per-block per-sample-per-channel with the
    option to manually index the buffer or get access to a channel slice at any
    time.
  - Easily leverage per-channel SIMD using the SIMD adapters on the buffer and
    block iterators.
  - Comes with bring-your-own-FFT adapters for common (inverse) short-time
    Fourier Transform operations. More to come.
- Optional sample accurate automation support for VST3 and CLAP that can be
  enabled by setting the `Plugin::SAMPLE_ACCURATE_AUTOMATION` constant to
  `true`.
- Comes with adapters for popular Rust GUI frameworks as well as some basic
  widgets for them that integrate with NIH-plug's parameter system. Currently
  there's support for [egui](nih_plug_egui), [iced](nih_plug_iced) and
  [VIZIA](nih_plug_vizia).
  - A simple and safe API for state saving and restoring from the editor is
    provided by the framework if you want to do your own internal preset
    management.
- Full support for both modern polyphonic note expressions as well as MIDI CCs,
  channel pressure, and pitch bend for CLAP and VST3.
- A plugin bundler accessible through the
  `cargo xtask bundle <package> <build_arguments>` command that automatically
  detects which plugin targets your plugin exposes and creates the correct
  plugin bundles for your target operating system and architecture, with
  cross-compilation support. The cargo subcommand can easily be added to [your
  own project](https://github.com/robbert-vdh/nih-plug/tree/master/nih_plug_xtask)
  as an alias or [globally](https://github.com/robbert-vdh/nih-plug/tree/master/cargo_nih_plug)
  as a regular cargo subcommand.
- Tested on Linux and Windows, with limited testing on macOS. Windows support
  has mostly been tested through Wine with
  [yabridge](https://github.com/robbert-vdh/yabridge).
- See the [`Plugin`](src/plugin.rs) trait's documentation for an incomplete list
  of the functionlaity that has currently not yet been implemented.

### Building

NIH-plug works with the latest stable Rust compiler.

After installing [Rust](https://rustup.rs/), you can compile any of the plugins
in the `plugins` directory in the following way, replacing `gain` with the name
of the plugin:

```shell
cargo xtask bundle gain --release
```

### Plugin formats

NIH-plug can currently export VST3 and
[CLAP](https://github.com/free-audio/clap) plugins. Exporting a specific plugin
format for a plugin is as simple as calling the `nih_export_<format>!(Foo);`
macro. The `cargo xtask bundle` commane will detect which plugin formats your
plugin supports and create the appropriate bundles accordingly, even when cross
compiling.

### Example plugins

The best way to get an idea for what the API looks like is to look at the
examples.

- [**gain**](plugins/examples/gain) is a simple smoothed gain plugin that shows
  off a couple other parts of the API, like support for storing arbitrary
  serializable state.
- **gain-gui** is the same plugin as gain, but with a GUI to control the
  parameter and a digital peak meter. Comes in three exciting flavors:
  [egui](plugins/examples/gain-gui-egui),
  [iced](plugins/examples/gain-gui-iced), and
  [VIZIA](plugins/examples/gain-gui-vizia).
- [**sine**](plugins/examples/sine) is a simple test tone generator plugin with
  frequency smoothing that can also make use of MIDI input instead of generating
  a static signal based on the plugin's parameters.
- [**stft**](plugins/examples/stft) shows off some of NIH-plug's other optional
  higher level helper features, such as an adapter to process audio with a
  short-term Fourier transform using the overlap-add method, all using the
  compositional `Buffer` interfaces.

## Licensing

The framework, its libraries, and the example plugins in `plugins/examples/` are
all licensed under the [ISC license](https://www.isc.org/licenses/). However,
the [VST3 bindings](https://github.com/RustAudio/vst3-sys) used by
`nih_export_vst3!()` are licensed under the GPLv3 license. This means that
unless you replace these bindings with your own bindings made from scratch, any
VST3 plugins built with NIH-plug need to be able to comply with the terms of the
GPLv3 license.

The other plugins in the `plugins/` directory may be licensed under the GPLv3
license. Check the plugin's `Cargo.toml` file for more information.
