//! Deterministic pseudo-random number generator.
//!
//! Hand-rolled SplitMix64 rather than an external crate so the stream can
//! never silently change under a dependency upgrade. Determinism of the
//! strategic history is a hard invariant; the RNG state is part of the
//! serialized world state.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimRng {
    state: u64,
}

impl SimRng {
    pub fn new(seed: u64) -> Self {
        SimRng { state: seed }
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform roll in `0..100`.
    pub fn roll_percent(&mut self) -> u8 {
        (self.next_u64() % 100) as u8
    }

    /// Uniform roll in `0..n`. Returns 0 for `n == 0`.
    pub fn roll_range(&mut self, n: u64) -> u64 {
        if n == 0 {
            0
        } else {
            self.next_u64() % n
        }
    }
}
