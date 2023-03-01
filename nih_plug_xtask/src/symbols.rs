use anyhow::{bail, Context, Result};
use std::fs;
use std::path::Path;

/// Check whether a binary exports the specified symbol. Used to detect the plugin formats supported
/// by a plugin library. Returns an error if the binary cuuld not be read. This function will also
/// parse non-native binaries.
pub fn exported<P: AsRef<Path>>(binary: P, symbol: &str) -> Result<bool> {
    // Parsing the raw binary instead of relying on nm-like tools makes cross compiling a bit easier
    let bytes = fs::read(&binary)
        .with_context(|| format!("Could not read '{}'", binary.as_ref().display()))?;
    match goblin::Object::parse(&bytes)? {
        goblin::Object::Elf(obj) => Ok(obj
            .dynsyms
            .iter()
            // We don't filter by functions here since we need to export a constant for CLAP
            .any(|sym| !sym.is_import() && obj.dynstrtab.get_at(sym.st_name) == Some(symbol))),
        goblin::Object::Mach(obj) => {
            let obj = match obj {
                goblin::mach::Mach::Fat(arches) => match arches
                    .get(0)
                    .context("Fat Mach-O binary without any binaries")?
                {
                    goblin::mach::SingleArch::MachO(obj) => obj,
                    // THis shouldn't be hit
                    goblin::mach::SingleArch::Archive(_) => {
                        anyhow::bail!(
                            "'{}' contained an unexpected Mach-O archive",
                            binary.as_ref().display()
                        )
                    }
                },
                goblin::mach::Mach::Binary(obj) => obj,
            };

            // XXX: Why are all exported symbols on macOS prefixed with an underscore?
            let symbol = format!("_{symbol}");

            Ok(obj.exports()?.into_iter().any(|sym| sym.name == symbol))
        }
        goblin::Object::PE(obj) => Ok(obj.exports.iter().any(|sym| sym.name == Some(symbol))),
        obj => bail!("Unsupported object type: {:?}", obj),
    }
}
