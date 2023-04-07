# Soft Vacuum (Airwindows port)

This is a straightforward port of Airwindows' [Hard
Vacuum](https://www.airwindows.com/hard-vacuum-vst/) plugin with parameter
smoothing and up to 16x linear-phase oversampling, because I liked the
distortion and just wished it had oversampling. All credit goes to Chris from
Airwindows. I just wanted to share this in case anyone else finds it useful.

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

After installing [Rust](https://rustup.rs/), you can compile Safety Limiter as
follows:

```shell
cargo xtask bundle soft_vacuum --release
```
