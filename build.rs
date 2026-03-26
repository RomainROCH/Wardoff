use std::env;
use std::path::PathBuf;

fn main() {
    let manifest_path =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap_or_default()).join("wardoff.manifest");
    let profile = env::var("PROFILE").unwrap_or_default();

    println!("cargo:rerun-if-changed={}", manifest_path.display());
    println!("cargo:rerun-if-env-changed=PROFILE");

    if cfg!(target_os = "windows") && profile == "release" {
        println!("cargo:rustc-link-arg-bin=wardoff=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg-bin=wardoff=/MANIFESTUAC:NO");
        println!(
            "cargo:rustc-link-arg-bin=wardoff=/MANIFESTINPUT:{}",
            manifest_path.display()
        );
    }
}
