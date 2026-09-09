// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
//! Exact nonnegative integer accumulation with one final binary64 rounding.

/// Allocation-free base-2 through base-36 integer to binary64 converter.
///
/// Keeps at most 1024 integer bits. Larger integers necessarily round to positive
/// infinity, so no unbounded integer storage is required. Each digit costs at
/// most 32 limb multiply/add operations; callers must bound their input scan.
/// Negative signs and lexical digit recognition belong to the caller.
#[derive(Clone, Debug)]
pub struct RadixInteger {
    limbs: [u32; 32],
    used: usize,
    radix: u32,
    overflow: bool,
}

impl RadixInteger {
    /// Creates a zero accumulator, or returns `None` for a radix outside 2..=36.
    #[must_use]
    pub fn new(radix: u32) -> Option<Self> {
        (2..=36).contains(&radix).then_some(Self {
            limbs: [0; 32],
            used: 1,
            radix,
            overflow: false,
        })
    }

    /// Appends a numeric digit. Returns false without modifying the accumulator
    /// if the digit is not less than the radix, including after overflow.
    pub fn push(&mut self, digit: u32) -> bool {
        if digit >= self.radix {
            return false;
        }
        if self.overflow {
            return true;
        }
        let mut carry = u64::from(digit);
        for limb in self.limbs.iter_mut().take(self.used) {
            let next = u64::from(*limb)
                .saturating_mul(u64::from(self.radix))
                .saturating_add(carry);
            *limb = u32::try_from(next & 0xffff_ffff).unwrap_or(0);
            carry = next >> 32;
        }
        if carry != 0 {
            if let Some(limb) = self.limbs.get_mut(self.used) {
                *limb = u32::try_from(carry).unwrap_or(0);
                self.used = self.used.saturating_add(1);
            } else {
                self.overflow = true;
            }
        }
        true
    }

    /// Rounds the exact accumulated integer to nearest, ties to even. Zero is
    /// positive; overflow produces positive infinity. Does not change state.
    #[must_use]
    pub fn to_f64(&self) -> f64 {
        if self.overflow {
            return f64::INFINITY;
        }
        if self.used <= 2 {
            let low = u64::from(self.limbs.first().copied().unwrap_or(0));
            let high = u64::from(self.limbs.get(1).copied().unwrap_or(0));
            return round_u64((high << 32) | low);
        }
        let high = self
            .limbs
            .get(self.used.saturating_sub(1))
            .copied()
            .unwrap_or(0);
        let bits = self
            .used
            .saturating_sub(1)
            .saturating_mul(32)
            .saturating_add(
                usize::try_from(32u32.saturating_sub(high.leading_zeros())).unwrap_or(0),
            );
        let shift = bits.saturating_sub(53);
        let mut significand = 0u64;
        for index in (shift..bits).rev() {
            significand = (significand << 1) | u64::from(self.bit(index));
        }
        let guard = shift.saturating_sub(1);
        let sticky = self.limbs.iter().enumerate().any(|(i, limb)| {
            let start = i.saturating_mul(32);
            if start >= guard {
                return false;
            }
            let count = u32::try_from(guard.saturating_sub(start).min(32)).unwrap_or(0);
            (u64::from(*limb) & ((1u64 << count).saturating_sub(1))) != 0
        });
        let round_up = self.bit(guard) && (sticky || significand & 1 != 0);
        // Adding one at the encoded significand's least significant bit also
        // handles carry into the exponent, including the finite/infinity edge.
        let exponent = u64::try_from(bits.saturating_add(1022)).unwrap_or(0);
        let encoded = (exponent << 52) | (significand & 0x000f_ffff_ffff_ffff);
        f64::from_bits(encoded.saturating_add(u64::from(round_up)))
    }

    fn bit(&self, index: usize) -> bool {
        self.limbs
            .get(index / 32)
            .is_some_and(|limb| (limb >> (index % 32)) & 1 != 0)
    }
}

#[expect(
    clippy::cast_precision_loss,
    clippy::as_conversions,
    reason = "Rust integer-to-binary64 conversion rounds once to nearest, ties to even"
)]
const fn round_u64(integer: u64) -> f64 {
    integer as f64
}
