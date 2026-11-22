#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PowerState {
    #[default]
    Running,
    WaitRelease,
    WaitLinks,
    Releasing,
    SaveFailed,
    ReleaseFailed,
    Off,
}
#[derive(Default)]
pub struct PowerFlow {
    pub state: PowerState,
    started: u32,
}
impl PowerFlow {
    pub fn request(&mut self, now: u32, wait_release: bool) {
        self.state = if wait_release {
            PowerState::WaitRelease
        } else {
            PowerState::WaitLinks
        };
        self.started = now;
    }
    pub fn released(&mut self, now: u32) {
        if self.state == PowerState::WaitRelease {
            self.request(now, false);
        }
    }
    pub fn retry(&mut self, now: u32, power: bool) -> bool {
        if power
            && matches!(
                self.state,
                PowerState::SaveFailed | PowerState::ReleaseFailed
            )
        {
            self.request(now, false);
            true
        } else {
            false
        }
    }
    pub fn links_ready(&self, now: u32, motor: bool, bluetooth: bool) -> bool {
        let expired = now.wrapping_sub(self.started) >= 128;
        self.state == PowerState::WaitLinks && (motor || expired) && (bluetooth || expired)
    }
    pub fn save_finished(&mut self, success: bool) {
        self.state = if success {
            PowerState::Releasing
        } else {
            PowerState::SaveFailed
        };
    }
}
