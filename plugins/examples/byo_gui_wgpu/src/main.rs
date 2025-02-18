use nih_plug::prelude::*;

use byo_gui_wgpu::MyPlugin;

fn main() {
    nih_export_standalone::<MyPlugin>();
}
