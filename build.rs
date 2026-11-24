use std::{
    fs,
    path::{Path, PathBuf},
};

fn source_files(directory: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(directory).expect("read build inputs") {
        let path = entry.expect("build input").path();
        if path.is_dir() {
            source_files(&path, files);
        } else if matches!(
            path.extension().and_then(|value| value.to_str()),
            Some(
                "rs" | "toml"
                    | "json"
                    | "slint"
                    | "S"
                    | "ld"
                    | "txt"
                    | "ttf"
                    | "svg"
                    | "bin"
                    | "cfg"
            )
        ) {
            files.push(path);
        }
    }
}

fn build_identity() {
    let root = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let mut files = Vec::new();
    for directory in [
        "src",
        "platform",
        "bootloader",
        "boot-contract",
        "ui",
        "assets",
        "config",
        "third_party",
        "firmware/original",
    ] {
        println!("cargo:rerun-if-changed={directory}");
        source_files(&root.join(directory), &mut files);
    }
    for name in [
        "Cargo.toml",
        "Cargo.lock",
        "build.rs",
        "rust-toolchain.toml",
        ".cargo/config.toml",
    ] {
        println!("cargo:rerun-if-changed={name}");
        files.push(root.join(name));
    }
    files.sort();
    let mut fingerprint = 0xcbf29ce484222325u64;
    for path in files {
        let name = path
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let contents = fs::read(&path).expect("read build identity input");
        for byte in name.bytes().chain([0]).chain(contents).chain([0]) {
            fingerprint = (fingerprint ^ byte as u64).wrapping_mul(0x100000001b3);
        }
    }
    let identity = format!("{fingerprint:016x}");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    fs::write(
        out.join("build_id.rs"),
        format!("pub const BUILD_ID: &str = \"{identity}\";\n"),
    )
    .expect("write Cargo build identity");
    let generated = root.join("build/generated");
    fs::create_dir_all(&generated).expect("create build metadata directory");
    fs::write(
        generated.join("build-id.json"),
        format!("{{\"build_id\":\"{identity}\",\"source_fingerprint_algorithm\":\"fnv1a64\"}}\n"),
    )
    .expect("write packaging identity");
}

fn main() {
    let script = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap())
        .join("platform/link.ld");
    println!("cargo:rustc-link-arg=-T{}", script.display());
    build_identity();
    println!("cargo:rerun-if-changed=platform/link.ld");
    println!("cargo:rerun-if-changed=ui/app.slint");
    slint_build::compile_with_config(
        "ui/app.slint",
        slint_build::CompilerConfiguration::new()
            .embed_resources(slint_build::EmbedResourcesKind::EmbedForSoftwareRenderer),
    )
    .expect("compile embedded Slint UI");
}
