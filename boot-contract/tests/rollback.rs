//! Native NOR fault injection; no CPU or peripheral emulation.
use embassy_boot::{AlignedBuffer, BlockingFirmwareState, BootLoader, BootLoaderConfig, State};
use embedded_storage::nor_flash::{
    ErrorType, NorFlash, NorFlashError, NorFlashErrorKind, ReadNorFlash,
};
use std::{cell::RefCell, rc::Rc};

#[derive(Debug, Clone, Copy)]
struct PowerCut;
impl NorFlashError for PowerCut {
    fn kind(&self) -> NorFlashErrorKind {
        NorFlashErrorKind::Other
    }
}

#[derive(Clone)]
struct Memory {
    bytes: Vec<u8>,
    cut_after: Option<usize>,
    mutations: usize,
}
impl Memory {
    fn mutate(&mut self) -> Result<(), PowerCut> {
        if self.cut_after == Some(self.mutations) {
            return Err(PowerCut);
        }
        self.mutations += 1;
        Ok(())
    }
}
#[derive(Clone)]
struct Region<const PAGE: usize> {
    memory: Rc<RefCell<Memory>>,
    start: usize,
    length: usize,
}
impl<const PAGE: usize> ErrorType for Region<PAGE> {
    type Error = PowerCut;
}
impl<const PAGE: usize> ReadNorFlash for Region<PAGE> {
    const READ_SIZE: usize = 1;
    fn capacity(&self) -> usize {
        self.length
    }
    fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), PowerCut> {
        assert!(offset as usize + bytes.len() <= self.length);
        bytes.copy_from_slice(
            &self.memory.borrow().bytes[self.start + offset as usize..][..bytes.len()],
        );
        Ok(())
    }
}
impl<const PAGE: usize> NorFlash for Region<PAGE> {
    const WRITE_SIZE: usize = 4;
    const ERASE_SIZE: usize = PAGE;
    fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), PowerCut> {
        assert_eq!(offset % 4, 0);
        assert_eq!(bytes.len() % 4, 0);
        assert!(offset as usize + bytes.len() <= self.length);
        let mut memory = self.memory.borrow_mut();
        for (index, word) in bytes.chunks_exact(4).enumerate() {
            memory.mutate()?;
            let address = self.start + offset as usize + index * 4;
            for (old, new) in memory.bytes[address..address + 4].iter_mut().zip(word) {
                assert_eq!(*old & new, *new, "NOR programming requires erase");
                *old &= new;
            }
        }
        Ok(())
    }
    fn erase(&mut self, from: u32, to: u32) -> Result<(), PowerCut> {
        assert!(from <= to && to as usize <= self.length);
        assert_eq!(from as usize % PAGE, 0);
        assert_eq!(to as usize % PAGE, 0);
        let mut memory = self.memory.borrow_mut();
        // A cut can leave a partially erased page, not merely an atomic erase.
        for address in (self.start + from as usize..self.start + to as usize).step_by(16) {
            memory.mutate()?;
            memory.bytes[address..address + 16].fill(0xff);
        }
        Ok(())
    }
}

fn regions<const PAGE: usize>(
    active_size: usize,
    state_size: usize,
) -> (
    Rc<RefCell<Memory>>,
    Region<PAGE>,
    Region<PAGE>,
    Region<PAGE>,
) {
    let dfu_size = active_size + PAGE;
    let memory = Rc::new(RefCell::new(Memory {
        bytes: vec![0xff; active_size + dfu_size + state_size + 64],
        cut_after: None,
        mutations: 0,
    }));
    memory.borrow_mut().bytes[..active_size].fill(0x11);
    memory.borrow_mut().bytes[active_size..active_size * 2].fill(0x22);
    let make = |start, length| Region {
        memory: memory.clone(),
        start,
        length,
    };
    let active = make(0, active_size);
    let dfu = make(active_size, dfu_size);
    let state = make(active_size + dfu_size, state_size);
    (memory, active, dfu, state)
}
fn prepare<const PAGE: usize>(
    active: &Region<PAGE>,
    dfu: &Region<PAGE>,
    state: &Region<PAGE>,
) -> Result<State, embassy_boot::BootError> {
    let mut loader = BootLoader::new(BootLoaderConfig {
        active: active.clone(),
        dfu: dfu.clone(),
        state: state.clone(),
    });
    let mut buffer = AlignedBuffer([0; 32]);
    loader.prepare_boot(&mut buffer.0)?;
    loader.read_state(&mut buffer.0)
}
fn mark<const PAGE: usize>(state: &Region<PAGE>, updated: bool) {
    let mut buffer = AlignedBuffer([0; 4]);
    let mut state = BlockingFirmwareState::new(state.clone(), &mut buffer.0);
    if updated {
        state.mark_updated().unwrap();
    } else {
        state.mark_booted().unwrap();
    }
}

#[test]
fn full_size_layout_swaps_confirms_and_preserves_reserved_data() {
    use ekd01_boot_contract as layout;
    let (memory, active, dfu, state) =
        regions::<4096>(layout::ACTIVE.len() as usize, layout::STATE.len() as usize);
    mark(&state, true);
    assert_eq!(prepare(&active, &dfu, &state).unwrap(), State::Swap);
    assert!(
        memory.borrow().bytes[..active.length]
            .iter()
            .all(|byte| *byte == 0x22)
    );
    mark(&state, false);
    assert_eq!(prepare(&active, &dfu, &state).unwrap(), State::Boot);
    assert!(
        memory.borrow().bytes[..active.length]
            .iter()
            .all(|byte| *byte == 0x22)
    );
    assert!(
        memory.borrow().bytes[state.start + state.length..]
            .iter()
            .all(|byte| *byte == 0xff)
    );
}

#[test]
fn interrupted_swap_or_revert_resumes_without_losing_the_original_image() {
    let (memory, active, dfu, state) = regions::<64>(192, 64);
    mark(&state, true);
    let initial = memory.borrow().clone();
    memory.borrow_mut().mutations = 0;
    prepare(&active, &dfu, &state).unwrap();
    let swap_mutations = memory.borrow().mutations;
    let swapped = memory.borrow().clone();
    memory.borrow_mut().mutations = 0;
    prepare(&active, &dfu, &state).unwrap();
    let revert_mutations = memory.borrow().mutations;
    for (snapshot, count, swapping) in [
        (initial, swap_mutations, true),
        (swapped, revert_mutations, false),
    ] {
        for cut in 0..count {
            *memory.borrow_mut() = snapshot.clone();
            memory.borrow_mut().mutations = 0;
            memory.borrow_mut().cut_after = Some(cut);
            assert!(prepare(&active, &dfu, &state).is_err());
            memory.borrow_mut().cut_after = None;
            let result = prepare(&active, &dfu, &state).unwrap();
            if swapping && result == State::Swap {
                // If the interrupted boot completed the swap, one more
                // unconfirmed boot must restore the old image.
                prepare(&active, &dfu, &state).unwrap();
            }
            assert!(
                memory.borrow().bytes[..active.length]
                    .iter()
                    .all(|byte| *byte == 0x11),
                "cut {cut}"
            );
        }
    }
}
