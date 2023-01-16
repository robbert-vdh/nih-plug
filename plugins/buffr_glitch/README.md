# Buffr Glitch

Like the sound of a CD player skipping? Well, do I have the plugin for you. This
plugin is essentially a MIDI triggered buffer repeat plugin. When you play a
note, the plugin will sample the period corresponding to that note's frequency
and use that as a single waveform cycle. This can end up sounding like an
in-tune glitch when used sparingly, or like a weird synthesizer when used less
subtly.

## Tips

- You can control the buffer's gain by enabling the velocity sensitive mode and
  changing the velocity. In Bitwig Studio and other DAWs that support volume
  note expressions you can also control the gain that way.

## Download

You can download the development binaries for Linux, Windows and macOS from the
[automated
builds](https://github.com/robbert-vdh/nih-plug/actions/workflows/build.yml?query=branch%3Amaster)
page. Or if you're not signed in on GitHub, then you can also find the latest nightly
build [here](https://nightly.link/robbert-vdh/nih-plug/workflows/build/master).

On macOS you may need to [disable
Gatekeeper](https://disable-gatekeeper.github.io/) as Apple has recently made it
more difficult to run unsigned code on macOS.

### Building

After installing [Rust](https://rustup.rs/), you can compile Buffr Glitch as
follows:

```shell
cargo xtask bundle buffr_glitch --release
```
