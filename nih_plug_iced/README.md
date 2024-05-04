# NIH-plug: iced support

This provides an adapter to create [iced](https://github.com/iced-rs/iced) based
GUIs with NIH-plug through
[iced_baseview](https://github.com/BillyDM/iced_baseview).

By default this targets OpenGL as wgpu causes segfaults on a number of
configurations. To use wgpu instead, include the crate with the following
options:

```toml
nih_plug_iced = { git = "https://github.com/robbert-vdh/nih-plug.git", default-features = false, features = ["wgpu"] }
```

Iced has many more optional features. Check the `Cargo.toml` file for more
information.
