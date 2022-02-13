# NIH-plug

[![Tests](https://github.com/robbert-vdh/nih-plugs/actions/workflows/test.yml/badge.svg)](https://github.com/robbert-vdh/nih-plugs/actions/workflows/test.yml)

This is a work in progress JUCE-lite-lite written in Rust to do some experiments
with, as well as a small collection of plugins. The idea is to have a statefull
but simple plugin API that gets rid of as much unnecessary ceremony wherever
possible, while also keeping the amount of magic to minimum. Since this is not
quite meant for general use just yet, the plugin API is limited to the
functionality I needed and I'll expose more functionality as I need it. See the
documentation comment in the `Plugin` trait for an incomplete list of missing
functionality.

### Table of contents

- [Plugins](#plugins)
- [Framework](#framework)
  - [Current status](#current-status)
  - [Building](#building)
  - [Example plugins](#example-plugins)
- [Licensing](#licensing)

## Plugins

Check each plugin's readme for more details on what the plugin actually does.
There are currently no automated builds available, so check the
[building](#building) section for instructions on how to compile these plugins
yourself.

- [**Diopser**](plugins/diopser) is a totally original phase rotation plugin.
  Useful for oomphing up kickdrums and basses, transforming synths into their
  evil phase-y cousin, and making everything sound like a cheap Sci-Fi laser
  beam. **This is an unfinished port of an existing plugin.**

## Framework

### Current status

It actually works! There's still lots of small things to implement, but the core
functionality and including basic GUI support are there. Currently the event
loop has not yet been implemented for macOS, and the Windows version should work
great but it has only been tested under Wine with
[yabridge](https://github.com/robbert-vdh/yabridge).

### Building

NIH-plug works with the latest stable Rust compiler.

After installing [Rust](https://rustup.rs/) you can compile any of the plugins
in the `plugins` directory in the following way, replacing `gain` with the name
of the plugin:

```shell
cargo xtask bundle gain --release --bundle-vst3
```

### Example plugins

The best way to get an idea for what the API looks like is to look at the
examples.

- **gain** is a simple smoothed gain plugin that shows off a couple other parts
  of the API, like support for storing arbitrary serializable state.
- **gain-gui** is the same plugin as gain, but with a GUI to control the
  parameter and a digital peak meter.
- **sine** is a simple test tone generator plugin with frequency smoothing that
  can also make use of MIDI input instead of generating a static signal based on
  the plugin's parameters.

## Licensing

The framework and its libraries are licensed under the [ISC
license](https://www.isc.org/licenses/). However, the [VST3
bindings](https://github.com/RustAudio/vst3-sys) used by `nih_export_vst3!()`
are licensed under the GPLv3. This means that unless you replace these bindings
with your own bindings that you made from scratch, any VST3 plugins built with
NIH-plug also need to be able to comply with the terms of the GPLv3 license.

The example plugins in `plugins/examples/` are also ISC-licensed, but the other
plugins in the `plugins/` directory may be licensed under the GPLv3 license.
Check the plugin's `Cargo.toml` file for more information.
