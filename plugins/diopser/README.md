# Diopser

You were expecting Disperser[¹](#disperser), but it was me, Diopser!

Diopser lets you rotate the phase of a signal around a specific frequency
without affecting its spectral content. This effect can be used to emphasize
transients and other parts of a sound that in a way that isn't possible with
regular equalizers or dynamics processors, especially when applied to low
pitched or wide band sounds. More extreme settings will make everything sound
like a cartoon laser beam, or a psytrance kickdrum.

![Screenshot](https://i.imgur.com/QLtHtQL.png)

This is a port of https://github.com/robbert-vdh/diopser with more features and
much better performance.

<sup id="disperser">
  *Disperser is a trademark of Kilohearts AB. Diopser is in no way related to
  Disperser or Kilohearts AB.
</sup>

## Tips

- Alt+click on the spectrum analyzer to enter to enter a frequency value in
  Hertz or musical notes.
- Hold down Alt/Option while dragging the filter frequency around to snap to
  whole notes.
- The safe mode is enabled by default. This limits the frequency range and the
  number of filter stages. Simply disable the safe mode if you want to crank
  everything up to 11. With safe mode disabled you may find that going down to
  the bottom of the frequency range introduces some loud low frequency
  resonances, especially when combined with a lot of filter stages. In that case
  you may want to use a peak limiter after this plugin until you understand how
  it reacts to different changes. Or maybe you'll want to check out [Safety
  Limiter](../safety_limiter), which is made for this exact purpose.
- Turn down the automation precision to reduce the DSP load hit of changing the
  filter frequency and resonance at the cost of introducing varying amounts of
  aliasing and zipper noises.
- The aforementioned artifacts introduced by setting a low automation precision
  can actually be useful for sound design purposes.
- Change the number of filter stages to immediately reset the filters and stop
  ringing.

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
