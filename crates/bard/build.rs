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
}
