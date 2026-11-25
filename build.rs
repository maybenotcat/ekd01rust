#[path = "build-support/identity.rs"]
mod identity;
#[path = "build-support/localization.rs"]
mod localization;
#[path = "build-support/revision.rs"]
mod revision;

fn main() {
    let root = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    revision::generate(&root);
    localization::generate(&root);
    let script = root.join("platform/link.ld");
    println!("cargo:rustc-link-arg=-T{}", script.display());
    identity::generate(
        &root,
        &[
            "src",
            "platform",
            "mt2502-hal",
            "ekd01-board/src",
            "boot-contract",
            "ui",
            "translations",
            "assets",
            "config",
            "third_party",
            "firmware/original",
        ],
        &[
            "Cargo.toml",
            "ekd01-board/Cargo.toml",
            "Cargo.lock",
            "build.rs",
            "build-support/identity.rs",
            "build-support/revision.rs",
            "build-support/localization.rs",
            "rust-toolchain.toml",
            ".cargo/config.toml",
        ],
        "build-id.json",
    );
    slint_build::compile_with_config(
        "ui/app.slint",
        slint_build::CompilerConfiguration::new()
            .with_bundled_translations("translations")
            .with_default_translation_context(slint_build::DefaultTranslationContext::None)
            .embed_resources(slint_build::EmbedResourcesKind::EmbedForSoftwareRenderer),
    )
    .expect("compile embedded Slint UI");
}
