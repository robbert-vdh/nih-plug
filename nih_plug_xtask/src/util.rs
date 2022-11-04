use anyhow::{Context, Result};
use std::fs;
use std::path::Path;
use std::process::Command;

use crate::CompilationTarget;

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

/// Either reflink `from` to `to` if `from` contains a single element, or combine multiple binaries
/// into `to` depending on the compilation target
pub fn reflink_or_combine<P: AsRef<Path>>(
    from: &[&Path],
    to: P,
    compilation_target: CompilationTarget,
) -> Result<()> {
    match (from, compilation_target) {
        ([], _) => anyhow::bail!("The 'from' slice is empty"),
        ([path], _) => {
            reflink(path, to.as_ref()).with_context(|| {
                format!(
                    "Could not copy {} to {}",
                    path.display(),
                    to.as_ref().display()
                )
            })?;
        }
        (paths, CompilationTarget::MacOSUniversal) => {
            lipo(paths, to.as_ref())
                .with_context(|| format!("Could not create universal binary from {paths:?}"))?;
        }
        _ => anyhow::bail!(
            "Combining multiple binaries is not yet supported for {compilation_target:?}."
        ),
    };

    Ok(())
}

/// Combine multiple macOS binaries into a universal macOS binary.
pub fn lipo(inputs: &[&Path], target: &Path) -> Result<()> {
    let status = Command::new("lipo")
        .arg("-create")
        .arg("-output")
        .arg(target)
        .args(inputs)
        .status()
        .context("Could not call the 'lipo' binary to create a universal macOS binary")?;
    if !status.success() {
        anyhow::bail!(
            "Could not call the 'lipo' binary to create a universal macOS binary from {inputs:?}",
        );
    } else {
        Ok(())
    }
}
