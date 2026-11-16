fn main() {
    tauri_build::build();
    println!("cargo:rerun-if-env-changed=MODULINK_TEST_MANIFEST");
    println!("cargo:rerun-if-changed=tests/windows-test.manifest");
    // tauri-build embeds resources into the app binary, but not the lib test
    // harness. Windows tests need Common Controls v6 for TaskDialogIndirect.
    if std::env::var_os("MODULINK_TEST_MANIFEST").is_some()
        && std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        let manifest = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap())
            .join("tests/windows-test.manifest");
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
    }
}
