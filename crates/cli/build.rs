// P3-4 Wave Z (Z11) per CC voice feedback §1.3 fix #2.
//
// Bake the CLI's compile-time git short-SHA into the binary so
// `forge-next doctor` can compare it against the running daemon's
// `git_sha` field. When they differ, the daemon binary is stale
// relative to the source the CLI was built from — typically because
// the user pulled new code, rebuilt the CLI, but forgot to restart
// the daemon. Surface that as a clear warning instead of letting
// users wonder why a fix they just merged isn't taking effect (the
// CC voice setup-and-isolation §1.3 reproducer: `cbd043f` /
// `ea76e82` / `ef99156` were in master but not in the daemon binary,
// and there was no upgrade prompt).

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let git_sha = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                String::from_utf8(o.stdout)
                    .ok()
                    .map(|s| s.trim().to_string())
            } else {
                None
            }
        })
        .unwrap_or_default();
    println!("cargo::rustc-env=FORGE_GIT_SHA={git_sha}");

    // P3-4 Wave Y (Y4): emit a pre-formatted version line so
    // `forge-next --version` can render `0.6.0 (38d7acc)` via a
    // single env! lookup. Doing the conditional formatting in build.rs
    // sidesteps the need for a const_format dep just to combine
    // env!() with option_env!() at compile time.
    let pkg_version = env!("CARGO_PKG_VERSION");
    let version_line = if git_sha.is_empty() {
        pkg_version.to_string()
    } else {
        format!("{pkg_version} ({git_sha})")
    };
    println!("cargo::rustc-env=FORGE_VERSION_LINE={version_line}");

    // Re-run when HEAD moves so the SHA stays fresh in incremental builds.
    println!("cargo::rerun-if-changed=.git/HEAD");

    Ok(())
}
