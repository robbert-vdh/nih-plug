use nih_plug::prelude::*;

use byo_gui_gl::MyPlugin;

fn main() {
    nih_export_standalone::<MyPlugin>();
}
