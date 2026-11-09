//! src/timing.rs
//! Unit tests for the interrupt-poll model.
//!
//! The 6502 polls the interrupt lines during the last cycle of an instruction.
//! The I flag only takes effect for that poll if it was written *before* the
//! polling cycle, which is why CLI/SEI delay an IRQ by one instruction while
//! PLP/RTI/BRK do not. `CPU::poll_iflag` carries exactly that value.

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use crate::bus::BusInterface;
    use crate::cpu::CPU;

    /// Minimal bus implementation for testing timing logic.
    ///
    /// `cycles` must advance on every access/tick: `CPU::step` relies on
    /// `cpu_cycles()` to know how much of the instruction's budget is spent.
    struct DummyBus {
        mem: [u8; 65536],
        cpu_pc: u16,
        cycles: u64,
    }
    impl DummyBus {
        fn new() -> Self {
            Self {
                mem: [0; 65536],
                cpu_pc: 0,
                cycles: 0,
            }
        }
    }
    impl BusInterface for DummyBus {
        fn read(&mut self, addr: u16) -> u8 {
            self.cycles += 1;
            self.mem[addr as usize]
        }
        fn write(&mut self, addr: u16, data: u8) {
            self.cycles += 1;
            self.mem[addr as usize] = data;
        }
        fn set_cpu_pc(&mut self, pc: u16) {
            self.cpu_pc = pc;
        }
        fn take_immediate_stolen_cycles(&mut self) -> u16 {
            0
        }
        fn tick(&mut self) -> bool {
            self.cycles += 1;
            false
        }
        fn cpu_cycles(&self) -> u64 {
            self.cycles
        }
    }

    /// Step a program laid out at `addr`; returns the cycles consumed.
    fn step_at(cpu: &mut CPU, bus: &mut DummyBus, addr: u16, opcode: u8) -> u16 {
        bus.mem[addr as usize] = opcode;
        cpu.pc = addr;
        cpu.step(bus)
    }

    #[test]
    fn sei_poll_still_sees_cleared_i() {
        let mut cpu = CPU::new();
        let mut bus = DummyBus::new();

        cpu.set_irq_disable(false); // I=0 before SEI
        step_at(&mut cpu, &mut bus, 0x0000, 0x78); // SEI

        assert!(cpu.irq_disabled(), "SEI sets I");
        assert!(cpu.irq_pending(), "poll at the end of SEI still sees I=0");
    }

    #[test]
    fn cli_poll_still_sees_set_i() {
        let mut cpu = CPU::new();
        let mut bus = DummyBus::new();

        cpu.set_irq_disable(true); // I=1 before CLI
        step_at(&mut cpu, &mut bus, 0x0000, 0x58); // CLI

        assert!(!cpu.irq_disabled(), "CLI clears I");
        assert!(
            !cpu.irq_pending(),
            "poll at the end of CLI still sees I=1, delaying IRQ by one instruction"
        );
    }

    #[test]
    fn irq_is_taken_after_the_instruction_following_cli() {
        let mut cpu = CPU::new();
        let mut bus = DummyBus::new();

        // CLI at $1000, then NOPs. IRQ vector points at $8000.
        bus.mem[0x1000] = 0x58; // CLI
        bus.mem[0x1001] = 0xEA; // NOP
        bus.mem[0xFFFE] = 0x00;
        bus.mem[0xFFFF] = 0x80;
        cpu.pc = 0x1000;
        cpu.set_irq_disable(true);

        cpu.step(&mut bus); // CLI
        assert!(!cpu.irq_pending(), "no IRQ at the end of CLI");

        cpu.step(&mut bus); // NOP
        assert!(cpu.irq_pending(), "IRQ polled at the end of the next instruction");
    }

    #[test]
    fn plp_poll_still_sees_the_old_i_flag() {
        let mut cpu = CPU::new();
        let mut bus = DummyBus::new();

        // Stack: [P with I clear] at $01FF.
        cpu.set_irq_disable(true);
        bus.mem[0x01FF] = 0x20; // I clear, bit5 set
        cpu.sp = 0xFE;
        step_at(&mut cpu, &mut bus, 0x0000, 0x28); // PLP

        assert!(!cpu.irq_disabled(), "PLP restored I=0");
        assert!(
            !cpu.irq_pending(),
            "PLP loads P on the polling cycle, so this poll still sees I=1"
        );
    }

    #[test]
    fn brk_poll_sees_i_set_by_the_sequence() {
        let mut cpu = CPU::new();
        let mut bus = DummyBus::new();

        cpu.set_irq_disable(false);
        bus.mem[0xFFFE] = 0x00;
        bus.mem[0xFFFF] = 0x80;
        step_at(&mut cpu, &mut bus, 0x1000, 0x00); // BRK

        assert_eq!(cpu.pc, 0x8000, "BRK vectors through IRQ/BRK");
        assert!(
            !cpu.irq_pending(),
            "I is set during BRK, so its own poll cannot take an IRQ"
        );
    }

    #[test]
    fn taken_branch_polls_one_cycle_early() {
        let mut cpu = CPU::new();
        let mut bus = DummyBus::new();

        // $1000: BNE +0 (taken without page cross) — Z clear.
        bus.mem[0x1000] = 0xD0;
        bus.mem[0x1001] = 0x00;
        cpu.pc = 0x1000;
        cpu.p = 0x24;
        cpu.step(&mut bus);

        assert_eq!(cpu.poll_delay, 1, "taken branch moves the poll one cycle earlier");

        // Same branch, but crossing a page: the extra cycle precedes the poll.
        let mut bus2 = DummyBus::new();
        bus2.mem[0x10F0] = 0xD0;
        bus2.mem[0x10F1] = 0x20; // target $1112 — crosses $1100 page boundary
        cpu.p = 0x24;
        cpu.pc = 0x10F0;
        cpu.step(&mut bus2);
        assert_eq!(cpu.poll_delay, 0, "page-crossing branch polls on its last cycle");
    }

    #[test]
    fn irq_sequence_sets_i_and_vectors() {
        let mut cpu = CPU::new();
        let mut bus = DummyBus::new();

        cpu.set_irq_disable(false);
        cpu.pc = 0x1234;
        bus.mem[0xFFFE] = 0xCD;
        bus.mem[0xFFFF] = 0xAB;

        cpu.irq(&mut bus);
        assert_eq!(cpu.pc, 0xABCD, "IRQ vectors through $FFFE");
        assert!(cpu.irq_disabled(), "I flag set after IRQ");
        assert!(!cpu.irq_pending(), "I is set before the next poll");
    }

    #[test]
    fn irq_pending_follows_poll_iflag_not_live_flag() {
        let mut cpu = CPU::new();
        cpu.set_irq_disable(false);
        cpu.poll_iflag = true;
        assert!(
            !cpu.irq_pending(),
            "the poll uses the sampled I flag, not the live one"
        );
    }

    /// Diagnostic: run every opcode once and list those whose internal cycles
    /// are still trailing padding (i.e. not placed at their hardware cycle).
    ///
    /// `cargo test -- --ignored report_trailing_padding --nocapture`
    #[test]
    #[ignore]
    fn report_trailing_padding() {
        let mut report: Vec<(u8, u16, u16)> = Vec::new();
        for opcode in 0u16..=0xFF {
            let mut cpu = CPU::new();
            let mut bus = DummyBus::new();
            bus.mem[0] = opcode as u8;
            bus.mem[1] = 0x02;
            bus.mem[2] = 0x00;
            cpu.pc = 0;
            cpu.p = 0x24;
            let _ = cpu.step(&mut bus);
            let not_taken = cpu.padded[opcode as usize];
            let mut cpu2 = CPU::new();
            let mut bus2 = DummyBus::new();
            bus2.mem[0] = opcode as u8;
            bus2.mem[1] = 0x02;
            bus2.mem[2] = 0x00;
            cpu2.pc = 0;
            cpu2.p = 0x27; // C, Z, I set
            let _ = cpu2.step(&mut bus2);
            let taken = cpu2.padded[opcode as usize];
            if not_taken || taken {
                report.push((opcode as u8, cpu.last_cycles, cpu2.last_cycles));
            }
        }
        eprintln!("--- {} opcodes use trailing padding ---", report.len());
        let mut line = String::new();
        for (op, a, b) in &report {
            line.push_str(&format!("{op:02X}({a}/{b}) "));
        }
        eprintln!("{line}");
    }
}
