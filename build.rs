use std::env;
use std::path::PathBuf;

fn main() {
    let manifest_path =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap_or_default()).join("wardoff.manifest");
    let profile = env::var("PROFILE").unwrap_or_default();
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();

    println!("cargo:rerun-if-changed={}", manifest_path.display());
    println!("cargo:rerun-if-env-changed=PROFILE");
    println!("cargo:rerun-if-env-changed=CARGO_CFG_TARGET_OS");

    if target_os == "windows" && profile == "release" {
        let manifest_path = manifest_path
            .to_str()
            .expect("wardoff.manifest path must be valid UTF-8");

        let mut resources = winres::WindowsResource::new();
        resources.set_manifest_file(manifest_path);
        resources
            .compile()
            .expect("failed to compile the Windows release manifest resource");
    }
}
