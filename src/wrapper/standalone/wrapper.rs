use parking_lot::RwLock;

use crate::plugin::Plugin;

pub struct Wrapper<P: Plugin> {
    /// The wrapped plugin instance.
    plugin: RwLock<P>,
}

impl<P: Plugin> Wrapper<P> {
    /// Instantiate a new instance of the standalone wrapper.
    //
    // TODO: This should take some arguments for the audio and MIDI IO.
    pub fn new() -> Self {
        Wrapper {
            plugin: RwLock::new(P::default()),
        }
    }
}
