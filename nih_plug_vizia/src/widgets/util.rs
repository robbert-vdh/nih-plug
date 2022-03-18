//! Utilities for writing VIZIA widgets.

use vizia::Modifiers;

/// An extension trait for [`Modifiers`] that adds platform-independent getters.
pub trait ModifiersExt {
    /// Returns true if the Command (on macOS) or Ctrl (on any other platform) key is pressed.
    fn command(&self) -> bool;

    /// Returns true if the Alt (or Option on macOS) key is pressed.
    fn alt(&self) -> bool;

    /// Returns true if the Shift key is pressed.
    fn shift(&self) -> bool;
}

impl ModifiersExt for Modifiers {
    fn command(&self) -> bool {
        #[cfg(target_os = "macos")]
        let result = self.contains(Modifiers::LOGO);

        #[cfg(not(target_os = "macos"))]
        let result = self.contains(Modifiers::CTRL);

        result
    }

    fn alt(&self) -> bool {
        self.contains(Modifiers::ALT)
    }

    fn shift(&self) -> bool {
        self.contains(Modifiers::SHIFT)
    }
}
