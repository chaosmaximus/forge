fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::compile_protos("proto/forge.proto")?;

    // Capture git short SHA at build time (best-effort — CI may not have .git)
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

    // Capture rustc version
    let rustc_version = std::process::Command::new("rustc")
        .args(["--version"])
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
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo::rustc-env=FORGE_RUSTC_VERSION={rustc_version}");

    // Capture target triple (unconditional — env!("FORGE_TARGET") in handler
    // would fail to compile if this is missing)
    let target = std::env::var("TARGET").unwrap_or_else(|_| "unknown".to_string());
    println!("cargo::rustc-env=FORGE_TARGET={target}");

    // Re-run build script when HEAD changes so git_sha stays current
    // in incremental builds.
    println!("cargo::rerun-if-changed=.git/HEAD");

    Ok(())
}
