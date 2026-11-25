#[path = "../build-support/identity.rs"]
mod identity;

fn main() {
    let manifest = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let root = manifest.parent().unwrap();
    let script =
        std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("link.ld");
    println!("cargo:rustc-link-arg=-T{}", script.display());
    println!("cargo:rerun-if-changed=link.ld");
    println!("cargo:rerun-if-changed=entry.S");
    identity::generate(
        root,
        &[
            "bootloader",
            "mt2502-hal",
            "boot-contract",
            "third_party/embassy-sync",
            "third_party/embassy-hal-internal",
        ],
        &[
            "Cargo.toml",
            "Cargo.lock",
            "build-support/identity.rs",
            "rust-toolchain.toml",
            ".cargo/config.toml",
        ],
        "boot-build-id.json",
    );
}
