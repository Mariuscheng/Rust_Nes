//! src/timing.rs
//! Unit tests for interrupt timing logic.
//!
//! NOTE: Most timing coordination has been moved into nes.rs and cpu::core.rs.

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use crate::bus::BusInterface;
    use crate::cpu::CPU;

    /// Minimal bus implementation for testing timing logic.
    struct DummyBus {
        mem: [u8; 65536],
        cpu_pc: u16,
    }
    impl DummyBus {
        fn new() -> Self {
            Self {
                mem: [0; 65536],
                cpu_pc: 0,
            }
        }
    }
    impl BusInterface for DummyBus {
        fn read(&self, addr: u16) -> u8 {
            self.mem[addr as usize]
        }
        fn write(&mut self, addr: u16, data: u8) {
            self.mem[addr as usize] = data;
        }
        fn set_cpu_pc(&mut self, pc: u16) {
            self.cpu_pc = pc;
        }
        fn take_immediate_stolen_cycles(&mut self) -> u16 {
            0
        }
        fn advance_ppu_cycle(&mut self) -> bool {
            false
        }
    }

    /// After SEI (I=1 with no allow counter), IRQ is blocked.
    #[test]
    fn sei_blocks_irq_when_no_allow_counter() {
        let mut cpu = CPU::new();
        let mut bus = DummyBus::new();

        // I=1, no allow counter (simulating SEI where I was already 1, or counter expired)
        cpu.set_irq_disable(true);
        cpu.irq_allow_counter = 0;

        cpu.irq(&mut bus);

        assert_eq!(cpu.pc, 0, "I=1 without allow counter should block IRQ");
    }

    /// During SEI execution (I transitions 0→1 with allow counter=2),
    /// IRQ is allowed through — matching 6502 behavior where IRQ is
    /// sampled on the penultimate cycle (before SEI sets I on the last cycle).
    #[test]
    fn sei_allow_counter_allows_irq_during_sei() {
        let mut cpu = CPU::new();
        let mut bus = DummyBus::new();

        // Simulate SEI: transition I from 0→1, set allow counter to 2
        cpu.set_irq_disable(false);
        cpu.set_irq_disable(true); // SEI effect
        cpu.irq_allow_counter = 2;

        // Set up IRQ vector
        bus.mem[0xFFFE] = 0xCD;
        bus.mem[0xFFFF] = 0xAB;

        // IRQ should be taken despite I=1 because allow counter > 0
        cpu.irq(&mut bus);
        assert_eq!(cpu.pc, 0xABCD, "SEI allow counter should allow IRQ during SEI");

        // Allow counter decrements after instruction → now at 1
        // Set PC back, call irq() again — still allowed
        cpu.set_irq_disable(true); // reset I after IRQ cleared it
        cpu.pc = 0x2000;
        cpu.irq_allow_counter = 0; // simulate counter expired
        cpu.irq(&mut bus);
        assert_eq!(cpu.pc, 0x2000, "IRQ blocked after allow counter expired");
    }

    /// CLI followed by SEI: only one IRQ is allowed (right after SEI),
    /// matching test 5 of the CLI latency test suite.
    #[test]
    fn cli_sei_allows_only_one_irq_after_sei() {
        let mut cpu = CPU::new();
        let mut bus = DummyBus::new();

        // Simulate CLI: I→0, inhibit=2
        cpu.set_irq_disable(false);
        cpu.irq_inhibit_counter = 2;
        // Simulate SEI immediately after: I→1, allow=2
        cpu.set_irq_disable(true);
        cpu.irq_allow_counter = 2;

        bus.mem[0xFFFE] = 0xCD;
        bus.mem[0xFFFF] = 0xAB;

        // First IRQ: allowed (I=1 but allow counter > 0)
        cpu.pc = 0x1000;
        cpu.irq(&mut bus);
        assert_eq!(cpu.pc, 0xABCD, "CLI+SEI: first IRQ allowed via allow counter");

        // After IRQ taken, both counters cleared by irq()
        // Second IRQ: blocked (I=1, allow=0)
        cpu.set_irq_disable(true);
        cpu.pc = 0x2000;
        cpu.irq(&mut bus);
        assert_eq!(cpu.pc, 0x2000, "CLI+SEI: only one IRQ allowed");
    }

    /// IRQ blocked when I=1 and allow counter is 0 (SEI/PLP delay expired).
    #[test]
    fn sei_no_allow_counter_when_i_stays_set() {
        let mut cpu = CPU::new();
        let mut bus = DummyBus::new();

        // I already 1, simulate SEI (no transition)
        cpu.set_irq_disable(true);
        cpu.irq_allow_counter = 0; // op_sei would not set allow counter because old_i != 0

        bus.mem[0xFFFE] = 0xCD;
        bus.mem[0xFFFF] = 0xAB;
        cpu.pc = 0x1000;
        cpu.irq(&mut bus);
        assert_eq!(cpu.pc, 0x1000, "IRQ blocked: I=1 and allow counter=0");
    }

    #[test]
    fn cli_delays_irq_by_two_instructions() {
        let mut cpu = CPU::new();
        let mut bus = DummyBus::new();

        // Start with I=1 (interrupts disabled)
        cpu.set_irq_disable(true);

        // Simulate CLI: clear I flag and set inhibit counter to 2
        cpu.set_irq_disable(false);
        cpu.irq_inhibit_counter = 2;

        // First instruction after CLI: IRQ blocked (counter=2 → check passes)
        assert!(cpu.irq_inhibit_counter > 0, "Counter should be 2");
        // irq() should be blocked
        cpu.irq(&mut bus);
        assert_eq!(cpu.pc, 0, "IRQ blocked on instruction after CLI (counter=2)");
        cpu.irq_inhibit_counter -= 1; // first instruction ends

        // Second instruction after CLI: IRQ blocked (counter=1 → check passes)
        cpu.irq(&mut bus);
        assert_eq!(cpu.pc, 0, "IRQ blocked on 2nd instruction after CLI (counter=1)");
        cpu.irq_inhibit_counter -= 1; // second instruction ends

        // Third instruction after CLI: IRQ allowed (counter=0)
        // Set up IRQ vector so it doesn't crash
        bus.mem[0xFFFE] = 0x00;
        bus.mem[0xFFFF] = 0x80;
        cpu.irq(&mut bus);
        // IRQ should have been taken → PC should be 0x8000
        assert_eq!(cpu.pc, 0x8000, "IRQ taken on 3rd instruction after CLI (counter=0)");
        // I flag should be set after IRQ
        assert!(cpu.irq_disabled(), "I flag set after IRQ");
    }

    #[test]
    fn irq_not_taken_when_i_flag_set() {
        let mut cpu = CPU::new();
        let mut bus = DummyBus::new();

        // Set I flag
        cpu.set_irq_disable(true);
        cpu.pc = 0x1234;

        cpu.irq(&mut bus);
        assert_eq!(cpu.pc, 0x1234, "IRQ not taken when I=1");
    }

    #[test]
    fn irq_taken_when_i_flag_clear() {
        let mut cpu = CPU::new();
        let mut bus = DummyBus::new();

        // Clear I flag
        cpu.set_irq_disable(false);
        cpu.pc = 0x1234;

        // Set up valid IRQ vector
        bus.mem[0xFFFE] = 0xCD;
        bus.mem[0xFFFF] = 0xAB;

        cpu.irq(&mut bus);
        assert_eq!(cpu.pc, 0xABCD, "IRQ taken when I=0");
        assert!(cpu.irq_disabled(), "I flag set after IRQ");
    }

    #[test]
    fn nmi_sets_i_flag_and_clears_inhibit_counter() {
        let mut cpu = CPU::new();
        let mut bus = DummyBus::new();

        // Simulate that there was a pending inhibit (CLI delay)
        cpu.irq_inhibit_counter = 2;
        cpu.set_irq_disable(false); // I=0

        // Set up NMI vector
        bus.mem[0xFFFA] = 0x34;
        bus.mem[0xFFFB] = 0x12;

        cpu.nmi(&mut bus);
        assert_eq!(cpu.pc, 0x1234, "NMI jumps to vector");
        assert!(cpu.irq_disabled(), "I flag set after NMI");
        assert_eq!(cpu.irq_inhibit_counter, 0, "Inhibit counter cleared by NMI");
    }

    #[test]
    fn irq_inhibit_counter_persists_across_step() {
        let mut cpu = CPU::new();
        let mut bus = DummyBus::new();

        // Simulate CLI: set counter to 2
        cpu.set_irq_disable(false);
        cpu.irq_inhibit_counter = 2;

        // Set up NOPs at consecutive addresses so step() doesn't hit BRK (0x00).
        bus.mem[0x0000] = 0xEA; // NOP (implied, size=1)
        bus.mem[0x0001] = 0xEA; // NOP (implied, size=1)
        bus.mem[0x0002] = 0xEA; // NOP (implied, size=1)
        bus.mem[0xFFFE] = 0x00;
        bus.mem[0xFFFF] = 0x80;

        // First step: counter decrements from 2→1
        let _ = cpu.step(&mut bus);
        assert_eq!(cpu.irq_inhibit_counter, 1, "Counter decremented after step 1");

        // Second step: counter decrements from 1→0
        let _ = cpu.step(&mut bus);
        assert_eq!(cpu.irq_inhibit_counter, 0, "Counter decremented from 1 to 0 after step 2");

        // Third step: counter stays 0
        let _ = cpu.step(&mut bus);
        assert_eq!(cpu.irq_inhibit_counter, 0, "Counter stays 0 after step 3");
    }
}

