//! Embeds the git commit for Settings' About row, as Electron's `__GIT_SHA__`
//! did. An explicit BARD_COMMIT wins; outside a checkout it is "dev".

use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=BARD_COMMIT");
    // Rebuild when HEAD moves, so the hash is not stale.
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/refs/heads");
    let commit = std::env::var("BARD_COMMIT").ok().or_else(|| {
        let out = Command::new("git").args(["rev-parse", "--short", "HEAD"]).output().ok()?;
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
    });
    println!("cargo:rustc-env=BARD_COMMIT={}", commit.unwrap_or_else(|| "dev".into()));

    // The toolchain, for the version row's tooltip (Electron listed its
    // Electron, Chrome and Node versions there): "1.91.0" from rustc -V.
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into());
    let version = Command::new(rustc)
        .arg("--version")
        .output()
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).split_whitespace().nth(1).map(str::to_string))
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=BARD_RUSTC={version}");

    // The icon Explorer and the taskbar show for bard.exe.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=../../resources/windows");
        embed_resource::compile("../../resources/windows/bard.rc", embed_resource::NONE)
            .manifest_optional()
            .expect("embedding the Windows icon");
    }
}
