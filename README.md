# NIH-plug

[![Automated builds](https://github.com/robbert-vdh/nih-plug/actions/workflows/build.yml/badge.svg?branch=master)](https://github.com/robbert-vdh/nih-plug/actions/workflows/build.yml?query=branch%3Amaster)
[![Tests](https://github.com/robbert-vdh/nih-plug/actions/workflows/test.yml/badge.svg?branch=master)](https://github.com/robbert-vdh/nih-plug/actions/workflows/test.yml?query=branch%3Amaster)
[![Docs](https://github.com/robbert-vdh/nih-plug/actions/workflows/docs.yml/badge.svg?branch=master)](https://nih-plug.robbertvanderhelm.nl/)

NIH-plug is an API-agnostic audio plugin framework written in Rust, as well as a
small collection of plugins. The idea is to have a stateful yet simple plugin
API that gets rid of as much unnecessary ceremony wherever possible, while also
keeping the amount of magic to minimum and making it easy to experiment with
different approaches to things. See the [current features](#current-features)
section for more information on the project's current status.

Check out the [documentation](https://nih-plug.robbertvanderhelm.nl/), or use
the [cookiecutter template](https://github.com/robbert-vdh/nih-plug-template) to
quickly get started with NIH-plug.

### Table of contents

- [Plugins](#plugins)
- [Framework](#framework)
  - [Current features](#current-features)
  - [Building](#building)
  - [Plugin formats](#plugin-formats)
  - [Example plugins](#example-plugins)
- [Licensing](#licensing)

## Plugins

Check each plugin's readme file for more details on what the plugin actually
does. You can download the development binaries for Linux, Windows and macOS
from the [automated
builds](https://github.com/robbert-vdh/nih-plug/actions/workflows/build.yml?query=branch%3Amaster)
page. Or if you're not signed in on GitHub, then you can also find the latest
nightly build
[here](https://nightly.link/robbert-vdh/nih-plug/workflows/build/master). You
may need to [disable Gatekeeper](https://disable-gatekeeper.github.io/) on macOS to be able to use
the plugins.

Scroll down for more information on the underlying plugin framework.

- [**Buffr Glitch**](plugins/buffr_glitch) is the plugin for you if you enjoy
  the sound of a CD player skipping This plugin is essentially a MIDI triggered
  buffer repeat plugin. When you play a note, the plugin will sample the period
  corresponding to that note's frequency and use that as a single waveform
  cycle. This can end up sounding like an in-tune glitch when used sparingly, or
  like a weird synthesizer when used less subtly.
- [**Crisp**](plugins/crisp) adds a bright crispy top end to any low bass sound.
  Inspired by Polarity's [Fake Distortion](https://youtu.be/MKfFn4L1zeg) video.
- [**Crossover**](plugins/crossover) is as boring as it sounds. It cleanly
  splits the signal into two to five bands using a variety of algorithms. Those
  bands are then sent to auxiliary outputs so they can be accessed and processed
  individually. Meant as an alternative to Bitwig's Multiband FX devices but
  with cleaner crossovers and a linear-phase option.
- [**Diopser**](plugins/diopser) is a totally original phase rotation plugin.
  Useful for oomphing up kickdrums and basses, transforming synths into their
  evil phase-y cousin, and making everything sound like a cheap Sci-Fi laser
  beam.
- [**Loudness War Winner**](plugins/loudness_war_winner) does what it says on
  the tin. Have you ever wanted to show off your dominance by winning the
  loudness war? Neither have I. Dissatisfaction guaranteed.
- [**Puberty Simulator**](plugins/puberty_simulator) is that patent pending One
  Weird Plugin that simulates the male voice change during puberty! If it was
  not already obvious from that sentence, this plugin is a joke, but it might
  actually be useful (or at least interesting) in some situations. This plugin
  pitches the signal down an octave, but it also has the side effect of causing
  things to sound like a cracking voice or to make them sound slightly out of
  tune.
- [**Safety Limiter**](plugins/safety_limiter) is a simple tool to prevent ear
  damage. As soon as there is a peak above 0 dBFS or the specified threshold,
  the plugin will cut over to playing SOS in Morse code, gradually fading out
  again when the input returns back to safe levels. Made for personal use during
  plugin development and intense sound design sessions, but maybe you'll find it
  useful too!
- [**Soft Vacuum**](plugins/soft_vacuum) is a straightforward port of
  Airwindows' [Hard Vacuum](https://www.airwindows.com/hard-vacuum-vst/) plugin
  with parameter smoothing and up to 16x linear-phase oversampling, because I
  liked the distortion and just wished it had oversampling. All credit goes to
  Chris from Airwindows. I just wanted to share this in case anyone else finds
  it useful.
- [**Spectral Compressor**](plugins/spectral_compressor) can squash anything
  into pink noise, apply simultaneous upwards and downwards compressor to
  dynamically match the sidechain signal's spectrum and morph one sound into
  another, and lots more. Have you ever wondered what a 16384 band OTT would
  sound like? Neither have I.

## Framework

### Current features

- Supports both VST3 and [CLAP](https://github.com/free-audio/clap) by simply
  adding the corresponding `nih_export_<api>!(Foo)` macro to your plugin's
  library.
- Standalone binaries can be made by calling `nih_export_standalone(Foo)` from
  your `main()` function. Standalones come with a CLI for configuration and full
  JACK audio, MIDI, and transport support.
- Rich declarative parameter system without any boilerplate.
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
  - Optional support for state migrations, for handling breaking changes in
    plugin parameters.
  - Group your parameters into logical groups by nesting `Params` objects using
    the `#[nested(group = "...")]`attribute.
  - The `#[nested]` attribute also enables you to use multiple copies of the
    same parameter, either as regular object fields or through arrays.
  - When needed, you can also provide your own implementation for the `Params`
    trait to enable compile time generated parameters and other bespoke
    functionality.
- Stateful. Behaves mostly like JUCE, just without all of the boilerplate.
- Comes with a simple yet powerful way to asynchronously run background tasks
  from a plugin that's both type-safe and realtime-safe.
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
- Optional support for compressing the human readable JSON state files using
  [Zstandard](https://en.wikipedia.org/wiki/Zstd).
- Comes with adapters for popular Rust GUI frameworks as well as some basic
  widgets for them that integrate with NIH-plug's parameter system. Currently
  there's support for [egui](nih_plug_egui), [iced](nih_plug_iced) and
  [VIZIA](nih_plug_vizia).
  - A simple and safe API for state saving and restoring from the editor is
    provided by the framework if you want to do your own internal preset
    management.
- Full support for receiving and outputting both modern polyphonic note
  expression events as well as MIDI CCs, channel pressure, and pitch bend for
  CLAP and VST3.
  - MIDI SysEx is also supported. Plugins can define their own structs or sum
    types to wrap around those messages so they don't need to interact with raw
    byte buffers in the process function.
- Support for flexible dynamic buffer configurations, including variable numbers
  of input and output ports.
- First-class support several more exotic CLAP features:
  - Both monophonic and polyphonic parameter modulation are supported.
  - Plugins can declaratively define pages of remote controls that DAWs can bind
    to hardware controllers.
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
  of the functionality that has currently not yet been implemented.

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
macro. The `cargo xtask bundle` command will detect which plugin formats your
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
  [egui](plugins/examples/gain_gui_egui),
  [iced](plugins/examples/gain_gui_iced), and
  [VIZIA](plugins/examples/gain_gui_vizia).

  There are also examples for making custom GUIs with
  [OpenGL](plugins/examples/byo_gui_gl), [wgpu](plugins/examples/byo_gui_wgpu),
  and [softbuffer](plugins/examples/byo_gui_softbuffer).

- [**midi_inverter**](plugins/examples/midi_inverter) takes note/MIDI events and
  flips around the note, channel, expression, pressure, and CC values. This
  example demonstrates how to receive and output those events.
- [**poly_mod_synth**](plugins/examples/poly_mod_synth) is a simple polyphonic
  synthesizer with support for polyphonic modulation in supported CLAP hosts.
  This demonstrates how polyphonic modulation can be used in NIH-plug.
- [**sine**](plugins/examples/sine) is a simple test tone generator plugin with
  frequency smoothing that can also make use of MIDI input instead of generating
  a static signal based on the plugin's parameters.
- [**stft**](plugins/examples/stft) shows off some of NIH-plug's other optional
  higher level helper features, such as an adapter to process audio with a
  short-term Fourier transform using the overlap-add method, all using the
  compositional `Buffer` interfaces.
- [**sysex**](plugins/examples/sysex) is a simple example of how to send and
  receive SysEx messages by defining custom message types.

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
