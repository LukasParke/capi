fn main() {
    // CEC C shim (compiled when present; CecFfi appended the link block below).
    let shim = "src/cec/shim.c";
    if std::path::Path::new(shim).exists() {
        let mut b = cc::Build::new();
        b.file(shim).include("/usr/include").flag("-std=c11");
        b.compile("capicec");
        println!("cargo:rerun-if-changed={shim}");
    }

    // Compile-time version: strict-semver git describe if available, else the
    // package version. Mirrors the Go Makefile's VERSION logic.
    let version = std::process::Command::new("git")
        .args([
            "describe",
            "--tags",
            "--match",
            "v[0-9]*.[0-9]*.[0-9]*",
            "--exclude",
            "*-*",
            "--always",
            "--dirty",
        ])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("v{}", env!("CARGO_PKG_VERSION")));
    println!("cargo:rustc-env=CAPI_VERSION={version}");
    println!("cargo:rerun-if-changed=build.rs");
    cec_link_config();
}

// --- appended by CecFfi: libcec link configuration ---
fn cec_link_config() {
    println!("cargo:rustc-link-lib=dylib=cec");
    println!("cargo:rustc-link-lib=dylib=p8-platform");
    if let Ok(paths) = std::env::var("PKG_CONFIG_PATH") {
        for p in std::env::split_paths(&paths) {
            if p.as_os_str().is_empty() {
                continue;
            }
            println!("cargo:rustc-link-search=native={}", p.display());
        }
    }
}
