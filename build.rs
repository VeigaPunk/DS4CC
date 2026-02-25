fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default() == "windows" {
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let res = format!("{}/assets/ds4cc.res", manifest_dir);
        println!("cargo:rustc-link-arg={res}");
        // Re-run build script if the resource file changes
        println!("cargo:rerun-if-changed=assets/ds4cc.res");
        println!("cargo:rerun-if-changed=assets/icon.ico");
    }
}
