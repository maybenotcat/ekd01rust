Upstream crate: embassy-hal-internal 0.4.0, Embassy commit `f242d794e3325bc9b3559e50d2ea7fc293e1d574`. MIT OR Apache-2.0; license texts included.

Local change: atomic_ring_buffer uses portable-atomic with its critical-section fallback instead of core atomics, because ARMv5TE has no native pointer atomics. The existing board critical-section implementation supplies interrupt masking. No hardware register access is added by this patch.
