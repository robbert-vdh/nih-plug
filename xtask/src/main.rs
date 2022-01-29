use anyhow::{bail, Context, Result};
use std::fs;
use std::path::Path;
use std::process::Command;

const USAGE_STRING: &'static str = "Usage: cargo xtask bundle <target> [--release] [--bundle-vst3]";

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
            let target = args
                .next()
                .context(format!("Missing target name\n\n{USAGE_STRING}"))?;
            let other_args: Vec<_> = args.collect();

            bundle(&target, other_args)
        }
        _ => bail!("Unknown command '{command}'\n\n{USAGE_STRING}"),
    }
}

// TODO: This probably needs more work for macOS. I don't know, I don't have a Mac.
fn bundle(target: &str, mut args: Vec<String>) -> Result<()> {
    let mut is_release_build = false;
    let mut bundle_vst3 = false;
    for arg_idx in (0..args.len()).rev() {
        let arg = &args[arg_idx];
        match arg.as_str() {
            "--bundle-vst3" => {
                bundle_vst3 = true;
                args.remove(arg_idx);
            }
            "--release" => is_release_build = true,
            _ => (),
        }
    }

    Command::new("cargo")
        .arg("build")
        .arg("-p")
        .arg(target)
        .args(args)
        .status()
        .context(format!("Could not build {target}"))?;

    let lib_path = Path::new("target")
        .join(if is_release_build { "release" } else { "debug" })
        .join(library_name(target));
    if !lib_path.exists() {
        bail!("Could not find built library at {}", lib_path.display());
    }

    if bundle_vst3 {
        let vst3_lib_path = Path::new("target").join(vst3_bundle_library_name(target));
        let vst3_bundle_home = vst3_lib_path.parent().unwrap().parent().unwrap();

        fs::create_dir_all(vst3_lib_path.parent().unwrap())
            .context("Could not create bundle directory")?;
        fs::copy(&lib_path, &vst3_lib_path).context("Could not copy library to bundle")?;

        eprintln!("Created a VST3 bundle at '{}'", vst3_bundle_home.display());
    } else {
        eprintln!("Not creating any plugin bundles")
    }

    Ok(())
}

#[cfg(target_os = "linux")]
fn library_name(target: &str) -> String {
    format!("lib{target}.so")
}

#[cfg(target_os = "macos")]
fn library_name(target: &str) -> String {
    format!("lib{target}.dylib")
}

#[cfg(target_os = "windows")]
fn library_name(target: &str) -> String {
    format!("{target}.dll")
}

// See https://developer.steinberg.help/display/VST/Plug-in+Format+Structure

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn vst3_bundle_library_name(target: &str) -> String {
    format!("{target}.vst3/x86_64-linux/{target}.so")
}

#[cfg(all(target_os = "linux", target_arch = "x86"))]
fn vst3_bundle_library_name(target: &str) -> String {
    format!("{target}.vst3/i386-linux/{target}.so")
}

// TODO: This should be a Mach-O bundle, not sure how that works
#[cfg(target_os = "macos")]
fn vst3_bundle_library_name(target: &str) -> String {
    format!("{target}.vst3/MacOS/{target}")
}

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
fn vst3_bundle_library_name(target: &str) -> String {
    format!("{target}.vst3/x86_64-win/{target}.vst3")
}

#[cfg(all(target_os = "windows", target_arch = "x86"))]
fn vst3_bundle_library_name(target: &str) -> String {
    format!("{target}.vst3/x86-win/{target}.vst3")
}
