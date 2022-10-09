# Diopser

You were expecting Disperser[¹](#disperser), but it was me, Diopser!

Diopser lets you rotate the phase of a signal around a specific frequency
without affecting its spectral content. This effect can be used to emphasize
transients and other parts of a sound that in a way that isn't possible with
regular equalizers or dynamics processors, especially when applied to low
pitched or wide band sounds. More extreme settings will make everything sound
like a cartoon laser beam, or a psytrance kickdrum.

Because this plugin lets you crank every parameter up to 11, you may want to
avoid rapidly sweeping the frequency parameter down all the way to 5 Hertz when
you have many filter stages enabled. Because of the way these filters work, this
may cause comparatively loud resonances in the 0-15 Hertz range. In that case
you may want to use a peak limiter after this plugin until you understand how it
reacts to different changes, or maybe you'll want to check out [Safety
Limiter](../safety_limiter), which is made for this exact purpose.

This is a port of https://github.com/robbert-vdh/diopser with more features and
much better performance.

<sup id="disperser">
  *Disperser is a trademark of Kilohearts AB. Diopser is in no way related to
  Disperser or Kilohearts AB.
</sup>

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

After installing [Rust](https://rustup.rs/) with the nightly toolchain (because
of the use of SIMD), you can compile Diopser as follows:

```shell
cargo +nightly xtask bundle diopser --release
```
