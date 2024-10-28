use gain_gui_iced::Gain;
use nih_plug::prelude::*;
use nih_plug::wrapper::standalone::nih_export_standalone;

fn main() {
    nih_export_standalone::<Gain>();
}
