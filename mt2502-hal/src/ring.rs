use core::{
    cell::RefCell,
    task::{Poll, Waker},
};
use critical_section::Mutex;

struct State<const N: usize> {
    bytes: [u8; N],
    head: usize,
    tail: usize,
    overruns: u32,
    waiter: Option<Waker>,
}
pub struct RxRing<const N: usize> {
    state: Mutex<RefCell<State<N>>>,
}
impl<const N: usize> RxRing<N> {
    pub const fn new() -> Self {
        Self {
            state: Mutex::new(RefCell::new(State {
                bytes: [0; N],
                head: 0,
                tail: 0,
                overruns: 0,
                waiter: None,
            })),
        }
    }
    pub fn clear(&self) {
        critical_section::with(|cs| {
            let mut state = self.state.borrow(cs).borrow_mut();
            state.head = 0;
            state.tail = 0;
            state.overruns = 0;
            state.waiter = None;
        });
    }
    pub fn push_irq(&self, byte: u8) -> bool {
        critical_section::with(|cs| {
            let mut state = self.state.borrow(cs).borrow_mut();
            if state.head.wrapping_sub(state.tail) >= N {
                state.overruns = state.overruns.wrapping_add(1);
                return false;
            }
            let index = state.head % N;
            state.bytes[index] = byte;
            state.head = state.head.wrapping_add(1);
            if let Some(waker) = state.waiter.take() {
                waker.wake();
            }
            true
        })
    }
    pub fn pop(&self) -> Option<u8> {
        critical_section::with(|cs| {
            let mut state = self.state.borrow(cs).borrow_mut();
            if state.head == state.tail {
                return None;
            }
            let byte = state.bytes[state.tail % N];
            state.tail = state.tail.wrapping_add(1);
            Some(byte)
        })
    }
    pub async fn read(&self, output: &mut [u8]) -> usize {
        if output.is_empty() {
            return 0;
        }
        core::future::poll_fn(|context| {
            critical_section::with(|cs| {
                let mut state = self.state.borrow(cs).borrow_mut();
                if state.head == state.tail {
                    state.waiter = Some(context.waker().clone());
                    return Poll::Pending;
                }
                let mut count = 0;
                while count < output.len() && state.head != state.tail {
                    output[count] = state.bytes[state.tail % N];
                    state.tail = state.tail.wrapping_add(1);
                    count += 1;
                }
                Poll::Ready(count)
            })
        })
        .await
    }
    pub fn overruns(&self) -> u32 {
        critical_section::with(|cs| self.state.borrow(cs).borrow().overruns)
    }
    pub fn note_error(&self) {
        critical_section::with(|cs| {
            let mut state = self.state.borrow(cs).borrow_mut();
            state.overruns = state.overruns.wrapping_add(1);
        });
    }
}
