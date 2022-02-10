use anyhow::{bail, Context, Result};
use std::fs;
use std::path::Path;
use std::process::Command;

const USAGE_STRING: &str =
    "Usage: cargo xtask bundle <package> [--release] [--target <triple>] [--bundle-vst3]";

fn main() -> Result<()> {
    let project_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .context("Could not find project root")?;
    std::env::set_current_dir(project_root)
        .context("Could not change to project root directory")?;

    let mut args = std::env::args().skip(1);
    let command = args
        .next()
        .context(format!("Missing command name\n\n{USAGE_STRING}"))?;
    match command.as_str() {
        "bundle" => {
            let package = args
                .next()
                .context(format!("Missing package name\n\n{USAGE_STRING}"))?;
            let other_args: Vec<_> = args.collect();

            bundle(&package, other_args)
        }
        _ => bail!("Unknown command '{command}'\n\n{USAGE_STRING}"),
    }
}

// TODO: This probably needs more work for macOS. I don't know, I don't have a Mac.
fn bundle(package: &str, mut args: Vec<String>) -> Result<()> {
    let mut is_release_build = false;
    let mut bundle_vst3 = false;
    let mut cross_compile_target: Option<String> = None;
    for arg_idx in (0..args.len()).rev() {
        let arg = &args[arg_idx];
        match arg.as_str() {
            "--bundle-vst3" => {
                bundle_vst3 = true;
                args.remove(arg_idx);
            }
            "--release" => is_release_build = true,
            "--target" => {
                // When cross compiling we should generate the correct bundle type
                cross_compile_target = Some(
                    args.get(arg_idx + 1)
                        .context("Missing cross-compile target")?
                        .to_owned(),
                );
            }
            arg if arg.starts_with("--target=") => {
                cross_compile_target = Some(
                    arg.strip_prefix("--target=")
                        .context("Missing cross-compile target")?
                        .to_owned(),
                );
            }
            _ => (),
        }
    }

    let status = Command::new("cargo")
        .arg("build")
        .arg("-p")
        .arg(package)
        .args(args)
        .status()
        .context(format!("Could not call cargo to build {package}"))?;
    if !status.success() {
        bail!("Could not build {}", package);
    }

    let lib_path = Path::new(target_base(cross_compile_target.as_deref())?)
        .join(if is_release_build { "release" } else { "debug" })
        .join(library_basename(package, cross_compile_target.as_deref())?);
    if !lib_path.exists() {
        bail!("Could not find built library at '{}'", lib_path.display());
    }

    eprintln!();
    if bundle_vst3 {
        let vst3_lib_path = Path::new("target").join(vst3_bundle_library_name(
            package,
            cross_compile_target.as_deref(),
        )?);
        let vst3_bundle_home = vst3_lib_path
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap();

        fs::create_dir_all(vst3_lib_path.parent().unwrap())
            .context("Could not create bundle directory")?;
        fs::copy(&lib_path, &vst3_lib_path).context("Could not copy library to bundle")?;

        eprintln!("Created a VST3 bundle at '{}'", vst3_bundle_home.display());
    } else {
        eprintln!("Not creating any plugin bundles")
    }

    Ok(())
}

fn target_base(cross_compile_target: Option<&str>) -> Result<&'static str> {
    match cross_compile_target {
        Some("x86_64-unknown-linux-gnu") => Ok("target/x86_64-unknown-linux-gnu"),
        Some("x86_64-pc-windows-gnu") => Ok("target/x86_64-pc-windows-gnu"),
        Some("x86_64-apple-darwin") => Ok("target/x86_64-apple-darwin"),
        Some(target) => bail!("Unhandled cross-compilation target: {}", target),
        None => Ok("target"),
    }
}

fn library_basename(package: &str, cross_compile_target: Option<&str>) -> Result<String> {
    fn library_basename_linux(package: &str) -> String {
        format!("lib{package}.so")
    }
    fn library_basename_macos(package: &str) -> String {
        format!("lib{package}.dylib")
    }
    fn library_basename_windows(package: &str) -> String {
        format!("{package}.dll")
    }

    match cross_compile_target {
        Some("x86_64-unknown-linux-gnu") => Ok(library_basename_linux(package)),
        Some("x86_64-apple-darwin") => Ok(library_basename_macos(package)),
        Some("x86_64-pc-windows-gnu") => Ok(library_basename_windows(package)),
        Some(target) => bail!("Unhandled cross-compilation target: {}", target),
        None => {
            #[cfg(target_os = "linux")]
            return Ok(library_basename_linux(package));
            #[cfg(target_os = "macos")]
            return Ok(library_basename_macos(package));
            #[cfg(target_os = "windows")]
            return Ok(library_basename_windows(package));
        }
    }
}

// See https://developer.steinberg.help/display/VST/Plug-in+Format+Structure

fn vst3_bundle_library_name(package: &str, cross_compile_target: Option<&str>) -> Result<String> {
    fn vst3_bundle_library_name_linux_x86_64(package: &str) -> String {
        format!("{package}.vst3/Contents/x86_64-linux/{package}.so")
    }
    #[allow(dead_code)]
    fn vst3_bundle_library_name_linux_x86(package: &str) -> String {
        format!("{package}.vst3/Contents/i386-linux/{package}.so")
    }
    // TODO: This should be a Mach-O bundle, not sure how that works
    fn vst3_bundle_library_name_macos(package: &str) -> String {
        format!("{package}.vst3/Contents/MacOS/{package}")
    }
    fn vst3_bundle_library_name_windows_x86_64(package: &str) -> String {
        format!("{package}.vst3/Contents/x86_64-win/{package}.vst3")
    }
    #[allow(dead_code)]
    fn vst3_bundle_library_name_windows_x86(package: &str) -> String {
        format!("{package}.vst3/Contents/x86-win/{package}.vst3")
    }

    match cross_compile_target {
        Some("x86_64-unknown-linux-gnu") => Ok(vst3_bundle_library_name_linux_x86_64(package)),
        Some("x86_64-apple-darwin") => Ok(vst3_bundle_library_name_macos(package)),
        Some("x86_64-pc-windows-gnu") => Ok(vst3_bundle_library_name_windows_x86_64(package)),
        Some(target) => bail!("Unhandled cross-compilation target: {}", target),
        None => {
            #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
            return Ok(vst3_bundle_library_name_linux_x86_64(package));
            #[cfg(all(target_os = "linux", target_arch = "x86"))]
            return Ok(vst3_bundle_library_name_linux_x86(package));
            #[cfg(target_os = "macos")]
            return Ok(vst3_bundle_library_name_macos(package));
            #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
            return Ok(vst3_bundle_library_name_windows_x86_64(package));
            #[cfg(all(target_os = "windows", target_arch = "x86"))]
            return Ok(vst3_bundle_library_name_windows_x86(package));
        }
    }
}
