# Frozen register-sequence reference

Source revision: 3d10466e56eada114e0d7160a16f0321e09962db.
This is a native-test fixture, never a firmware dependency or a second shipping HAL.
The source was copied before the board/HAL refactor.

Instrumentation changes:
- Primitive MMIO reads/writes use the shared scripted bus. RMW bodies remain frozen.
- Boot and display modules compile on the native host.
- Vector STR is represented by its exact address/value/word-width store.
- Native diagnostic CPSR reads return zero; no privileged instruction is executed.
- Display tests substitute the numeric DMA address. The framebuffer remains a real
  host slice, so its volatile payload read is safe and unchanged.

Backlight and pin fixtures are from the same source revision.
snapshot.sha256 protects normalized-LF Rust source and the fixture manifest.
Production algorithms must not be copied back into this fixture during refactoring.
The golden trace hashes were captured from this fixture, not the new implementation.
Both trace equality and scenario success/failure intent are checked.

This is a register interaction script, not a CPU or complete peripheral emulator.
It cannot prove electrical behavior, oscillator accuracy, DMA bus coherency or timing
on silicon. Final ELF checks and hardware validation remain separate.