# Spectral Compressor

Have you ever wondered what a 16384 band OTT would sound like? Neither have I.
Spectral Compressor can squash anything into pink noise, apply simultaneous
upwards and downwards compressor to dynamically match the sidechain signal's
spectrum and morph one sound into another, and lots more.

![Screenshot](https://i.imgur.com/r6WfoYp.png)

This is a port of https://github.com/robbert-vdh/spectral-compressor with more
features and much better performance.

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

After installing [Rust](https://rustup.rs/), you can compile Spectral Compressor
as follows:

```shell
cargo xtask bundle spectral_compressor --release
```
