# ARMv5 configuration

The local synchronization implementation uses portable atomics with the board's
critical-section backend. Its `heapless` dependency also explicitly enables
`portable-atomic-critical-section`: ARMv5TE has no native pointer atomics.
This keeps the loader independent of an otherwise unused direct dependency
whose only purpose was feature unification. The application already enabled
the same heapless backend, so its atomic configuration is unchanged.
