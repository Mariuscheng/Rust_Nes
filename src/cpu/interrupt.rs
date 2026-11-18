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
use crate::bus::CpuBus;
use crate::constants::{
    IRQ_VECTOR_HI, IRQ_VECTOR_LO, NMI_VECTOR_HI, NMI_VECTOR_LO, RESET_VECTOR_HI, RESET_VECTOR_LO,
};

impl CPU {
    /// Power-on / reset: the 7-cycle reset sequence.
    ///
    /// Unlike NMI/IRQ this pushes nothing. It reads `PC`, `PC+1` and then the
    /// three stack addresses `$0100|S`, `|S-1`, `|S-2` as *reads* (the 6502
    /// drives those addresses while it decrements S), decrements S by three,
    /// sets I, and finally fetches the reset vector. A, X, Y, the decimal flag
    /// and the previous stack contents are left alone, which is what lets a
    /// game distinguish a reset from a cold boot.
    pub fn reset(&mut self, bus: &mut impl CpuBus) {
        // c1-c2: dummy reads of the (stale) program counter.
        let _ = bus.read(self.pc);
        let _ = bus.read(self.pc.wrapping_add(1));

        // c3-c5: S is decremented three times, with a read at each address.
        for _ in 0..3 {
            let _ = bus.read(0x0100 | self.sp as u16);
            self.sp = self.sp.wrapping_sub(1);
        }

        // c6-c7: vector fetch.
        let lo = bus.read(RESET_VECTOR_LO) as u16;
        let hi = bus.read(RESET_VECTOR_HI) as u16;
        self.pc = (hi << 8) | lo;

        self.p |= 0x04;
        self.poll_iflag = true;
        self.poll_delay = 0;
        self.nmi_polled = false;
    }

    /// Whether the poll at the end of the current instruction takes an IRQ.
    ///
    /// Uses `poll_iflag`, i.e. the I flag as it was visible on the polling
    /// cycle (see the field docs), not the live flag.
    pub fn irq_pending(&self) -> bool {
        !self.poll_iflag
    }

    /// Execute NMI (non-maskable interrupt). Returns the 7 CPU cycles consumed.
    pub fn nmi(&mut self, bus: &mut dyn CpuBus) -> u16 {
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

        7
    }

    /// IRQ: maskable interrupt. The caller has already decided via
    /// [`CPU::irq_pending`] that the poll takes it.
    ///
    /// An NMI asserted during the first cycles of this sequence hijacks it: the
    /// frame pushed here is kept (B cleared, I not yet set) but the NMI vector
    /// is fetched instead. The NMI is sampled with the same timing BRK uses
    /// (`op_brk`): the poll happens on the status-push cycle and sees the line
    /// as of the previous cycle, so edges latched on c1-c4 win and an edge on
    /// c5 or later is serviced after the following instruction
    /// (cpu_interrupts 3-nmi_and_irq).
    pub fn irq(&mut self, bus: &mut dyn CpuBus) {
        // c1-c2: internal cycles (dummy reads of the interrupted PC).
        let _ = bus.read(self.pc);
        let _ = bus.read(self.pc.wrapping_add(1));

        // c3-c5: push PC high, PC low, then status (B cleared).
        let pc = self.pc;
        self.stack_push_u16(bus, pc);
        self.stack_push_u8(bus, (self.p & 0xEF) | 0x20);

        // c5: sample NMI before the vector fetch; a pending edge redirects it.
        let nmi_taken = bus.take_nmi_edge();

        // I flag is set once the status byte is on the stack.
        self.p |= 0x04;
        self.poll_iflag = true;
        self.poll_delay = 0;

        // c6-c7: fetch the NMI vector when hijacked, else the IRQ vector.
        let (lo_addr, hi_addr) = if nmi_taken {
            (NMI_VECTOR_LO, NMI_VECTOR_HI)
        } else {
            (IRQ_VECTOR_LO, IRQ_VECTOR_HI)
        };
        let lo = bus.read(lo_addr) as u16;
        let hi = bus.read(hi_addr) as u16;
        self.pc = (hi << 8) | lo;
    }

    fn stack_push_u16(&mut self, bus: &mut dyn CpuBus, data: u16) {
        crate::cpu::stack_flags::stack_push_u16(self, bus, data);
    }

    fn stack_push_u8(&mut self, bus: &mut dyn CpuBus, data: u8) {
        crate::cpu::stack_flags::stack_push_u8(self, bus, data);
    }
}
