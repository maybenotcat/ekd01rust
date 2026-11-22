use alloc::{format, string::String, vec::Vec};

use crate::types::{Error, Result};

/// Splits littlefs-style absolute or relative paths into components.
///
/// Empty paths and `/` are rejected here because callers either handle root
/// specially or need a concrete file/directory name. Repeated separators and a
/// trailing slash are normalized away to match littlefs C path handling: C
/// accepts `docs//note` as the same visible path as `docs/note`.
pub(crate) fn components(path: &str) -> Result<Vec<&str>> {
    let parts = path
        .split('/')
        .filter(|component| !component.is_empty())
        .collect::<Vec<_>>();
    if parts.is_empty() {
        return Err(Error::InvalidPath);
    }
    Ok(parts)
}

/// Normalizes a directory path for recursive walking. The public API accepts
/// both `docs` and `/docs`, but manifests and recursive joins use absolute
/// paths to stay comparable with the C inspector.
pub(crate) fn normalize_dir_path(path: &str) -> Result<String> {
    if path.is_empty() || path == "/" {
        return Ok(String::from("/"));
    }

    let parts = components(path)?;
    Ok(format!("/{}", parts.join("/")))
}

/// Joins an already-normalized parent path and a child name.
pub(crate) fn join_path(parent: &str, name: &str) -> String {
    if parent == "/" {
        format!("/{name}")
    } else {
        format!("{parent}/{name}")
    }
}
