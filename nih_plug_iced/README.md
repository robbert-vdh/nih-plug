# NIH-plug: iced support

This provides an adapter to create [iced](https://github.com/iced-rs/iced) based
GUIs with NIH-plug through
[iced_baseview](https://github.com/BillyDM/iced_baseview).

By default this targets [wgpu](https://github.com/gfx-rs/wgpu). To use OpenGL
instead, include the crate with the following options. Note that some iced
features may not be available in the OpenGL backend.

```toml
nih_plug_iced = { git = "https://github.com/robbert-vdh/nih-plug.git", default_features = false, features = ["opengl"] }
```

Iced has many more optional features. Check the `Cargo.toml` file for more
information.
