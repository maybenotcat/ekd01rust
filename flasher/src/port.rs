use crate::Result;
use serialport::{SerialPort, SerialPortType};
use std::{
    collections::BTreeSet,
    io, thread,
    time::{Duration, Instant},
};

fn port_names() -> Result<Vec<String>> {
    Ok(serialport::available_ports()
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|port| port.port_name)
        .collect())
}

/// Ignore existing ports, latch the first arrival, and retry it until it opens
/// or disappears. A disconnected initial port may be selected when it returns.
struct PortArrival {
    previous: BTreeSet<String>,
    selected: Option<String>,
}
impl PortArrival {
    fn new(existing: Vec<String>) -> Self {
        Self {
            previous: existing.into_iter().collect(),
            selected: None,
        }
    }
    fn observe(&mut self, ports: Vec<String>) -> Option<String> {
        let current: BTreeSet<_> = ports.iter().cloned().collect();
        if self
            .selected
            .as_ref()
            .is_some_and(|name| !current.contains(name))
        {
            self.selected = None;
        }
        if self.selected.is_none() {
            self.selected = ports.into_iter().find(|name| !self.previous.contains(name));
        }
        self.previous = current;
        self.selected.clone()
    }
}

pub fn list() -> Result<()> {
    for port in serialport::available_ports().map_err(|e| e.to_string())? {
        match port.port_type {
            SerialPortType::UsbPort(usb) => println!(
                "{} USB {:04X}:{:04X} {}",
                port.port_name,
                usb.vid,
                usb.pid,
                usb.product.unwrap_or_default()
            ),
            _ => println!("{}", port.port_name),
        }
    }
    Ok(())
}
pub fn wait(name: Option<&str>, timeout: Duration) -> Result<SerialLink> {
    let mut arrival = if name.is_none() {
        Some(PortArrival::new(port_names()?))
    } else {
        None
    };
    println!(
        "Waiting for {} ({:.0}s). Connect/reconnect display USB with main power disconnected.",
        name.unwrap_or("the first newly appearing serial port"),
        timeout.as_secs_f32()
    );
    let start = Instant::now();
    let mut last_error = String::new();
    while start.elapsed() < timeout {
        let candidate = if let Some(name) = name {
            Some(name.to_string())
        } else {
            arrival.as_mut().unwrap().observe(port_names()?)
        };
        if let Some(name) = candidate {
            match serialport::new(&name, 115200)
                .timeout(Duration::from_millis(50))
                .data_bits(serialport::DataBits::Eight)
                .parity(serialport::Parity::None)
                .stop_bits(serialport::StopBits::One)
                .flow_control(serialport::FlowControl::None)
                .open()
            {
                Ok(port) => {
                    port.clear(serialport::ClearBuffer::Input)
                        .map_err(|e| e.to_string())?;
                    println!("Opened {name}");
                    return Ok(SerialLink(port));
                }
                Err(error) => last_error = error.to_string(),
            }
        }
        thread::sleep(Duration::from_millis(100));
    }
    Err(format!(
        "USB port detection timed out. Close Varstrom and other users of the port. {last_error}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn names(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).into()).collect()
    }
    #[test]
    fn first_arrival_is_retained_while_driver_becomes_ready() {
        let mut arrival = PortArrival::new(names(&["COM6"]));
        assert_eq!(arrival.observe(names(&["COM6"])), None);
        assert_eq!(
            arrival.observe(names(&["COM6", "COM3"])),
            Some("COM3".into())
        );
        assert_eq!(
            arrival.observe(names(&["COM4", "COM3", "COM6"])),
            Some("COM3".into())
        );
    }
    #[test]
    fn an_existing_port_can_disappear_and_return_with_the_same_name() {
        let mut arrival = PortArrival::new(names(&["COM3", "COM6"]));
        assert_eq!(arrival.observe(names(&["COM6"])), None);
        assert_eq!(
            arrival.observe(names(&["COM3", "COM6"])),
            Some("COM3".into())
        );
        assert_eq!(arrival.observe(names(&["COM6"])), None);
        assert_eq!(
            arrival.observe(names(&["COM6", "COM7"])),
            Some("COM7".into())
        );
    }
}
pub struct SerialLink(pub Box<dyn SerialPort>);
impl io::Read for SerialLink {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.0.read(bytes)
    }
}
impl io::Write for SerialLink {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}
