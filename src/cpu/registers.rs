//! CPU registers and status flags.
//!
//! This module owns the "how does CPU state get read and written" knowledge:
//! the P register layout and the accessors every instruction handler uses.
//! Instruction semantics live in `ops/`, cycle placement in `schedule`, and
//! interrupt sampling in `interrupt`.

use super::core::CPU;

// ── Status flag constants ──
pub const FLAG_C: u8 = 0x01;
pub const FLAG_Z: u8 = 0x02;
pub const FLAG_I: u8 = 0x04;
pub const FLAG_D: u8 = 0x08;
pub const FLAG_B: u8 = 0x10;
pub const FLAG_U: u8 = 0x20;
pub const FLAG_V: u8 = 0x40;
pub const FLAG_N: u8 = 0x80;

impl CPU {
    // ── Status flag accessors (p is a raw u8) ──

    #[inline]
    pub fn carry(&self) -> bool {
        (self.p & FLAG_C) != 0
    }
    #[inline]
    pub fn set_carry(&mut self, v: bool) {
        if v {
            self.p |= FLAG_C;
        } else {
            self.p &= !FLAG_C;
        }
    }

    #[inline]
    pub fn zero(&self) -> bool {
        (self.p & FLAG_Z) != 0
    }
    #[inline]
    pub fn set_zero(&mut self, v: bool) {
        if v {
            self.p |= FLAG_Z;
        } else {
            self.p &= !FLAG_Z;
        }
    }

    #[inline]
    pub fn irq_disabled(&self) -> bool {
        (self.p & FLAG_I) != 0
    }
    #[inline]
    pub fn set_irq_disable(&mut self, v: bool) {
        if v {
            self.p |= FLAG_I;
        } else {
            self.p &= !FLAG_I;
        }
    }

    #[inline]
    pub fn set_decimal(&mut self, v: bool) {
        if v {
            self.p |= FLAG_D;
        } else {
            self.p &= !FLAG_D;
        }
    }

    #[inline]
    pub fn overflow(&self) -> bool {
        (self.p & FLAG_V) != 0
    }
    #[inline]
    pub fn set_overflow(&mut self, v: bool) {
        if v {
            self.p |= FLAG_V;
        } else {
            self.p &= !FLAG_V;
        }
    }

    #[inline]
    pub fn negative(&self) -> bool {
        (self.p & FLAG_N) != 0
    }
    #[inline]
    pub fn set_negative(&mut self, v: bool) {
        if v {
            self.p |= FLAG_N;
        } else {
            self.p &= !FLAG_N;
        }
    }

    /// For PHP: push P with B=1 and bit5=1 (6502 convention).
    #[inline]
    pub fn for_push_brk(&self) -> u8 {
        self.p | FLAG_B | FLAG_U
    }

    /// Restore P from stack (PLP), clearing B and setting bit5 unconditionally.
    #[inline]
    pub fn restore_from_stack(&mut self, v: u8) {
        self.p = (v & !FLAG_B) | FLAG_U;
    }

    /// Serialize P to u8.
    #[inline]
    pub fn as_u8(&self) -> u8 {
        self.p
    }
}
