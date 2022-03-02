# NIH-plug: cargo subcommand for bundling plugins

This is NIH-plug's `cargo xtask` command, but as a `cargo` subcommand. This way
you can use it outside of NIH-plug projects. If you're using NIH-plug, you'll
want to use the xtask integration directly instead so you don't need to worry
about keeping the command up to date, see:
<https://github.com/robbert-vdh/nih-plug/tree/master/nih_plug_xtask>.

Since this has not yet been published to `crates.io`, you'll need to install
this using:

```shell
cargo install --git https://github.com/robbert-vdh/nih-plug.git cargo-nih-plug
```

Once that's installed, you can compile and bundle plugins using:

```shell
cargo nih-plug bundle <package_name> --release
```
