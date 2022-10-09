# Safety Limiter

This plugin is a simple tool to prevent ear damage. As soon as there is a peak
above 0 dBFS or the specified threshold, the plugin will cut over to playing SOS
in Morse code, gradually fading out again when the input returns back to safe
levels. The same thing happens if the input contains infinite or NaN values.
Made for personal use during plugin development and intense sound design
sessions, but maybe you'll find it useful too!

**Why not use a regular brickwall peak limiter?**  
The downside of doing that is that you may end up mixing or doing sound design
into that limiter without realizing it. And at that point, you'll probably need
to either redo part of the process. So this plugin simply gives you a very
non-subtle heads up instead of altering the sound.

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
cargo xtask bundle safety_limiter --release
```
