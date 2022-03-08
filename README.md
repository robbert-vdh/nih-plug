# NIH-plug

[![Tests](https://github.com/robbert-vdh/nih-plug/actions/workflows/test.yml/badge.svg?branch=master)](https://github.com/robbert-vdh/nih-plug/actions/workflows/test.yml?query=branch%3Amaster)

This is a work in progress JUCE-lite-lite written in Rust to do some experiments
with, as well as a small collection of plugins. The idea is to have a statefull
but simple plugin API that gets rid of as much unnecessary ceremony wherever
possible, while also keeping the amount of magic to minimum. Since this is not
quite meant for general use just yet, the plugin API is limited to the
functionality I needed and I'll expose more functionality as I need it. See the
documentation comment in the `Plugin` trait for an incomplete list of missing
functionality.

Come join us on the [Rust Audio Discord](https://discord.gg/ykxU3rt4Cb).

### Table of contents

- [Plugins](#plugins)
- [Framework](#framework)
  - [Current status](#current-status)
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

### Current status

It actually works! There's still lots of small things to implement, but the core
functionality and basic GUI support are there, with export targets and plugin
bundling for both VST3 and CLAP. Currently the Windows support has only been
tested under Wine with [yabridge](https://github.com/robbert-vdh/yabridge), and
the macOS version hasn't been tested at all. Feel free to be the first one!

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
- [**gain-gui**](plugins/examples/gain-gui) is the same plugin as gain, but with
  a GUI to control the parameter and a digital peak meter.
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
