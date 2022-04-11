use anyhow::{bail, Context};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

mod symbols;

/// Re-export for the main function.
pub use anyhow::Result;

/// The base birectory for the bundler's output.
const BUNDLE_HOME: &str = "target/bundled";

fn build_usage_string(command_name: &str) -> String {
    format!(
        "Usage:
  {command_name} bundle <package> [--release] [--target <triple>]
  {command_name} bundle -p <package1> -p <package2> ... [--release] [--target <triple>]"
    )
}

/// Any additional configuration that might be useful for creating plugin bundles, stored as
/// `bundler.toml` alongside the workspace's main `Cargo.toml` file.
type BundlerConfig = HashMap<String, PackageConfig>;

#[derive(Debug, Clone, Deserialize)]
struct PackageConfig {
    name: Option<String>,
}

/// The target we're generating a plugin for. This can be either the native target or a cross
/// compilation target, so to reduce redundancy when determining the correct bundle paths we'll use
/// an enum for this.
#[derive(Debug, Clone, Copy)]
pub enum CompilationTarget {
    Linux(Architecture),
    MacOS(Architecture),
    Windows(Architecture),
}

#[derive(Debug, Clone, Copy)]
pub enum Architecture {
    X86,
    X86_64,
    // There are also a ton of different 32-bit ARM architectures, we'll just pretend they don't
    // exist for now
    AArch64,
}

/// The main xtask entry point function. See the readme for instructions on how to use this.
pub fn main() -> Result<()> {
    let args = std::env::args().skip(1);
    main_with_args("cargo xtask", args)
}

/// The main xtask entry point function, but with custom command line arguments. `args` should not
/// contain the command name, so you should always skip at least one argument from
/// `std::env::args()` before passing it to this function.
pub fn main_with_args(command_name: &str, mut args: impl Iterator<Item = String>) -> Result<()> {
    chdir_workspace_root()?;

    let usage_string = build_usage_string(command_name);
    let command = args
        .next()
        .context(format!("Missing command name\n\n{usage_string}",))?;
    match command.as_str() {
        "bundle" => {
            // For convenience's sake we'll allow building multiple packages with `-p` just like
            // carg obuild, but you can also build a single package without specifying `-p`. Since
            // multiple packages can be built in parallel if we pass all of these flags to a single
            // `cargo build` we'll first build all of these packages and only then bundle them.
            let mut args = args.peekable();
            let mut packages = Vec::new();
            if args.peek().map(|s| s.as_str()) == Some("-p") {
                while args.peek().map(|s| s.as_str()) == Some("-p") {
                    packages.push(
                        args.nth(1)
                            .context(format!("Missing package name after -p\n\n{usage_string}"))?,
                    );
                }
            } else {
                packages.push(
                    args.next()
                        .context(format!("Missing package name\n\n{usage_string}"))?,
                );
            };
            let other_args: Vec<_> = args.collect();

            // As explained above, for efficiency's sake this is a two step process
            build(&packages, &other_args)?;

            bundle(&packages[0], &other_args)?;
            for package in packages.into_iter().skip(1) {
                bundle(&package, &other_args)?;
            }

            Ok(())
        }
        // This is only meant to be used by the CI, since using awk for this can be a bit spotty on
        // macOS
        "known-packages" => list_known_packages(),
        _ => bail!("Unknown command '{command}'\n\n{usage_string}"),
    }
}

/// Change the current directory into the Cargo workspace's root.
pub fn chdir_workspace_root() -> Result<()> {
    let xtask_project_dir = std::env::var("CARGO_MANIFEST_DIR")
        .context("'$CARGO_MANIFEST_DIR' was not set, are you running this binary directly?")?;
    let project_root = Path::new(&xtask_project_dir).parent().context(
        "'$CARGO_MANIFEST_DIR' has an unexpected value, are you running this binary directly?",
    )?;
    std::env::set_current_dir(project_root).context("Could not change to project root directory")
}

/// Build one or more packages using the provided `cargo build` arguments. This should be caleld
/// before callingq [`bundle()`]. This requires the current working directory to have been set to
/// the workspace's root using [`chdir_workspace_root()`].
pub fn build(packages: &[String], args: &[String]) -> Result<()> {
    let package_args = packages.iter().flat_map(|package| ["-p", package]);

    let status = Command::new("cargo")
        .arg("build")
        .args(package_args)
        .args(args)
        .status()
        .context(format!(
            "Could not call cargo to build {}",
            packages.join(", ")
        ))?;
    if !status.success() {
        bail!("Could not build {}", packages.join(", "));
    } else {
        Ok(())
    }
}

/// Bundle a package that was previoulsly built by a call to [`build()`] using the provided `cargo
/// build` arguments. These two functions are split up because building can be done in parallel by
/// Cargo itself while bundling is sequential. Options from the `bundler.toml` file in the
/// workspace's root are respected (see
/// <https://github.com/robbert-vdh/nih-plug/blob/master/bundler.toml>). This requires the current
/// working directory to have been set to the workspace's root using [`chdir_workspace_root()`].
pub fn bundle(package: &str, args: &[String]) -> Result<()> {
    let bundle_name = match load_bundler_config()?.and_then(|c| c.get(package).cloned()) {
        Some(PackageConfig { name: Some(name) }) => name,
        _ => package.to_string(),
    };

    let mut is_release_build = false;
    let mut cross_compile_target: Option<String> = None;
    for arg_idx in (0..args.len()).rev() {
        let arg = &args[arg_idx];
        match arg.as_str() {
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

    let compilation_target = compilation_target(cross_compile_target.as_deref())?;
    let lib_path = target_base(cross_compile_target.as_deref())?
        .join(if is_release_build { "release" } else { "debug" })
        .join(library_basename(package, compilation_target));
    if !lib_path.exists() {
        bail!("Could not find built library at '{}'", lib_path.display());
    }

    // We'll detect the pugin formats supported by the plugin binary and create bundled accordingly
    // NOTE: NIH-plug does not support VST2, but we'll support bundling VST2 plugins anyways because
    //       this bundler can also be used standalone.
    let bundle_clap = symbols::exported(&lib_path, "clap_entry")
        .with_context(|| format!("Could not parse '{}'", lib_path.display()))?;
    // We'll ignore the platofrm-specific entry points for VST2 plugins since there's no reason to
    // create a new Rust VST2 plugin that doesn't work in modern DAWs
    let bundle_vst2 = symbols::exported(&lib_path, "VSTPluginMain")
        .with_context(|| format!("Could not parse '{}'", lib_path.display()))?;
    let bundle_vst3 = symbols::exported(&lib_path, "GetPluginFactory")
        .with_context(|| format!("Could not parse '{}'", lib_path.display()))?;
    let bundled_plugin = bundle_clap || bundle_vst2 || bundle_vst3;

    eprintln!();
    if bundle_clap {
        let clap_bundle_library_name = clap_bundle_library_name(&bundle_name, compilation_target);
        let clap_lib_path = Path::new(BUNDLE_HOME).join(&clap_bundle_library_name);

        fs::create_dir_all(clap_lib_path.parent().unwrap())
            .context("Could not create CLAP bundle directory")?;
        reflink::reflink_or_copy(&lib_path, &clap_lib_path)
            .context("Could not copy library to CLAP bundle")?;

        // In contrast to VST3, CLAP only uses bundles on macOS, so we'll just take the first
        // component of the library name instead
        let clap_bundle_home = Path::new(BUNDLE_HOME).join(
            Path::new(&clap_bundle_library_name)
                .components()
                .next()
                .expect("Malformed CLAP library path"),
        );
        maybe_create_macos_bundle_metadata(package, &clap_bundle_home, compilation_target)?;

        eprintln!("Created a CLAP bundle at '{}'", clap_bundle_home.display());
    }
    if bundle_vst2 {
        let vst2_bundle_library_name = vst2_bundle_library_name(&bundle_name, compilation_target);
        let vst2_lib_path = Path::new(BUNDLE_HOME).join(&vst2_bundle_library_name);

        fs::create_dir_all(vst2_lib_path.parent().unwrap())
            .context("Could not create VST2 bundle directory")?;
        reflink::reflink_or_copy(&lib_path, &vst2_lib_path)
            .context("Could not copy library to VST2 bundle")?;

        // VST2 only uses bundles on macOS, so we'll just take the first component of the library
        // name instead
        let vst2_bundle_home = Path::new(BUNDLE_HOME).join(
            Path::new(&vst2_bundle_library_name)
                .components()
                .next()
                .expect("Malformed VST2 library path"),
        );
        maybe_create_macos_bundle_metadata(package, &vst2_bundle_home, compilation_target)?;

        eprintln!("Created a VST2 bundle at '{}'", vst2_bundle_home.display());
    }
    if bundle_vst3 {
        let vst3_lib_path =
            Path::new(BUNDLE_HOME).join(vst3_bundle_library_name(&bundle_name, compilation_target));

        fs::create_dir_all(vst3_lib_path.parent().unwrap())
            .context("Could not create VST3 bundle directory")?;
        reflink::reflink_or_copy(&lib_path, &vst3_lib_path)
            .context("Could not copy library to VST3 bundle")?;

        let vst3_bundle_home = vst3_lib_path
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        maybe_create_macos_bundle_metadata(package, vst3_bundle_home, compilation_target)?;

        eprintln!("Created a VST3 bundle at '{}'", vst3_bundle_home.display());
    }
    if !bundled_plugin {
        eprintln!("Not creating any plugin bundles because the package does not export any plugins")
    }

    Ok(())
}

/// This lists the packages configured in `bundler.toml`. This is only used as part of the CI when
/// bundling plugins.
pub fn list_known_packages() -> Result<()> {
    if let Some(config) = load_bundler_config()? {
        for package in config.keys() {
            println!("{package}");
        }
    }

    Ok(())
}

/// Load the `bundler.toml` file, if it exists. If it does exist but it cannot be parsed, then this
/// will return an error.
fn load_bundler_config() -> Result<Option<BundlerConfig>> {
    // We're already in the project root
    let bundler_config_path = Path::new("bundler.toml");
    if !bundler_config_path.exists() {
        return Ok(None);
    }

    let result = toml::from_str(
        &fs::read_to_string(&bundler_config_path)
            .with_context(|| format!("Could not read '{}'", bundler_config_path.display()))?,
    )
    .with_context(|| format!("Could not parse '{}'", bundler_config_path.display()))?;

    Ok(Some(result))
}

/// The target we're compiling for. This is used to determine the paths and options for creating
/// plugin bundles.
fn compilation_target(cross_compile_target: Option<&str>) -> Result<CompilationTarget> {
    match cross_compile_target {
        Some("i686-unknown-linux-gnu") => Ok(CompilationTarget::Linux(Architecture::X86)),
        Some("i686-apple-darwin") => Ok(CompilationTarget::MacOS(Architecture::X86)),
        Some("i686-pc-windows-gnu") | Some("i686-pc-windows-msvc") => {
            Ok(CompilationTarget::Windows(Architecture::X86))
        }
        Some("x86_64-unknown-linux-gnu") => Ok(CompilationTarget::Linux(Architecture::X86_64)),
        Some("x86_64-apple-darwin") => Ok(CompilationTarget::MacOS(Architecture::X86_64)),
        Some("x86_64-pc-windows-gnu") | Some("x86_64-pc-windows-msvc") => {
            Ok(CompilationTarget::Windows(Architecture::X86_64))
        }
        Some("aarch64-unknown-linux-gnu") => Ok(CompilationTarget::Linux(Architecture::AArch64)),
        Some("aarch64-apple-darwin") => Ok(CompilationTarget::MacOS(Architecture::AArch64)),
        Some("aarch64-pc-windows-gnu") | Some("aarch64-pc-windows-msvc") => {
            Ok(CompilationTarget::Windows(Architecture::AArch64))
        }
        Some(target) => bail!("Unhandled cross-compilation target: {}", target),
        None => {
            #[cfg(target_arch = "x86")]
            let architecture = Architecture::X86;
            #[cfg(target_arch = "x86_64")]
            let architecture = Architecture::X86_64;
            #[cfg(target_arch = "aarch64")]
            let architecture = Architecture::ARM64;

            #[cfg(target_os = "linux")]
            return Ok(CompilationTarget::Linux(architecture));
            #[cfg(target_os = "macos")]
            return Ok(CompilationTarget::MacOS(architecture));
            #[cfg(target_os = "windows")]
            return Ok(CompilationTarget::Windows(architecture));
        }
    }
}

/// The base directory for the compiled binaries. This does not use [`CompilationTarget`] as we need
/// to be able to differentiate between native and cross-compilation.
fn target_base(cross_compile_target: Option<&str>) -> Result<PathBuf> {
    match cross_compile_target {
        // Unhandled targets will already be handled in `compilation_target`
        Some(target) => Ok(Path::new("target").join(target)),
        None => Ok(PathBuf::from("target")),
    }
}

/// The file name of the compiled library for a `cdylib` crate.
fn library_basename(package: &str, target: CompilationTarget) -> String {
    // Cargo will replace dashes with underscores
    let lib_name = package.replace('-', "_");

    match target {
        CompilationTarget::Linux(_) => format!("lib{lib_name}.so"),
        CompilationTarget::MacOS(_) => format!("lib{lib_name}.dylib"),
        CompilationTarget::Windows(_) => format!("{lib_name}.dll"),
    }
}

/// The filename of the CLAP plugin for Linux and Windows, or the full path to the library file
/// inside of a CLAP bundle on macOS
fn clap_bundle_library_name(package: &str, target: CompilationTarget) -> String {
    match target {
        CompilationTarget::Linux(_) | CompilationTarget::Windows(_) => format!("{package}.clap"),
        CompilationTarget::MacOS(_) => format!("{package}.clap/Contents/MacOS/{package}"),
    }
}

/// On Linux and Windows VST2 plugins are regular library files, and on macOS they are put in a
/// bundle.
fn vst2_bundle_library_name(package: &str, target: CompilationTarget) -> String {
    match target {
        CompilationTarget::Linux(_) => format!("{package}.so"),
        CompilationTarget::MacOS(_) => format!("{package}.vst/Contents/MacOS/{package}"),
        CompilationTarget::Windows(_) => format!("{package}.dll"),
    }
}

/// The full path to the library file inside of a VST3 bundle, including the leading `.vst3`
/// directory.
///
/// See <https://developer.steinberg.help/display/VST/Plug-in+Format+Structure>.
fn vst3_bundle_library_name(package: &str, target: CompilationTarget) -> String {
    match target {
        CompilationTarget::Linux(Architecture::X86) => {
            format!("{package}.vst3/Contents/i386-linux/{package}.so")
        }
        CompilationTarget::Linux(Architecture::X86_64) => {
            format!("{package}.vst3/Contents/x86_64-linux/{package}.so")
        }
        CompilationTarget::Linux(Architecture::AArch64) => {
            format!("{package}.vst3/Contents/aarch64-linux/{package}.so")
        }
        CompilationTarget::MacOS(_) => format!("{package}.vst3/Contents/MacOS/{package}"),
        CompilationTarget::Windows(Architecture::X86) => {
            format!("{package}.vst3/Contents/x86-win/{package}.vst3")
        }
        CompilationTarget::Windows(Architecture::X86_64) => {
            format!("{package}.vst3/Contents/x86_64-win/{package}.vst3")
        }
        CompilationTarget::Windows(Architecture::AArch64) => {
            format!("{package}.vst3/Contents/arm_64-win/{package}.vst3")
        }
    }
}

/// If compiling for macOS, create all of the bundl-y stuff Steinberg and Apple require you to have.
///
/// This still requires you to move the dylib file to `{bundle_home}/Contents/macOS/{package}`
/// yourself first.
pub fn maybe_create_macos_bundle_metadata(
    package: &str,
    bundle_home: &Path,
    target: CompilationTarget,
) -> Result<()> {
    if !matches!(target, CompilationTarget::MacOS(_)) {
        return Ok(());
    }

    // TODO: May want to add bundler.toml fields for the identifier, version and signature at some
    //       point.
    fs::write(bundle_home.join("Contents").join("PkgInfo"), "BNDL????")
        .context("Could not create PkgInfo file")?;
    fs::write(
        bundle_home.join("Contents").join("Info.plist"),
        format!(r#"<?xml version="1.0" encoding="UTF-8"?>

<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist>
  <dict>
    <key>CFBundleExecutable</key>
    <string>{package}</string>
    <key>CFBundleIconFile</key>
    <string></string>
    <key>CFBundleIdentifier</key>
    <string>com.nih-plug.{package}</string>
    <key>CFBundleName</key>
    <string>{package}</string>
    <key>CFBundleDisplayName</key>
    <string>{package}</string>
    <key>CFBundlePackageType</key>
    <string>BNDL</string>
    <key>CFBundleSignature</key>
    <string>????</string>
    <key>CFBundleShortVersionString</key>
    <string>1.0.0</string>
    <key>CFBundleVersion</key>
    <string>1.0.0</string>
    <key>NSHumanReadableCopyright</key>
    <string></string>
    <key>NSHighResolutionCapable</key>
    <true/>
  </dict>
</plist>
"#),
    )
    .context("Could not create Info.plist file")?;

    Ok(())
}
