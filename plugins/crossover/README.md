# Crossover

This plugin is as boring as it sounds. It cleanly splits the signal into two to
five bands using a variety of algorithms. Those bands are then sent to auxiliary
outputs so they can be accessed and processed individually. Meant as an
alternative to Bitwig's Multiband FX devices but with cleaner crossovers and a
linear-phase option.

In Bitwig Studio you'll want to click on the 'Show plug-in multi-out chain
selector' button and then on 'Add missing chains' to access the chains. The main
output will not output any audio.

<!-- Screenshots and other binary assets aren't in this repo as that would add bloat to NIH-plug checkouts -->

![Screenshot in Bitwig](https://imgur.com/lvwgHQf.png)

## Download

You can download the development binaries for Linux, Windows and macOS from the
[automated
builds](https://github.com/robbert-vdh/nih-plug/actions/workflows/build.yml?query=branch%3Amaster)
page. Or if you're not signed in on GitHub, then you can also find the latest nightly
build [here](https://nightly.link/robbert-vdh/nih-plug/workflows/build/master).

The macOS version has not been tested and may not work correctly. You may also
have to [disable Gatekeeper](https://disable-gatekeeper.github.io/) to use the
VST3 version as Apple has recently made it more difficult to run unsigned code
on macOS.

### Building

After installing **nightly** [Rust](https://rustup.rs/) toolchain, you can
compile Crossover as follows:

```shell
cargo +nightly xtask bundle crossover --release
```
