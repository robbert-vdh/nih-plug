//! Utilities for Diopser's safe-mode mechanism.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use nih_plug_vizia::vizia::prelude::EventContext;

use crate::params::DiopserParams;

/// Restricts the ranges of several parameters when enabled. This makes it more difficult to
/// generate load resonances with Diopser's default settings.
#[derive(Debug, Clone)]
pub struct SafeModeClamper {
    /// Whether the safe mode toggle has been enabled.
    enabled: Arc<AtomicBool>,
}

impl SafeModeClamper {
    pub fn new(params: Arc<DiopserParams>) -> Self {
        Self {
            enabled: params.safe_mode.clone(),
        }
    }

    /// Return the current status of the safe mode swtich.
    pub fn status(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    /// Enable or disable safe mode. Enabling safe mode immediately clamps the parameters to their
    /// new restricted ranges.
    pub fn toggle(&self, cx: &mut EventContext) {
        // TODO: Restrict the parameter ranges when the button is enabled
        self.enabled.fetch_xor(true, Ordering::Relaxed);
    }

    /// Enable safe mode. Enabling safe mode immediately clamps the parameters to their new
    /// restricted ranges.
    pub fn enable(&self, cx: &mut EventContext) {
        // TODO: Restrict the parameter ranges when the button is enabled
        self.enabled.store(true, Ordering::Relaxed);
    }

    /// Disable safe mode.
    pub fn disable(&self, cx: &mut EventContext) {
        self.enabled.store(false, Ordering::Relaxed);
    }
}
