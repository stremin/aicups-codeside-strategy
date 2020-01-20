// based on java.lang.Random

use core::mem;

pub struct Random {
    seed: u64,
}

const MULTIPLIER: u64 = 0x5DEECE66D;
const ADDEND: u64 = 0xB;
const MASK: u64 = (1 << 48) - 1;

#[allow(dead_code)]
impl Random {
    pub fn new(seed: u64) -> Self {
        Self { seed: (seed ^ MULTIPLIER) & MASK }
    }

    pub fn next(&mut self, bits: u32) -> u32 {
        self.seed = (self.seed.wrapping_mul(MULTIPLIER).wrapping_add(ADDEND)) & MASK;
        (self.seed >> (48 - bits) as u64) as u32
    }

    pub fn next_u32(&mut self) -> u32 {
        self.next(32)
    }

    pub fn next_u64(&mut self) -> u64 {
        ((self.next_u32() as u64) << 32) | (self.next_u32() as u64)
    }

    /// Return the next random f64 selected from the half-open
    /// interval `[0, 1)`.
    pub fn next_f64(&mut self) -> f64 {
        const UPPER_MASK: u64 = 0x3FF0000000000000;
        const LOWER_MASK: u64 = 0xFFFFFFFFFFFFF;
        let tmp = UPPER_MASK | (self.next_u64() & LOWER_MASK);
        let result: f64 = unsafe { mem::transmute(tmp) };
        result - 1.0
    }

    /// [0, bound)
    pub fn next_u32_bounded(&mut self, bound: u32) -> u32 {
        // this is the largest number that fits into $unsigned
        // that `range` divides evenly, so, if we've sampled
        // `n` uniformly from this region, then `n % range` is
        // uniform in [0, range)
        let zone = std::u32::MAX - std::u32::MAX % bound;

        loop {
            let value = self.next(32);
            if value < zone {
                return value % bound;
            }
        }
    }
}