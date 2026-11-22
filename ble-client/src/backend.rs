//! Bluetooth runs on its own Tokio thread; Slint is only touched by the UI thread.
use crate::protocol::{Assembler, CHARACTERISTIC_UUID, DEVICE_NAME, SERVICE_UUID, Snapshot};
use btleplug::{
    api::{Central, CharPropFlags, Manager as _, Peripheral as _, ScanFilter},
    platform::{Adapter, Manager, Peripheral},
};
use futures_util::StreamExt;
use std::{
    collections::BTreeMap,
    sync::mpsc::Sender,
    thread,
    time::{Duration, Instant},
};
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
    time::{interval, timeout},
};

pub enum Command {
    Scan,
    Connect(String),
    Disconnect,
}
#[derive(Clone)]
pub struct DeviceInfo {
    pub id: String,
    pub label: String,
}
pub enum Event {
    Devices(Vec<DeviceInfo>),
    State {
        status: String,
        scanning: bool,
        connected: bool,
        busy: bool,
        generation: u64,
    },
    Snapshot {
        value: Snapshot,
        generation: u64,
    },
    NotificationEnded(u64),
}

pub struct Worker {
    pub commands: mpsc::UnboundedSender<Command>,
    stop: Option<oneshot::Sender<()>>,
}
impl Worker {
    pub fn start(events: Sender<Event>) -> Self {
        let (commands, receiver) = mpsc::unbounded_channel();
        let (stop, stopping) = oneshot::channel();
        thread::Builder::new()
            .name("ekd01-ble".into())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        let _ = events.send(Event::State {
                            status: error.to_string(),
                            scanning: false,
                            connected: false,
                            busy: false,
                            generation: 0,
                        });
                        return;
                    }
                };
                runtime.block_on(async move {
                    let mut client = BleClient::new(events);
                    tokio::select! { _ = stopping => {}, _ = client.run(receiver) => {} }
                    client.release().await;
                    if let Some(adapter) = &client.adapter {
                        let _ = timeout(Duration::from_secs(2), adapter.stop_scan()).await;
                    }
                });
            })
            .expect("start BLE thread");
        Self {
            commands,
            stop: Some(stop),
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}

struct BleClient {
    adapter: Option<Adapter>,
    devices: BTreeMap<String, (DeviceInfo, Peripheral)>,
    active: Option<Peripheral>,
    listener: Option<JoinHandle<()>>,
    events: Sender<Event>,
    generation: u64,
    scanning: bool,
}
impl BleClient {
    fn new(events: Sender<Event>) -> Self {
        Self {
            adapter: None,
            devices: BTreeMap::new(),
            active: None,
            listener: None,
            events,
            generation: 0,
            scanning: false,
        }
    }
    fn state(&self, status: impl Into<String>, busy: bool) {
        let _ = self.events.send(Event::State {
            status: status.into(),
            scanning: self.scanning,
            connected: self.active.is_some() && self.listener.is_some(),
            busy,
            generation: self.generation,
        });
    }
    async fn ensure_adapter(&mut self) -> Result<(), String> {
        if self.adapter.is_none() {
            let manager = Manager::new().await.map_err(|e| e.to_string())?;
            self.adapter = Some(
                manager
                    .adapters()
                    .await
                    .map_err(|e| e.to_string())?
                    .into_iter()
                    .next()
                    .ok_or("No Bluetooth adapter found")?,
            );
        }
        Ok(())
    }
    async fn scan(&mut self) -> Result<(), String> {
        self.ensure_adapter().await?;
        let adapter = self.adapter.as_ref().unwrap();
        if self.scanning {
            adapter.stop_scan().await.map_err(|e| e.to_string())?;
            self.scanning = false;
            self.state("Scan stopped", false);
        } else {
            // Do not rely on service filters: EKD01 advertises its name, and
            // BlueZ may merge filters belonging to several discovery clients.
            adapter
                .start_scan(ScanFilter::default())
                .await
                .map_err(|e| e.to_string())?;
            self.scanning = true;
            self.state("Scanning for EKD01 displays…", false);
            self.refresh_devices().await?;
        }
        Ok(())
    }
    async fn refresh_devices(&mut self) -> Result<(), String> {
        let Some(adapter) = &self.adapter else {
            return Ok(());
        };
        for peripheral in adapter.peripherals().await.map_err(|e| e.to_string())? {
            let Some(properties) = peripheral.properties().await.map_err(|e| e.to_string())? else {
                continue;
            };
            if properties.local_name.as_deref() != Some(DEVICE_NAME)
                && !properties.services.contains(&SERVICE_UUID)
            {
                continue;
            }
            let id = peripheral.id().to_string();
            let rssi = properties
                .rssi
                .map(|rssi| format!("{rssi} dBm"))
                .unwrap_or_else(|| "RSSI unavailable".into());
            let label = format!(
                "{}   {}   {rssi}",
                properties.local_name.as_deref().unwrap_or(DEVICE_NAME),
                properties.address
            );
            self.devices
                .insert(id.clone(), (DeviceInfo { id, label }, peripheral));
        }
        let _ = self.events.send(Event::Devices(
            self.devices
                .values()
                .map(|(device, _)| device.clone())
                .collect(),
        ));
        Ok(())
    }
    async fn connect(&mut self, id: &str) -> Result<(), String> {
        let peripheral = self
            .devices
            .get(id)
            .ok_or("Select a discovered display first")?
            .1
            .clone();
        self.generation = self.generation.wrapping_add(1);
        self.state("Connecting and discovering telemetry service…", true);
        if let Some(adapter) = &self.adapter {
            adapter.stop_scan().await.map_err(|e| e.to_string())?;
        }
        self.scanning = false;
        self.release().await;
        // Keep ownership before awaiting so shutdown/error cleanup also closes
        // a partially established connection.
        self.active = Some(peripheral.clone());
        let setup = async {
            peripheral.connect().await.map_err(|e| e.to_string())?;
            peripheral
                .discover_services()
                .await
                .map_err(|e| e.to_string())?;
            let characteristic = peripheral
                .characteristics()
                .into_iter()
                .find(|characteristic| {
                    characteristic.uuid == CHARACTERISTIC_UUID
                        && characteristic.service_uuid == SERVICE_UUID
                })
                .ok_or("Telemetry characteristic not found")?;
            if !characteristic.properties.contains(CharPropFlags::NOTIFY) {
                return Err("Telemetry notifications are not supported".into());
            }
            let stream = peripheral
                .notifications()
                .await
                .map_err(|e| e.to_string())?;
            peripheral
                .subscribe(&characteristic)
                .await
                .map_err(|e| e.to_string())?;
            Ok::<_, String>((characteristic, stream))
        };
        let (characteristic, mut stream) = timeout(Duration::from_secs(20), setup)
            .await
            .map_err(|_| "Connection timed out".to_string())??;
        let initial = if characteristic.properties.contains(CharPropFlags::READ) {
            timeout(Duration::from_secs(3), peripheral.read(&characteristic))
                .await
                .ok()
                .and_then(Result::ok)
        } else {
            None
        };
        let events = self.events.clone();
        let generation = self.generation;
        self.listener = Some(tokio::spawn(async move {
            let start = Instant::now();
            let mut assembler = Assembler::default();
            if let Some(initial) = initial
                && let Some(value) = assembler.notification(&initial, start.elapsed())
            {
                let _ = events.send(Event::Snapshot { value, generation });
            }
            while let Some(notification) = stream.next().await {
                if notification.uuid != CHARACTERISTIC_UUID {
                    continue;
                }
                if let Some(value) = assembler.notification(&notification.value, start.elapsed()) {
                    if events.send(Event::Snapshot { value, generation }).is_err() {
                        break;
                    }
                }
            }
            let _ = events.send(Event::NotificationEnded(generation));
        }));
        self.state("Connected · Waiting for telemetry…", false);
        Ok(())
    }
    async fn release(&mut self) {
        if let Some(listener) = self.listener.take() {
            listener.abort();
        }
        if let Some(peripheral) = self.active.take() {
            let _ = timeout(Duration::from_secs(3), async {
                if let Some(characteristic) = peripheral
                    .characteristics()
                    .into_iter()
                    .find(|c| c.uuid == CHARACTERISTIC_UUID)
                {
                    let _ = peripheral.unsubscribe(&characteristic).await;
                }
                let _ = peripheral.disconnect().await;
            })
            .await;
        }
    }
    async fn run(&mut self, mut commands: mpsc::UnboundedReceiver<Command>) {
        let mut timer = interval(Duration::from_millis(500));
        loop {
            tokio::select! {
                command = commands.recv() => {
                    let Some(command) = command else { break; };
                    let result = match command {
                        Command::Scan => self.scan().await,
                        Command::Connect(id) => self.connect(&id).await,
                        Command::Disconnect => {
                            self.generation = self.generation.wrapping_add(1);
                            self.release().await; self.state("Disconnected", false); Ok(())
                        }
                    };
                    if let Err(error) = result {
                        self.release().await; self.state(error, false);
                    }
                },
                _ = timer.tick() => {
                    if self.scanning && let Err(error) = self.refresh_devices().await { self.state(error, false); }
                    if let Some(peripheral) = &self.active {
                        if !peripheral.is_connected().await.unwrap_or(false) {
                            self.generation = self.generation.wrapping_add(1);
                            self.release().await; self.state("Connection lost", false);
                        }
                    }
                }
            }
        }
    }
}
