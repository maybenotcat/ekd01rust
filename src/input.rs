#[derive(Clone, Copy, Default)]
pub struct ButtonEvents {
    pub pressed: u8,
    pub released: u8,
    pub short: u8,
    pub long: u8,
    pub repeat: u8,
    pub held: u8,
    pub queued_tick: u32,
}
impl ButtonEvents {
    /// Map physical +/- positions to logical actions; POWER and other bits stay intact.
    pub const fn orient_mask(mask: u8, flipped: bool) -> u8 {
        if flipped {
            (mask & !3) | ((mask & 1) << 1) | ((mask & 2) >> 1)
        } else {
            mask
        }
    }
    pub fn oriented(self, flipped: bool) -> Self {
        Self {
            pressed: Self::orient_mask(self.pressed, flipped),
            released: Self::orient_mask(self.released, flipped),
            short: Self::orient_mask(self.short, flipped),
            long: Self::orient_mask(self.long, flipped),
            repeat: Self::orient_mask(self.repeat, flipped),
            held: Self::orient_mask(self.held, flipped),
            queued_tick: self.queued_tick,
        }
    }
}
#[derive(Clone, Copy, Default)]
struct Key {
    changed: u32,
    down: u32,
    repeated: u32,
    candidate: bool,
    stable: bool,
    armed: bool,
    long_sent: bool,
}
pub struct ButtonFilter {
    keys: [Key; 3],
}
impl ButtonFilter {
    pub fn new(now: u32) -> Self {
        Self {
            keys: [Key {
                changed: now,
                down: now,
                repeated: now,
                ..Key::default()
            }; 3],
        }
    }
    pub fn mcu_event(&mut self, code: u8, now: u32) -> Option<ButtonEvents> {
        let index = match code & 0xf0 {
            0x20 => 0,
            0x10 => 1,
            _ => return None,
        };
        let phase = code & 15;
        if !(1..=3).contains(&phase) {
            return None;
        }
        let bit = 1 << index;
        let key = &mut self.keys[index];
        let mut event = ButtonEvents {
            queued_tick: now,
            ..ButtonEvents::default()
        };
        if phase != 3 {
            if !key.stable {
                key.down = now;
                key.repeated = now;
                key.long_sent = false;
                event.pressed = bit;
            }
            key.stable = true;
            key.candidate = true;
            key.armed = true;
            if phase == 2 && !key.long_sent {
                key.long_sent = true;
                event.long = bit;
            }
        } else {
            if key.stable && key.armed {
                event.released = bit;
                if !key.long_sent {
                    event.short = bit;
                }
            }
            key.stable = false;
            key.candidate = false;
            key.armed = true;
        }
        key.changed = now;
        for (index, key) in self.keys.iter().enumerate() {
            if key.stable && key.armed {
                event.held |= 1 << index;
            }
        }
        Some(event)
    }
    pub fn sample(&mut self, raw: u8, valid: u8, now: u32) -> ButtonEvents {
        let mut event = ButtonEvents {
            queued_tick: now,
            ..ButtonEvents::default()
        };
        for (index, key) in self.keys.iter_mut().enumerate() {
            let bit = 1 << index;
            if valid & bit == 0 {
                continue;
            }
            let value = raw & bit != 0;
            if value != key.candidate {
                key.candidate = value;
                key.changed = now;
            }
            if now.wrapping_sub(key.changed) >= 4 {
                if !value && !key.stable {
                    key.armed = true;
                }
                if value != key.stable {
                    key.stable = value;
                    if value {
                        key.down = now;
                        key.repeated = now;
                        key.long_sent = false;
                        if key.armed {
                            event.pressed |= bit;
                        }
                    } else {
                        if key.armed {
                            event.released |= bit;
                            if !key.long_sent {
                                event.short |= bit;
                            }
                        }
                        key.armed = true;
                    }
                }
            }
            if key.stable && key.armed {
                event.held |= bit;
                if !key.long_sent && now.wrapping_sub(key.down) >= 256 {
                    key.long_sent = true;
                    event.long |= bit;
                }
                if index != 2
                    && now.wrapping_sub(key.down) >= 64
                    && now.wrapping_sub(key.repeated) >= 16
                {
                    key.repeated = now;
                    event.repeat |= bit;
                }
            }
        }
        event
    }
}
