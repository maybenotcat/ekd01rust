use super::{
    controller::BluetoothController,
    state::{BluetoothControl, BluetoothState},
    telemetry::TelemetryService,
};
use crate::{soc::DebugUart, storage::FlashBus};
use embassy_futures::select::{Either, select};
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use trouble_host::{
    att::{AttClient, AttReq},
    prelude::*,
};

#[gatt_server(connections_max=1,mutex_type=NoopRawMutex,attribute_table_size=16)]
struct Server {
    telemetry: BikeTelemetry,
}
#[gatt_service(uuid = "7b8d0001-6f53-4ea1-9b66-734b1dd1e001")]
struct BikeTelemetry {
    #[characteristic(uuid="7b8d0002-6f53-4ea1-9b66-734b1dd1e001",read,notify,value=[0;60])]
    value: [u8; 60],
}

async fn wait_disabled() {
    while BluetoothControl::enabled() {
        embassy_time::Timer::after_ticks(4).await;
    }
}

pub async fn run(bus: FlashBus) -> ! {
    loop {
        while !BluetoothControl::enabled() {
            BluetoothControl::set_state(BluetoothState::Off);
            embassy_time::Timer::after_ticks(4).await;
        }
        BluetoothControl::set_state(BluetoothState::Starting);
        let mut controller = match BluetoothController::start(bus.clone()).await {
            Ok(controller) => controller,
            Err(error) => {
                use core::fmt::Write;
                let _ = writeln!(DebugUart, "[BT] INIT_FAILED: {error:?}");
                BluetoothControl::set_state(BluetoothState::Failed);
                wait_disabled().await;
                continue;
            }
        };
        let failed = {
            // Borrow the transport so dropping the stack returns power-management
            // control to the board driver without unsafe controller extraction.
            let external = ExternalController::<_, 4>::new(&controller.transport);
            let mut resources: HostResources<DefaultPacketPool, 1, 3> = HostResources::new();
            let stack = trouble_host::new(external, &mut resources).build();
            let mut runner = stack.runner();
            let mut peripheral = stack.peripheral();
            let server = Server::new_with_config(GapConfig::Peripheral(PeripheralConfig {
                name: "EKD01Free",
                appearance: &appearance::power_device::GENERIC_POWER_DEVICE,
            }))
            .expect("GATT attribute table");
            let _ = server.set(&server.telemetry.value, &TelemetryService::snapshot());
            let mut advert = [0; 31];
            let mut scan = [0; 31];
            let advert_len = AdStructure::encode_slice(
                &[
                    AdStructure::Flags(LE_GENERAL_DISCOVERABLE | BR_EDR_NOT_SUPPORTED),
                    AdStructure::CompleteLocalName(b"EKD01Free"),
                ],
                &mut advert,
            )
            .expect("advertising data");
            let scan_len = AdStructure::encode_slice(
                &[AdStructure::CompleteLocalName(b"EKD01Free")],
                &mut scan,
            )
            .expect("scan data");
            let application = async {
                loop {
                    let acceptor = peripheral
                        .advertise(
                            &Default::default(),
                            Advertisement::ConnectableScannableUndirected {
                                adv_data: &advert[..advert_len],
                                scan_data: &scan[..scan_len],
                            },
                        )
                        .await?;
                    BluetoothControl::set_state(BluetoothState::Ready);
                    DebugUart::log("[BT] BLE_ADVERTISING\n");
                    let connection = acceptor.accept().await?.with_attribute_server(&server)?;
                    BluetoothControl::set_state(BluetoothState::Connected);
                    DebugUart::log("[BLE] CONNECTED\n");
                    let requests = async {
                        loop {
                            match connection.next().await {
                                GattConnectionEvent::Disconnected { .. } => break,
                                GattConnectionEvent::Gatt { event } => {
                                    if let GattEvent::Read(read) = &event {
                                        if read.handle() == server.telemetry.value.handle
                                            && matches!(
                                                event.payload().incoming(),
                                                AttClient::Request(
                                                    AttReq::Read { .. }
                                                        | AttReq::ReadBlob { offset: 0, .. }
                                                )
                                            )
                                        {
                                            let _ = server.set(
                                                &server.telemetry.value,
                                                &TelemetryService::snapshot(),
                                            );
                                        }
                                    }
                                    if let Ok(reply) = event.accept() {
                                        reply.send().await;
                                    }
                                }
                                _ => {}
                            }
                        }
                    };
                    let notifications = async {
                        loop {
                            if server.telemetry.value.should_notify(&connection) {
                                let frame = TelemetryService::snapshot();
                                for page in frame.chunks_exact(20) {
                                    if server
                                        .telemetry
                                        .value
                                        .notify_raw(&connection, page, false)
                                        .await
                                        .is_err()
                                    {
                                        break;
                                    }
                                }
                            }
                            embassy_time::Timer::after_millis(250).await;
                        }
                    };
                    select(requests, notifications).await;
                }
                #[allow(unreachable_code)]
                Ok::<(), trouble_host::BleHostError<super::transport::TransportError>>(())
            };
            match select(wait_disabled(), select(runner.run(), application)).await {
                Either::First(()) => {
                    DebugUart::log("[BT] DISABLE_REQUESTED\n");
                    false
                }
                Either::Second(stopped) => {
                    use core::fmt::Write;
                    match stopped {
                        Either::First(result) => {
                            let _ = writeln!(DebugUart, "[BT] HOST_EXIT: {result:?}");
                        }
                        Either::Second(result) => {
                            let _ = writeln!(DebugUart, "[BT] GATT_EXIT: {result:?}");
                        }
                    }
                    BluetoothControl::set_state(BluetoothState::Failed);
                    true
                }
            }
        };
        BluetoothControl::set_state(BluetoothState::Stopping);
        controller.transport.close().await;
        BluetoothControl::set_state(if failed {
            BluetoothState::Failed
        } else {
            BluetoothState::Off
        });
        if BluetoothControl::enabled() {
            wait_disabled().await;
        }
    }
}
