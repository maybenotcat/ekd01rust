fn main() {
    println!("cargo:rerun-if-changed=protocol.json");
    let descriptor: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string("protocol.json").expect("read BLE descriptor"),
    )
    .expect("parse BLE descriptor");
    let mut constants = String::new();
    for (name, key) in [
        ("SERVICE_UUID", "serviceUuid"),
        ("CHARACTERISTIC_UUID", "characteristicUuid"),
    ] {
        let uuid = u128::from_str_radix(&descriptor[key].as_str().unwrap().replace('-', ""), 16)
            .expect("valid BLE UUID");
        constants.push_str(&format!(
            "pub const {name}: uuid::Uuid = uuid::Uuid::from_u128(0x{uuid:032x});\n"
        ));
    }
    constants.push_str(&format!(
        "pub const DEVICE_NAME: &str = {:?};\n",
        descriptor["name"].as_str().unwrap()
    ));
    let output = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    std::fs::write(output.join("protocol_constants.rs"), constants).expect("write BLE constants");
    println!("cargo:rerun-if-changed=ui/app.slint");
    slint_build::compile_with_config(
        "ui/app.slint",
        slint_build::CompilerConfiguration::new().with_style("fluent-dark".into()),
    )
    .expect("compile BLE client UI");
}
