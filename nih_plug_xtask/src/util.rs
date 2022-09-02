use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

/// Acts the same as [`reflink::reflink_or_copy()`], but it removes existing files first. This works
/// around a limitation of macOS that the reflink crate also applies to other platforms to stay
/// consistent. See the [`reflink`] crate documentation or #26 for more information.
pub fn reflink<P: AsRef<Path>, Q: AsRef<Path>>(from: P, to: Q) -> Result<Option<u64>> {
    let to = to.as_ref();
    if to.exists() {
        fs::remove_file(to).context("Could not remove file before reflinking")?;
    }

    reflink::reflink_or_copy(from, to).context("Could not reflink or copy file")
}
