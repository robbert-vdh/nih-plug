# NIH-plug: bundler and other utilities

This is NIH-plug's `cargo xtask` command, but as a library. This way you can use
it in your own projects without having to either fork this repo or vendor the
binary into your own repo. This is necessary until Cargo supports [running
binaries from dependencies
directly](https://github.com/rust-lang/rfcs/pull/3168).

To use this, add an `xtask` binary to your project using `cargo new --bin xtask`
and add it to the Cargo workspace in your repository's main `Cargo.toml` file:

```toml
# Cargo.toml

[workspace]
members = ["xtask"]
```

Then add `nih_plug_xtask` to your new xtask package's dependencies:

```toml
# xtask/Cargo.toml

[dependencies]
nih_plug_xtask = { git = "https://github.com/robbert-vdh/nih-plug.git" }
```

Call `nih_plug_xtask`'s main function own xtask binary:

```rust
# xtask/src/main.rs

fn main() -> nih_plug_xtask::Result<()> {
    nih_plug_xtask::main()
}
```

And finally create a `.cargo/config` file in your repository and add a Cargo
alias so you can invoke the binary with `cargo xtask`:

```toml
# .cargo/config

[alias]
xtask = "run --package xtask --release --"
```
