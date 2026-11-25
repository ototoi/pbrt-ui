//! `cargo xtask dist [--dev] [--bundle-r4] [OUT [RENDERER]]`: the release
//! folder, the same on Linux, macOS and Windows.
//!
//! Installs `pbrt-ui`, `pbrt-ui-host`, `pbrt-ui-ctl`, `pbrt-ui-mcp` and
//! `pbrt-ui-capture` side by side (the host discovers its siblings).
//! `--bundle-r4` builds the pinned pbrt-r4 from crates.io with its own
//! lockfile (its exact pins conflict with this workspace) and installs it,
//! with its license, beside them. RENDERER bundles an existing binary instead.
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

/// Version of the bundled pbrt-r4 renderer.
const R4_VERSION: &str = "4.5.4";
const BINARIES: [&str; 5] = [
    "pbrt-ui",
    "pbrt-ui-host",
    "pbrt-ui-ctl",
    "pbrt-ui-mcp",
    "pbrt-ui-capture",
];

fn main() -> ExitCode {
    match run(env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("xtask: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Vec<String>) -> Result<(), String> {
    let mut args = args.into_iter();
    if args.next().as_deref() != Some("dist") {
        return Err("usage: cargo xtask dist [--dev] [--bundle-r4] [OUT [RENDERER]]".into());
    }
    let (mut dev, mut bundle_r4, mut paths) = (false, false, Vec::new());
    for arg in args {
        match arg.as_str() {
            "--dev" => dev = true,
            "--bundle-r4" => bundle_r4 = true,
            flag if flag.starts_with("--") => return Err(format!("unknown option {flag}")),
            _ => paths.push(PathBuf::from(arg)),
        }
    }
    // tools/xtask → the workspace root; not canonicalized (Windows would get `\\?\` paths).
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .ok_or("no workspace root")?
        .to_path_buf();
    let output = paths
        .first()
        .cloned()
        .unwrap_or_else(|| root.join("target/dist"));
    let mut renderer = paths.get(1).cloned();
    if paths.len() > 2 || (bundle_r4 && renderer.is_some()) {
        return Err("--bundle-r4 and RENDERER are exclusive; at most OUT and RENDERER".into());
    }

    let mut build = cargo();
    build.current_dir(&root).args(["build", "--locked"]);
    if !dev {
        build.arg("--release");
    }
    for package in ["desktop", "core", "cli", "mcp", "scene-view"] {
        build.args(["-p", package]);
    }
    status(&mut build)?;

    let mut r4_license = None;
    if bundle_r4 {
        let r4_root = root.join(format!("target/pbrt-r4-{R4_VERSION}"));
        status(
            cargo()
                .args(["install", "pbrt-r4", "--locked", "--version"])
                .arg(format!("={R4_VERSION}"))
                .arg("--root")
                .arg(&r4_root),
        )?;
        renderer = Some(r4_root.join("bin").join(exe("pbrt-r4")));
        // Apache-2.0: the bundled binary ships with its license.
        r4_license = Some(registry_license()?);
    }

    fs::create_dir_all(&output).map_err(|e| format!("{}: {e}", output.display()))?;
    let profile = root
        .join("target")
        .join(if dev { "debug" } else { "release" });
    for binary in BINARIES {
        copy(&profile.join(exe(binary)), &output.join(exe(binary)))?;
    }
    if let Some(renderer) = renderer {
        copy(&renderer, &output.join(exe("pbrt-r4")))?;
    }
    if let Some(license) = r4_license {
        copy(&license, &output.join("LICENSE-pbrt-r4"))?;
    }
    for file in ["README.md", "LICENSE", "NOTICE"] {
        copy(&root.join(file), &output.join(file))?;
    }
    println!("Built distribution: {}", output.display());
    Ok(())
}

fn cargo() -> Command {
    Command::new(env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
}

fn status(command: &mut Command) -> Result<(), String> {
    let status = command.status().map_err(|e| format!("{command:?}: {e}"))?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| format!("{command:?} failed: {status}"))
}

fn exe(name: &str) -> String {
    format!("{name}{}", env::consts::EXE_SUFFIX)
}

fn copy(from: &Path, to: &Path) -> Result<(), String> {
    fs::copy(from, to)
        .map(drop)
        .map_err(|e| format!("copy {} -> {}: {e}", from.display(), to.display()))
}

/// `cargo install` keeps the unpacked crate under `$CARGO_HOME/registry/src/<index>/`.
fn registry_license() -> Result<PathBuf, String> {
    let cargo_home = env::var_os("CARGO_HOME").map(PathBuf::from).or_else(|| {
        env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
            .map(|home| PathBuf::from(home).join(".cargo"))
    });
    let source = cargo_home
        .map(|home| home.join("registry/src"))
        .ok_or("cannot locate CARGO_HOME")?;
    fs::read_dir(&source)
        .map_err(|e| format!("{}: {e}", source.display()))?
        .flatten()
        .map(|index| index.path().join(format!("pbrt-r4-{R4_VERSION}/LICENSE")))
        .find(|license| license.is_file())
        .ok_or_else(|| {
            format!(
                "pbrt-r4 {R4_VERSION} LICENSE not found under {}",
                source.display()
            )
        })
}
