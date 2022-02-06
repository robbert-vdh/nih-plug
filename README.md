# NIH-plug

[![Tests](https://github.com/robbert-vdh/nih-plugs/actions/workflows/test.yml/badge.svg)](https://github.com/robbert-vdh/nih-plugs/actions/workflows/test.yml)

This is a work in progress JUCE-lite-lite written in Rust to do some experiments
with. The idea is to have a statefull but simple plugin API that gets rid of as
much unnecessary ceremony wherever possible, while also keeping the amount of
magic to minimum. Since this is not quite meant for general use just yet, the
plugin API is limited to the functionality I needed and I'll expose more
functionality as I need it. See the documentation comment in the `Plugin` trait
for an incomplete list of missing functionality.

## Current status

It actually mostly work! There's still lots of small things to implement, but
the core functionality and including basic GUI support are there. That is, when
using Linux. Currently the event loop has not yet been implemented for Windows
and macOS.

## Building

NIH-plug doesn't use any unstable features, and works with the latest stable
Rust compiler.

After installing [Rust](https://rustup.rs/) you can compile any of the plugins
in the `plugins` directory in the following way, replacing `gain` with the name
of the plugin:

```shell
cargo xtask bundle gain --release --bundle-vst3
```

## Example plugins

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

Right now everything is licensed under the GPLv3+ license, partly because the
VST3 bindings used are also GPL licensed. I may split off the VST3 wrapper into
its own crate and relicense the core library under a more permissive license
later.
