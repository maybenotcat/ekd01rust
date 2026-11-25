//! Independent source identities for the application and bootloader.
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

fn write_if_changed(path: &Path, data: &str) {
    if fs::read(path).ok().as_deref() != Some(data.as_bytes()) {
        fs::write(path, data).expect("write build identity");
    }
}

pub fn generate(root: &Path, directories: &[&str], extra_files: &[&str], metadata_name: &str) {
    let mut files = Vec::new();
    for name in directories {
        let directory = root.join(name);
        println!("cargo:rerun-if-changed={}", directory.display());
        source_files(&directory, &mut files);
    }
    for name in extra_files {
        let path = root.join(name);
        println!("cargo:rerun-if-changed={}", path.display());
        files.push(path);
    }
    files.sort();
    files.dedup();
    let mut fingerprint = 0xcbf29ce484222325u64;
    for path in files {
        let name = path
            .strip_prefix(root)
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
    write_if_changed(
        &out.join("build_id.rs"),
        &format!("pub const BUILD_ID: &str = \"{identity}\";\n"),
    );
    let generated = root.join("build/generated");
    fs::create_dir_all(&generated).expect("create build metadata directory");
    write_if_changed(
        &generated.join(metadata_name),
        &format!("{{\"build_id\":\"{identity}\",\"source_fingerprint_algorithm\":\"fnv1a64\"}}\n"),
    );
}
