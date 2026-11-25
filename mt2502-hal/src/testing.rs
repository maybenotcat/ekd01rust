//! Native-only scripted register interactions. This is not a CPU/peripheral emulator.
use crate::mmio::RegisterIo;
use std::{
    cell::RefCell,
    collections::{BTreeMap, VecDeque},
    vec::Vec,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Access {
    pub write: bool,
    pub width: u8,
    pub address: u32,
    pub value: u32,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub access: Access,
    pub count: usize,
}
#[derive(Default)]
struct State {
    active: bool,
    accesses: usize,
    values: BTreeMap<(u8, u32), u32>,
    scripted: BTreeMap<(u8, u32), VecDeque<u32>>,
    counters: BTreeMap<(u8, u32), u32>,
    fixed: BTreeMap<(u8, u32), u32>,
    trace: Vec<Run>,
    ticks: u64,
    dma_address: Option<u32>,
}
std::thread_local! { static STATE: RefCell<State> = RefCell::new(State::default()); }

pub fn reset() {
    STATE.with(|s| {
        *s.borrow_mut() = State {
            active: true,
            ..State::default()
        }
    });
}
pub fn value(width: u8, address: u32, value: u32) {
    STATE.with(|s| {
        s.borrow_mut().values.insert((width, address), value);
    });
}
pub fn fixed(width: u8, address: u32, value: u32) {
    STATE.with(|s| {
        s.borrow_mut().fixed.insert((width, address), value);
    });
}
pub fn reads(width: u8, address: u32, values: impl IntoIterator<Item = u32>) {
    STATE.with(|s| {
        s.borrow_mut()
            .scripted
            .insert((width, address), values.into_iter().collect());
    });
}
pub fn counter(width: u8, address: u32, start: u32, step: u32) {
    value(width, address, start);
    STATE.with(|s| {
        s.borrow_mut().counters.insert((width, address), step);
    });
}
pub fn ticks() -> u64 {
    STATE.with(|s| s.borrow().ticks)
}
pub fn advance(ticks: u64) {
    STATE.with(|s| s.borrow_mut().ticks += ticks);
}
pub fn trace() -> Vec<Run> {
    STATE.with(|s| s.borrow().trace.clone())
}
pub fn writes() -> Vec<Run> {
    trace().into_iter().filter(|r| r.access.write).collect()
}
pub fn scripted_reads_remaining() -> usize {
    STATE.with(|s| s.borrow().scripted.values().map(VecDeque::len).sum())
}
fn record(state: &mut State, access: Access) {
    state.accesses += 1;
    assert!(
        state.accesses <= 2_000_000,
        "native register script access budget exceeded"
    );
    if let Some(last) = state.trace.last_mut() {
        if last.access == access {
            last.count += 1;
            return;
        }
    }
    state.trace.push(Run { access, count: 1 });
}
fn read(width: u8, address: u32) -> u32 {
    STATE.with(|s| {
        let mut state = s.borrow_mut();
        assert!(
            state.active,
            "register access without an active native script"
        );
        let key = (width, address);
        let scripted = state.scripted.get_mut(&key).and_then(VecDeque::pop_front);
        let result = scripted
            .or_else(|| state.fixed.get(&key).copied())
            .unwrap_or_else(|| *state.values.get(&key).unwrap_or(&0));
        if let Some(step) = state.counters.get(&key).copied() {
            state.values.insert(key, result.wrapping_add(step));
        }
        record(
            &mut state,
            Access {
                write: false,
                width,
                address,
                value: result,
            },
        );
        result
    })
}
fn write(width: u8, address: u32, value: u32) {
    STATE.with(|s| {
        let mut state = s.borrow_mut();
        assert!(
            state.active,
            "register access without an active native script"
        );
        state.values.insert((width, address), value);
        record(
            &mut state,
            Access {
                write: true,
                width,
                address,
                value,
            },
        );
    });
}
pub struct ScriptIo;
impl RegisterIo for ScriptIo {
    fn read8(address: u32) -> u8 {
        read(8, address) as u8
    }
    fn read16(address: u32) -> u16 {
        read(16, address) as u16
    }
    fn read32(address: u32) -> u32 {
        read(32, address)
    }
    fn write8(address: u32, value: u8) {
        write(8, address, value as u32);
    }
    fn write16(address: u32, value: u16) {
        write(16, address, value as u32);
    }
    fn write32(address: u32, value: u32) {
        write(32, address, value);
    }
}

pub fn set_dma_address(address: u32) {
    STATE.with(|s| s.borrow_mut().dma_address = Some(address));
}
pub fn dma_address(pointer: *const u16) -> u32 {
    STATE.with(|s| s.borrow().dma_address.unwrap_or(pointer as u32))
}
