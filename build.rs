#[path = "build-support/identity.rs"]
mod identity;

fn main() {
    let root = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let script = root.join("platform/link.ld");
    println!("cargo:rustc-link-arg=-T{}", script.display());
    identity::generate(
        &root,
        &[
            "src",
            "platform",
            "mt2502-hal",
            "boot-contract",
            "ui",
            "assets",
            "config",
            "third_party",
            "firmware/original",
        ],
        &[
            "Cargo.toml",
            "Cargo.lock",
            "build.rs",
            "build-support/identity.rs",
            "rust-toolchain.toml",
            ".cargo/config.toml",
        ],
        "build-id.json",
    );
    slint_build::compile_with_config(
        "ui/app.slint",
        slint_build::CompilerConfiguration::new()
            .embed_resources(slint_build::EmbedResourcesKind::EmbedForSoftwareRenderer),
    )
    .expect("compile embedded Slint UI");
}
