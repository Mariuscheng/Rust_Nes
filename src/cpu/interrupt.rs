//! Interrupt entry sequences and the interrupt poll.
//!
//! The 6502 samples NMI/IRQ on the cycle before an instruction's last one, and
//! the value it samples is the I flag as it was *on that cycle* — which is why
//! CLI/SEI delay an IRQ by one instruction while PLP/RTI and BRK do not. That
//! sample lives in [`CPU::poll_iflag`]; this module owns turning a pending
//! interrupt into the 7-cycle entry sequence.
//!
//! Everything here is a straight sequence of bus accesses, one per cycle, so
//! the cycle positions are explicit rather than implied by call order.

use super::core::CPU;
use crate::bus::BusInterface;
use crate::constants::{
    IRQ_VECTOR_HI, IRQ_VECTOR_LO, NMI_VECTOR_HI, NMI_VECTOR_LO, RESET_VECTOR_HI, RESET_VECTOR_LO,
};

impl CPU {
    /// Power-on / reset: jump through the reset vector.
    pub fn reset(&mut self, bus: &mut impl BusInterface) {
        let lo = bus.read(RESET_VECTOR_LO) as u16;
        let hi = bus.read(RESET_VECTOR_HI) as u16;
        self.pc = (hi << 8) | lo;
    }

    /// Whether the poll at the end of the current instruction takes an IRQ.
    ///
    /// Uses `poll_iflag`, i.e. the I flag as it was visible on the polling
    /// cycle (see the field docs), not the live flag.
    pub fn irq_pending(&self) -> bool {
        !self.poll_iflag
    }

    /// Execute NMI (non-maskable interrupt). Returns the 7 CPU cycles consumed.
    pub fn nmi(&mut self, bus: &mut dyn BusInterface) -> u16 {
        // c1-c2: internal cycles (dummy reads of the interrupted PC).
        let _ = bus.read(self.pc);
        let _ = bus.read(self.pc.wrapping_add(1));

        // c3-c5: push PC high, PC low, then status (B cleared for NMI).
        let pc = self.pc;
        self.stack_push_u16(bus, pc);
        self.stack_push_u8(bus, (self.p & 0xEF) | 0x20);

        // c6-c7: fetch the NMI vector.
        let lo = bus.read(NMI_VECTOR_LO) as u16;
        let hi = bus.read(NMI_VECTOR_HI) as u16;
        self.pc = (hi << 8) | lo;

        self.p |= 0x04;
        self.poll_iflag = true;
        self.poll_delay = 0;

        self.last_cycles = 7;
        7
    }

    /// IRQ: maskable interrupt. The caller has already decided via
    /// [`CPU::irq_pending`] that the poll takes it.
    pub fn irq(&mut self, bus: &mut dyn BusInterface) {
        // c1-c2: internal cycles (dummy reads of the interrupted PC).
        let _ = bus.read(self.pc);
        let _ = bus.read(self.pc.wrapping_add(1));

        // c3-c5: push PC high, PC low, then status (B cleared).
        let pc = self.pc;
        self.stack_push_u16(bus, pc);
        self.stack_push_u8(bus, (self.p & 0xEF) | 0x20);

        // I flag is set once the status byte is on the stack.
        self.p |= 0x04;
        self.poll_iflag = true;
        self.poll_delay = 0;

        // c6-c7: fetch the IRQ vector.
        let lo = bus.read(IRQ_VECTOR_LO) as u16;
        let hi = bus.read(IRQ_VECTOR_HI) as u16;
        self.pc = (hi << 8) | lo;

        self.last_cycles = 7;
    }

    fn stack_push_u16(&mut self, bus: &mut dyn BusInterface, data: u16) {
        crate::cpu::stack_flags::stack_push_u16(self, bus, data);
    }

    fn stack_push_u8(&mut self, bus: &mut dyn BusInterface, data: u8) {
        crate::cpu::stack_flags::stack_push_u8(self, bus, data);
    }
}
