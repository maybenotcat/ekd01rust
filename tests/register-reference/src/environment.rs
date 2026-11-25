//! Statically dispatched integration hooks. No global callbacks or executor dependency.
pub trait Trace {
    fn log(text: &str);
    fn hex(name: &str, value: u32);
}
#[allow(async_fn_in_trait)]
pub trait Runtime: Trace {
    /// Wrapping ticks at the board's established 128 Hz rate.
    fn now() -> u32;
    async fn delay_ticks(ticks: u64);
    async fn delay_ms(ms: u64) {
        Self::delay_ticks((ms * 128 + 999) / 1000 + 1).await;
    }
}
pub trait BootEnvironment: Trace {
    fn stop(message: &str) -> !;
}
