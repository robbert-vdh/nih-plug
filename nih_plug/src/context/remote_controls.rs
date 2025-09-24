//! A context for defining plugin-specific [remote
//! pages](https://github.com/free-audio/clap/blob/main/include/clap/ext/draft/remote-controls.h)
//! for CLAP plugins.

use crate::prelude::Param;

/// A context for defining plugin-specific [remote
/// pages](https://github.com/free-audio/clap/blob/main/include/clap/ext/draft/remote-controls.h)
/// for CLAP plugins.
///
/// These pages can contain references to up to eight parameters, but if the plugin defines more
/// parameters for a page then the pages are automatically split.
pub trait RemoteControlsContext {
    type Section: RemoteControlsSection;

    /// Define a section containing one or more remote control pages. This can be used to group
    /// remote control pages together. For instance, because an oscillator has so many parameters
    /// that it needs to span multiple pages, or to group the parameters for both filters into a
    /// single section.
    fn add_section(&mut self, name: impl Into<String>, f: impl FnOnce(&mut Self::Section));
}

/// A section or group of parameter pages. Empty sections will not be visible when using the plugin.
pub trait RemoteControlsSection {
    type Page: RemoteControlsPage;

    /// Add a named parameter page to the section. See the documentation of [`RemoteControlsPage`]
    /// for more information.
    fn add_page(&mut self, name: impl Into<String>, f: impl FnOnce(&mut Self::Page));
}

/// A page containing references to up to eight parameters. If the number of slots used exceeds
/// eight, then the page is split automatically. In that case the split page will have indices
/// appended to it. For example, the `Lengty Params Page` defining 16 parameters will become `Lengty
/// Params Page 1` and `Lengthy Params Page 2`.
pub trait RemoteControlsPage {
    // Add a reference to one of the plugin's parameters to the page.
    fn add_param(&mut self, param: &impl Param);

    // Add an empty space on the page. Can be useful for grouping and aligning parameters within a
    // page.
    fn add_spacer(&mut self);
}
