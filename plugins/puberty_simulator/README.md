# Puberty Simulator

This patent pending One Weird Plugin simulates the male voice change during
puberty! If it was not already obvious from that sentence, this plugin is a
joke, but it might actually be useful (or at least interesting) in some
situations. This plugin pitches the signal down an octave, but it also has the
side effect of causing things to sound like a cracking voice or to make them
sound slightly out of tune.

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

After installing [Rust](https://rustup.rs/), you can compile Puberty Simulator
as follows:

```shell
cargo xtask bundle puberty_simulator --release
```
