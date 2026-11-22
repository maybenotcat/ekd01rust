//! Original-layout, unsigned GFH packaging, matching tools/pack_image.py.
use crate::{Result, elf, read};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::{fs, path::Path};

const IMAGE: &str = "EKD01RUST.bin";
const SCATTER: &str = "SOUTHBASE2502C_WT_11C_BB.cfg";
const OLD_IMAGE: &[u8] = b"EKD01_CAN_BF_PCB01_gsm_MT2502_S00.EKD01_CAN_BF_N22.bin";
const LOADERS: [&str; 2] = [
    "EKD01_CAN_BF_BOOTLOADER_V005_MT2502_EKD01_CAN_BF_N22.bin",
    "EXT_BOOTLOADER",
];

fn sha256(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}
fn json_file(path: &Path) -> Result<Value> {
    let data = read(path)?;
    let data = data.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&data);
    serde_json::from_slice(data).map_err(|error| format!("{}: {error}", path.display()))
}
fn validate_file(directory: &Path, entry: &Value) -> Result<()> {
    let name = entry["path"]
        .as_str()
        .ok_or("Missing original artifact path")?;
    let path = Path::new(name);
    if path
        .components()
        .any(|part| !matches!(part, std::path::Component::Normal(_)))
    {
        return Err("Invalid original artifact path".into());
    }
    let data = read(&directory.join(path))?;
    if entry["bytes"].as_u64() != Some(data.len() as u64)
        || entry["sha256"].as_str() != Some(sha256(&data).as_str())
    {
        return Err(format!("Original artifact changed: {name}"));
    }
    Ok(())
}
fn validate_originals(directory: &Path) -> Result<()> {
    let manifest = json_file(&directory.join("manifest.json"))?;
    for entry in manifest["files"]
        .as_array()
        .ok_or("Missing original artifact manifest")?
    {
        validate_file(directory, entry)?;
    }
    let mut patch = json_file(&directory.join("bt-patch.json"))?;
    if patch["header_bytes"].as_u64() != Some(30) {
        return Err("Controller patch data or metadata changed".into());
    }
    patch["path"] = json!("bt-patch.bin");
    validate_file(directory, &patch)
}

fn scatter(template: &[u8]) -> Result<Vec<u8>> {
    let locations: Vec<_> = template
        .windows(OLD_IMAGE.len())
        .enumerate()
        .filter_map(|(offset, bytes)| (bytes == OLD_IMAGE).then_some(offset))
        .collect();
    if locations.len() != 1 {
        return Err("Unexpected original scatter".into());
    }
    let offset = locations[0];
    let mut output = template[..offset].to_vec();
    output.extend_from_slice(IMAGE.as_bytes());
    output.extend_from_slice(&template[offset + OLD_IMAGE.len()..]);
    Ok(output)
}

fn image(mut header: Vec<u8>, mut payload: Vec<u8>) -> Result<Vec<u8>> {
    payload.resize(payload.len().next_multiple_of(4), 0xff);
    if header.len() != 0x520 || &header[..4] != b"MMM\x01" || header[27] != 0 {
        return Err("Expected original unsigned GFH".into());
    }
    let length = header.len() + payload.len();
    if length > 0x4b0000 {
        return Err("Application overlaps private storage".into());
    }
    for (index, value) in [length as u32, 0x4b0000, 0x520, 0, 0x520, 3]
        .iter()
        .enumerate()
    {
        header[32 + index * 4..36 + index * 4].copy_from_slice(&value.to_le_bytes());
    }
    header.extend(payload);
    Ok(header)
}

// Keep the Python packer's insertion order, indentation, ASCII escaping and
// native text-file newlines, so image.json is byte-identical on the same OS.
fn metadata(image: &[u8], identity: Option<Value>) -> Result<Vec<u8>> {
    let mut fields = Map::new();
    for (key, value) in [
        ("image", json!(IMAGE)),
        ("bytes", json!(image.len())),
        ("sha256", json!(sha256(image))),
        ("entry", json!(format!("{:08X}", elf::ENTRY))),
        ("signature", json!("NONE")),
        ("original_bootloaders", json!(true)),
        ("auxiliary_mcu_included", json!(false)),
        ("hardware_tested", json!(false)),
        ("source_port_complete", json!(true)),
        ("storage_begin", json!("00500000")),
        ("storage_end_exclusive", json!("00560000")),
    ] {
        fields.insert(key.into(), value);
    }
    if let Some(identity) = identity {
        for (key, value) in identity
            .as_object()
            .ok_or("Build identity must be a JSON object")?
        {
            fields.insert(key.clone(), value.clone());
        }
    }
    let text = serde_json::to_string_pretty(&fields).map_err(|error| error.to_string())?;
    let mut ascii = String::new();
    for character in text.chars() {
        if character.is_ascii() {
            ascii.push(character);
        } else {
            for unit in character.encode_utf16(&mut [0; 2]) {
                use std::fmt::Write;
                write!(ascii, "\\u{unit:04x}").unwrap();
            }
        }
    }
    ascii.push('\n');
    if cfg!(windows) {
        ascii = ascii.replace('\n', "\r\n");
    }
    Ok(ascii.into_bytes())
}

pub fn pack(root: &Path, elf: &Path, output: &Path, identity: &Path) -> Result<()> {
    let originals = root.join("firmware/original");
    validate_originals(&originals)?;
    // Validate all inputs before creating or overwriting any output files.
    let main = image(read(&originals.join("main-gfh.bin"))?, elf::payload(elf)?)?;
    let configuration = scatter(&read(&originals.join(SCATTER))?)?;
    let loaders = LOADERS
        .map(|name| read(&originals.join(name)))
        .into_iter()
        .collect::<Result<Vec<_>>>()?;
    let identity = if identity.exists() {
        Some(json_file(identity)?)
    } else {
        None
    };
    let build_id = identity
        .as_ref()
        .and_then(|value| value["build_id"].as_str())
        .unwrap_or("unknown")
        .to_string();
    let info = metadata(&main, identity)?;
    fs::create_dir_all(output).map_err(|error| format!("{}: {error}", output.display()))?;
    for (name, data) in [
        (IMAGE, main.as_slice()),
        (SCATTER, configuration.as_slice()),
        ("image.json", info.as_slice()),
    ]
    .into_iter()
    .chain(LOADERS.into_iter().zip(loaders.iter().map(Vec::as_slice)))
    {
        fs::write(output.join(name), data).map_err(|error| format!("{name}: {error}"))?;
    }
    println!(
        "Packed {}: {} bytes; BUILD_ID={}",
        output.join(IMAGE).display(),
        main.len(),
        build_id
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scatter_replaces_exactly_one_name_without_rewriting_newlines() {
        let template = [b"first\r\n".as_slice(), OLD_IMAGE, b"\r\nlast\n"].concat();
        assert_eq!(
            scatter(&template).unwrap(),
            b"first\r\nEKD01RUST.bin\r\nlast\n"
        );
        assert!(scatter(b"no filename").is_err());
        assert!(scatter(&[OLD_IMAGE, OLD_IMAGE].concat()).is_err());
    }
    #[test]
    fn gfh_preserves_original_bytes_outside_length_fields_and_pads_payload() {
        let mut header = vec![0x55; 0x520];
        header[..4].copy_from_slice(b"MMM\x01");
        header[27] = 0;
        let packed = image(header.clone(), vec![1, 2, 3, 4, 5]).unwrap();
        assert_eq!(&packed[..32], &header[..32]);
        assert_eq!(&packed[56..0x520], &header[56..]);
        assert_eq!(&packed[0x520..], &[1, 2, 3, 4, 5, 255, 255, 255]);
        assert_eq!(
            u32::from_le_bytes(packed[32..36].try_into().unwrap()),
            0x528
        );
        header[27] = 1;
        assert!(image(header, vec![]).is_err());
    }
}
