fn main() {
    let script =
        std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("link.ld");
    println!("cargo:rustc-link-arg=-T{}", script.display());
    println!("cargo:rerun-if-changed=link.ld");
    println!("cargo:rerun-if-changed=entry.S");
    println!("cargo:rerun-if-changed=../src/soc/flash.rs");
    println!("cargo:rerun-if-changed=../src/soc/registers.rs");
}
