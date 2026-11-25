#[path = "../../build-support/localization.rs"]
mod localization;
fn main() {
    let root =
        std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    localization::generate(&root);
    // slint-build uses the Cargo package name as gettext domain. Copy the exact
    // firmware catalogs into OUT_DIR under the native test package's domain.
    let bundle =
        std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("translations");
    let registry = localization::parse_registry(
        &std::fs::read_to_string(root.join("translations/languages.txt")).unwrap(),
    )
    .unwrap();
    for locale in registry {
        if locale.code == "en" {
            continue;
        }
        let destination = bundle.join(&locale.code).join("LC_MESSAGES");
        std::fs::create_dir_all(&destination).unwrap();
        std::fs::copy(
            root.join("translations")
                .join(&locale.code)
                .join("LC_MESSAGES/ekd01rust.po"),
            destination.join("ekd01rust-parity.po"),
        )
        .unwrap();
    }
    slint_build::compile_with_config(
        "localization.slint",
        slint_build::CompilerConfiguration::new()
            .with_bundled_translations(bundle)
            .with_default_translation_context(slint_build::DefaultTranslationContext::None),
    )
    .expect("native translation probe");
}
