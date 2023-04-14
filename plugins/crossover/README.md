# Crossover

This plugin is as boring as it sounds. It cleanly splits the signal into two to
five bands using a variety of algorithms. Those bands are then sent to auxiliary
outputs so they can be accessed and processed individually. Meant as an
alternative to Bitwig's Multiband FX devices but with cleaner crossovers and a
linear-phase option.

In Bitwig Studio you'll want to click on the 'Show plug-in multi-out chain
selector' button and then on 'Add missing chains' to access the chains. The main
output will not output any audio. To save time, you can save this setup as the
default preset by right clicking on the device. Any new Crossover instances will
then already have the additional output chains set up. You can also download
[this
preset](https://cdn.discordapp.com/attachments/767397282344599602/1096417371880685669/Crossover_setup.bwpreset),
load it, and then set it as your default preset for Crossover.

<!-- Screenshots and other binary assets aren't in this repo as that would add bloat to NIH-plug checkouts -->

![Screenshot in Bitwig](https://i.imgur.com/hrn9uhR.png)

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

After installing **nightly** [Rust](https://rustup.rs/) toolchain, you can
compile Crossover as follows:

```shell
cargo +nightly xtask bundle crossover --release
```
