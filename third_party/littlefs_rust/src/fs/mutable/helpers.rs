#[derive(Default)]
struct PendingRecord {
    name: Option<String>,
    ty: Option<FileType>,
    data: Option<FileData>,
    attrs: BTreeMap<u8, Vec<u8>>,
}

fn ascii_string(data: &[u8]) -> Result<String> {
    if data.iter().any(|byte| *byte == 0 || *byte > 0x7f) {
        return Err(Error::Utf8);
    }
    String::from_utf8(data.to_vec()).map_err(|_| Error::Utf8)
}

fn path_in_parent(parents: &[&str], name: &str) -> String {
    let parent = if parents.is_empty() {
        String::from("/")
    } else {
        format!("/{}", parents.join("/"))
    };
    join_path(&parent, name)
}
