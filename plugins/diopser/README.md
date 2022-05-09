# Diopser

You were expecting Disperser[¹](#disperser), but it was me, Diopser!

Diopser lets you rotate the phase of a signal around a specific frequency
without affecting its spectral content. This effect can be used to emphasize
transients and other parts of a sound that in a way that isn't possible with
regular equalizers or dynamics processors, especially when applied to low
pitched or wide band sounds. More extreme settings will make everything sound
like a cartoon laser beam, or a psytrance kickdrum. If you are experimenting
with those kinds of settings, then you may want to consider temporarily placing
a peak limiter after the plugin in case loud resonances start building up.

This is a port from https://github.com/robbert-vdh/diopser with more features
and much better performance.

<sup id="disperser">
  *Disperser is a trademark of Kilohearts AB. Diopser is in no way related to
  Disperser or Kilohearts AB.
</sup>

## Download

You can download the development binaries for Linux, Windows and macOS from the
[automated
builds](https://github.com/robbert-vdh/nih-plug/actions/workflows/test.yml?query=branch%3Amaster)
page. If you're not signed in on GitHub, then you can also find the latest nightly
build [here](https://nightly.link/robbert-vdh/nih-plug/workflows/build/master).

The macOS version has not been tested and may not work correctly. You may also
have to [disable Gatekeeper](https://disable-gatekeeper.github.io/) to use the
VST3 version as Apple has recently made it more difficult to run unsigned code
on macOS.

### Building

After installing [Rust](https://rustup.rs/) with the nightly toolchain (because
of the use of SIMD), you can compile Diopser as follows:

```shell
cargo +nightly xtask bundle diopser --release
```
