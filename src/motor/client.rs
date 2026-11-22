use super::{
    settings::{BlockId, ParameterBlock},
    transfer::{AdvancedView, BlockTransfer, RequestResult, TransferStatus},
};
use crate::{
    protocol::{
        BridgeMessage, BridgeParser, CanFrame, MOTOR_BASIC_SETTINGS, McuMessage, VERSION_QUERY,
    },
    soc::{DebugUart, bridge_uart::BridgeUart},
    telemetry::Telemetry,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BasicStatus {
    #[default]
    Idle,
    Queued,
    Sent,
    Verified,
    Timeout,
    BadValues,
    Offline,
    Busy,
    Neutral,
}
#[derive(Clone, Copy, Default)]
pub struct BikeStatus {
    pub rx: u32,
    pub tx: u32,
    pub malformed: u32,
    pub uart_errors: u32,
    pub last_id: u32,
    pub last_rx: u32,
    pub motor_rx: u32,
    pub last_motor_rx: u32,
    pub mcu_rx: u32,
    pub mcu_voltage_mv: u32,
    pub mcu_voltage_tick: u32,
    pub mcu_voltage_valid: bool,
    pub mcu_version: [u8; 5],
    pub mcu_version_valid: bool,
    pub basic_state: BasicStatus,
    pub requested_basic: [u16; 3],
    pub basic_sent: u32,
    pub levels: u8,
    pub assist: u8,
    pub lights: bool,
    pub active: bool,
    pub stop_requested: bool,
    pub stopped: bool,
    pub last_frame: CanFrame,
    pub telemetry: Telemetry,
    pub advanced: AdvancedView,
    pub power_mode: u8,
    pub basic_result: RequestResult,
}
impl BikeStatus {
    pub fn can_online(&self, now: u32) -> bool {
        self.rx != 0 && now.wrapping_sub(self.last_rx) < 384
    }
    pub fn motor_online(&self, now: u32) -> bool {
        self.motor_rx != 0 && now.wrapping_sub(self.last_motor_rx) < 384
    }
    pub fn voltage_fresh(&self, now: u32) -> bool {
        self.mcu_voltage_valid && now.wrapping_sub(self.mcu_voltage_tick) < 384
    }
    fn request_check(&self, now: u32, busy: bool, neutral: bool) -> RequestResult {
        if busy {
            RequestResult::Busy
        } else if !self.active || self.stop_requested || self.stopped || !self.motor_online(now) {
            RequestResult::Offline
        } else if neutral && self.assist != 0 {
            RequestResult::NeedsNeutral
        } else {
            RequestResult::Accepted
        }
    }
}
enum Request {
    Read(BlockId),
    Write(BlockId, ParameterBlock, u32),
}
impl Request {
    fn id(&self) -> BlockId {
        match self {
            Self::Read(id) | Self::Write(id, _, _) => *id,
        }
    }
    fn writes(&self) -> bool {
        matches!(self, Self::Write(..))
    }
}

pub struct MotorClient {
    uart: BridgeUart,
    parser: BridgeParser,
    status: BikeStatus,
    advanced: BlockTransfer,
    request: Option<Request>,
    wire_state: bool,
    version_queried: bool,
    next_tx: u32,
    last_byte: u32,
    previous_errors: u32,
    fraction: u8,
    unknown: [u32; 8],
    unknown_count: usize,
    stop_sent: Option<u32>,
}
impl MotorClient {
    pub fn start(now: u32) -> Self {
        Self {
            uart: BridgeUart::initialize(),
            parser: BridgeParser::new(),
            status: BikeStatus {
                levels: 5,
                active: true,
                ..BikeStatus::default()
            },
            advanced: BlockTransfer::default(),
            request: None,
            wire_state: false,
            version_queried: false,
            next_tx: now,
            last_byte: now,
            previous_errors: 0,
            fraction: 0,
            unknown: [0; 8],
            unknown_count: 0,
            stop_sent: None,
        }
    }
    pub fn status(&self) -> &BikeStatus {
        &self.status
    }
    pub fn controls(&mut self, levels: u8, assist: u8, lights: bool, power_mode: u8) {
        if !matches!(levels, 3 | 5 | 9)
            || assist > levels
            || power_mode > 3
            || self.status.stop_requested
        {
            return;
        }
        self.status.levels = levels;
        self.status.assist = assist;
        self.status.lights = lights;
        self.status.power_mode = power_mode;
        DebugUart::hex("[BIKE] ASSIST_REQUEST", assist as u32);
    }
    pub fn stop(&mut self) {
        self.request = None;
        self.advanced.cancel();
        self.status.advanced = self.advanced.view;
        self.status.assist = 0;
        self.status.lights = false;
        self.status.stop_requested = true;
    }
    pub fn force_stop(&mut self) {
        self.status.active = false;
        self.status.stopped = true;
        self.uart.stop();
        DebugUart::log("[CAN] STOP_TIMEOUT\n");
    }
    pub fn clear_trip(&mut self) {
        self.status.telemetry.reset_trip();
    }
    fn busy(&self) -> bool {
        matches!(
            self.status.basic_state,
            BasicStatus::Queued | BasicStatus::Sent
        ) || self.request.is_some()
            || self.advanced.busy()
    }
    pub fn apply_basic(&mut self, values: [u16; 3], now: u32) -> RequestResult {
        let mut result = self.status.request_check(now, self.busy(), true);
        if result == RequestResult::Accepted
            && CanFrame::basic_settings(values[0], values[1], values[2]).is_none()
        {
            result = RequestResult::Invalid;
        }
        self.status.basic_result = result;
        if result == RequestResult::Accepted {
            self.status.requested_basic = values;
            self.status.basic_state = BasicStatus::Queued;
            self.status.basic_sent = now;
        } else if result != RequestResult::Busy {
            self.status.basic_state = match result {
                RequestResult::NeedsNeutral => BasicStatus::Neutral,
                RequestResult::Invalid => BasicStatus::BadValues,
                _ => BasicStatus::Offline,
            };
        }
        DebugUart::hex("[MOTOR] REQUEST_RESULT", result as u32);
        result
    }
    fn rejected_state(result: RequestResult) -> TransferStatus {
        match result {
            RequestResult::Accepted => TransferStatus::Idle,
            RequestResult::Busy => TransferStatus::Busy,
            RequestResult::Offline => TransferStatus::Offline,
            RequestResult::NeedsNeutral => TransferStatus::Neutral,
            RequestResult::Invalid => TransferStatus::BadBlock,
        }
    }
    fn request(&mut self, request: Request, now: u32) -> RequestResult {
        let result = self
            .status
            .request_check(now, self.busy(), request.writes());
        self.advanced.view.requested_block = Some(request.id());
        self.advanced.view.request_result = result;
        if result == RequestResult::Accepted {
            self.request = Some(request);
        } else if result != RequestResult::Busy {
            self.advanced.view.state = Self::rejected_state(result);
        }
        self.status.advanced = self.advanced.view;
        result
    }
    pub fn read_block(&mut self, id: BlockId, now: u32) -> RequestResult {
        self.request(Request::Read(id), now)
    }
    pub fn apply_block(
        &mut self,
        id: BlockId,
        block: ParameterBlock,
        revision: u32,
        now: u32,
    ) -> RequestResult {
        self.request(Request::Write(id, block, revision), now)
    }
    fn service_request(&mut self, now: u32) {
        if !self.status.stop_requested {
            if let Some(request) = self.request.take() {
                let mut result = self.status.request_check(now, false, request.writes());
                if result == RequestResult::Accepted {
                    match request {
                        Request::Read(id) => self.advanced.read(id, now),
                        Request::Write(id, block, revision) => {
                            if !self.advanced.apply(id, &block, revision, now) {
                                result = RequestResult::Invalid;
                            }
                        }
                    }
                }
                if self.advanced.view.request_result == RequestResult::Accepted {
                    self.advanced.view.request_result = result;
                }
                if result != RequestResult::Accepted {
                    self.advanced.view.state = Self::rejected_state(result);
                }
            }
        }
        self.advanced.poll(now);
        self.status.advanced = self.advanced.view;
    }
    fn receive_can(
        status: &mut BikeStatus,
        advanced: &mut BlockTransfer,
        frame: &CanFrame,
        now: u32,
    ) {
        status.rx = status.rx.wrapping_add(1);
        status.last_id = frame.id;
        status.last_rx = now;
        let recognized = status.telemetry.decode(frame, now) | advanced.receive(frame, now);
        if recognized && frame.id >> 24 == 2 {
            status.motor_rx = status.motor_rx.wrapping_add(1);
            status.last_motor_rx = now;
        }
        status.advanced = advanced.view;
        let values = [
            status.telemetry.limit,
            status.telemetry.wheel,
            status.telemetry.circumference,
        ];
        if status.basic_state == BasicStatus::Sent
            && frame.id == MOTOR_BASIC_SETTINGS
            && frame.length == 6
            && values == status.requested_basic
        {
            status.basic_state = BasicStatus::Verified;
            DebugUart::log("[MOTOR] READBACK_MATCH\n");
        }
        status.last_frame = *frame;
        if status.rx == 1 {
            DebugUart::hex("[CAN] FIRST_RX_ID", frame.id);
        }
    }
    fn receive(&mut self, now: u32, mut key: impl FnMut(u8)) {
        if now.wrapping_sub(self.last_byte) > 8 {
            self.parser.timeout_partial();
        }
        let errors = self.uart.errors();
        if errors != self.previous_errors {
            self.parser.reset();
            self.previous_errors = errors;
        }
        for _ in 0..1024 {
            let Some(byte) = self.uart.receive() else {
                break;
            };
            let Self {
                parser,
                status,
                advanced,
                unknown,
                unknown_count,
                ..
            } = self;
            parser.feed(byte, |message| match message {
                BridgeMessage::Can(frame) => Self::receive_can(status, advanced, &frame, now),
                BridgeMessage::Service(payload) => {
                    if let Some(message) = McuMessage::decode(payload) {
                        status.mcu_rx = status.mcu_rx.wrapping_add(1);
                        match message {
                            McuMessage::Voltage(voltage) => {
                                if !status.mcu_voltage_valid {
                                    DebugUart::hex("[MCU] ADC_MILLIVOLTS", voltage);
                                }
                                status.mcu_voltage_valid = true;
                                status.mcu_voltage_mv = voltage;
                                status.mcu_voltage_tick = now;
                            }
                            McuMessage::Key(code) => key(code),
                            McuMessage::Version(version) => {
                                status.mcu_version = version;
                                status.mcu_version_valid = true;
                                DebugUart::log("[MCU] APP_VERSION_REPLY\n");
                            }
                        }
                    } else {
                        let signature = ((payload.len() as u32) << 16)
                            | ((payload.first().copied().unwrap_or(0) as u32) << 8)
                            | payload.get(1).copied().unwrap_or(0) as u32;
                        if *unknown_count < 8 && !unknown[..*unknown_count].contains(&signature) {
                            unknown[*unknown_count] = signature;
                            *unknown_count += 1;
                            DebugUart::hex("[MCU] UNKNOWN_LENGTH_SERVICE_VALUE", signature);
                        }
                    }
                }
            });
            self.last_byte = now;
        }
        self.status.uart_errors = errors;
        self.status.malformed = self.parser.malformed;
    }
    fn send(&mut self, frame: CanFrame) -> bool {
        let mut encoded = [0; 15];
        let Some(length) = frame.encode(&mut encoded) else {
            return false;
        };
        if !self.uart.send(&encoded[..length]) {
            return false;
        }
        self.status.tx = self.status.tx.wrapping_add(1);
        true
    }
    pub fn service(&mut self, now: u32, key: impl FnMut(u8)) {
        if !self.status.active || self.status.stopped {
            return;
        }
        if self
            .stop_sent
            .is_some_and(|tick| now.wrapping_sub(tick) >= 1)
        {
            self.uart.stop();
            self.status.stopped = true;
            DebugUart::log("[CAN] STOPPED; assist0 sent\n");
            return;
        }
        self.service_request(now);
        self.receive(now, key);
        if matches!(
            self.status.basic_state,
            BasicStatus::Queued | BasicStatus::Sent
        ) && now.wrapping_sub(self.status.basic_sent) >= 256
        {
            self.status.basic_state = BasicStatus::Timeout;
        }
        if self.status.basic_state == BasicStatus::Queued && !self.status.stop_requested {
            let ready = self.status.request_check(now, false, true);
            if ready != RequestResult::Accepted {
                self.status.basic_result = ready;
                self.status.basic_state = if ready == RequestResult::NeedsNeutral {
                    BasicStatus::Neutral
                } else {
                    BasicStatus::Offline
                };
            } else {
                let [limit, wheel, circumference] = self.status.requested_basic;
                if let Some(frame) = CanFrame::basic_settings(limit, wheel, circumference) {
                    if self.send(frame) {
                        self.status.basic_state = BasicStatus::Sent;
                        self.status.basic_sent = now;
                    }
                }
            }
        }
        if !self.version_queried && !self.status.stop_requested && self.uart.send(&VERSION_QUERY) {
            self.version_queried = true;
        }
        if !self.status.stop_requested {
            if let Some(frame) = self.advanced.pending() {
                if self.send(frame) {
                    self.advanced.sent(now);
                }
            }
        }
        self.status.advanced = self.advanced.view;
        if (now.wrapping_sub(self.next_tx) as i32 >= 0 || self.status.stop_requested)
            && self.stop_sent.is_none()
        {
            if let Some(frame) = CanFrame::controls(
                self.status.levels,
                self.status.assist,
                self.status.lights,
                self.wire_state,
                self.status.power_mode,
            ) {
                if self.send(frame) {
                    self.wire_state = true;
                    self.fraction += 8;
                    let ticks = if self.fraction >= 10 {
                        self.fraction -= 10;
                        13
                    } else {
                        12
                    };
                    self.next_tx = self.next_tx.wrapping_add(ticks);
                    if now.wrapping_sub(self.next_tx) as i32 >= 0 {
                        self.next_tx = now.wrapping_add(ticks);
                    }
                    if self.status.stop_requested {
                        self.stop_sent = Some(now);
                    }
                }
            }
        }
    }
}
