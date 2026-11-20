fn main() {
    // Windows: put the icon, manifest and version details into WeeklyScheduler.exe
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        #[cfg(windows)]
        embed();
        #[cfg(not(windows))]
        embed_cross();
    }
    println!("cargo:rerun-if-changed=assets/app.rc");
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-changed=assets/app.manifest");
}

#[cfg(windows)]
fn embed() {
    embed_resource::compile("assets/app.rc", embed_resource::NONE).manifest_required().unwrap();
}

/// Cross-compiling from Linux: run MinGW's windres ourselves and link the result.
#[cfg(not(windows))]
fn embed_cross() {
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let obj = out.join("app_res.o");
    let windres = std::env::var("WINDRES").unwrap_or_else(|_| "x86_64-w64-mingw32-windres".into());
    let ok = std::process::Command::new(&windres)
        .args(["--input", "assets/app.rc", "--output-format=coff", "--output"])
        .arg(&obj)
        .args(["--include-dir", "assets"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !ok {
        panic!("{windres} failed: install mingw-w64 to cross-compile for Windows");
    }
    println!("cargo:rustc-link-arg-bins={}", obj.display());
}
