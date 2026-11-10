//! Cycle-level schedule vocabulary.
//!
//! The CPU's timing is "which cycle did what", and today that knowledge is
//! implicit: handlers perform bus accesses in the order they appear, and any
//! internal cycle is padded onto the end of the instruction. That makes timing
//! bugs invisible until a test ROM disagrees.
//!
//! This module starts making the schedule explicit. First step is a recording
//! bus plus assertions for representative instructions; once every opcode is
//! described as data, `CPU::step` can be driven by it instead of by call order.
//!
//! `Cycle` is what the *bus observes*, not what the CPU intends: a read of the
//! not-yet-corrected address shows up as its own `Read`, which is exactly the
//! dummy access hardware performs.

/// One CPU cycle as seen from the bus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cycle {
    /// Bus read at this address.
    Read(u16),
    /// Bus write of this value at this address.
    Write(u16, u8),
    /// A cycle with no bus access (internal work, or the tail of an implied
    /// instruction). This is the explicit form of what used to be padding.
    Idle,
}

/// Compact encoding of a [`Cycle`] for the frozen schedule table.
pub const KIND_READ: u8 = 0;
pub const KIND_WRITE: u8 = 1;
pub const KIND_IDLE: u8 = 2;

pub fn kind_of(cycle: Cycle) -> u8 {
    match cycle {
        Cycle::Read(_) => KIND_READ,
        Cycle::Write(..) => KIND_WRITE,
        Cycle::Idle => KIND_IDLE,
    }
}

#[cfg(test)]
mod tests {
    use super::{kind_of, Cycle};
    use crate::bus::CpuBus;
    use crate::cpu::CPU;

    /// A recording bus: behaves like RAM, records every cycle in order.
    struct RecordingBus {
        mem: Box<[u8; 65536]>,
        cycles: u64,
        log: Vec<Cycle>,
    }

    impl RecordingBus {
        fn new() -> Self {
            Self {
                mem: vec![0u8; 65536]
                    .into_boxed_slice()
                    .try_into()
                    .expect("fixed 64 KiB"),
                cycles: 0,
                log: Vec::new(),
            }
        }
    }

    impl CpuBus for RecordingBus {
        fn read(&mut self, addr: u16) -> u8 {
            self.cycles += 1;
            self.log.push(Cycle::Read(addr));
            self.mem[addr as usize]
        }
        fn write(&mut self, addr: u16, data: u8) {
            self.cycles += 1;
            self.log.push(Cycle::Write(addr, data));
            self.mem[addr as usize] = data;
        }
        fn tick(&mut self) -> bool {
            self.cycles += 1;
            self.log.push(Cycle::Idle);
            false
        }
        fn set_cpu_pc(&mut self, _pc: u16) {}
        fn take_immediate_stolen_cycles(&mut self) -> u16 {
            0
        }
        fn cpu_cycles(&self) -> u64 {
            self.cycles
        }
        fn take_nmi_edge(&mut self) -> bool {
            false
        }
    }

    /// Run one instruction at `pc`, with A/X/Y set up by the caller.
    fn run(cpu: &mut CPU, bus: &mut RecordingBus, pc: u16, opcode: u8) {
        bus.mem[pc as usize] = opcode;
        cpu.pc = pc;
        let _ = cpu.step(bus);
    }

    #[test]
    fn lda_zp_x_reads_base_then_indexed() {
        let mut cpu = CPU::new();
        let mut bus = RecordingBus::new();
        bus.mem[0x8001] = 0x10; // zero-page operand
        bus.mem[0x0010] = 0x42;
        bus.mem[0x0012] = 0x42; // value at base+X
        cpu.x = 0x02;
        run(&mut cpu, &mut bus, 0x8000, 0xB5); // LDA $10,X

        assert_eq!(cpu.a, 0x42);
        assert_eq!(
            bus.log,
            vec![
                Cycle::Read(0x8000), // opcode fetch
                Cycle::Read(0x8001), // zero-page operand
                Cycle::Read(0x0010), // cycle 3: un-indexed read (dummy)
                Cycle::Read(0x0012), // effective address
            ]
        );
    }

    #[test]
    fn lda_abs_x_page_cross_reads_the_wrong_address_first() {
        let mut cpu = CPU::new();
        let mut bus = RecordingBus::new();
        bus.mem[0x8001] = 0x00;
        bus.mem[0x8002] = 0x20; // base $2000
        bus.mem[0x2000] = 0x99;
        cpu.x = 0x00;
        run(&mut cpu, &mut bus, 0x8000, 0xBD); // LDA $2000,X

        assert_eq!(
            bus.log,
            vec![
                Cycle::Read(0x8000),
                Cycle::Read(0x8001),
                Cycle::Read(0x8002),
                Cycle::Read(0x2000),
            ],
            "no page cross: straight to the effective address"
        );

        let mut cpu = CPU::new();
        let mut bus = RecordingBus::new();
        bus.mem[0x8001] = 0xFF;
        bus.mem[0x8002] = 0x20; // base $20FF
        bus.mem[0x2104] = 0x77; // value at $20FF + $05
        cpu.x = 0x05;
        run(&mut cpu, &mut bus, 0x8000, 0xBD);

        assert_eq!(
            bus.log,
            vec![
                Cycle::Read(0x8000),
                Cycle::Read(0x8001),
                Cycle::Read(0x8002),
                Cycle::Read(0x2004), // dummy read at the un-carried address
                Cycle::Read(0x2104),
            ],
            "page cross: hardware reads the wrong address first"
        );
    }

    #[test]
    fn rmw_abs_writes_the_old_value_then_the_new_one() {
        let mut cpu = CPU::new();
        let mut bus = RecordingBus::new();
        bus.mem[0x8001] = 0x00;
        bus.mem[0x8002] = 0x30;
        bus.mem[0x3000] = 0x01;
        run(&mut cpu, &mut bus, 0x8000, 0xEE); // INC $3000

        assert_eq!(
            bus.log,
            vec![
                Cycle::Read(0x8000),
                Cycle::Read(0x8001),
                Cycle::Read(0x8002),
                Cycle::Read(0x3000),
                Cycle::Write(0x3000, 0x01), // old value (dummy write)
                Cycle::Write(0x3000, 0x02), // result
            ]
        );
    }

    /// Regenerate `src/cpu/schedule_table.rs` from the current handlers.
    ///
    /// `cargo test -- --ignored regenerate_schedule_table`
    ///
    /// The table freezes the access *shape* of every opcode (read / write /
    /// idle, in order). Any handler change that reorders or drops an access
    /// fails `schedule_table_matches_handlers` instead of silently changing
    /// timing.
    #[test]
    #[ignore]
    fn regenerate_schedule_table() {
        use std::fmt::Write as _;
        let mut out = String::new();
        out.push_str("// GENERATED by cpu::schedule::tests::regenerate_schedule_table.\n");
        out.push_str("// Do not edit by hand: rerun that test instead.\n");
        out.push_str("pub const SHAPES: [&[u8]; 256] = [\n");
        for opcode in 0u16..=0xFF {
            let mut cpu = CPU::new();
            let mut bus = RecordingBus::new();
            bus.mem[1] = 0x02;
            bus.mem[2] = 0x00;
            run(&mut cpu, &mut bus, 0, opcode as u8);
            out.push_str("    &[");
            for (i, cycle) in bus.log.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write!(out, "{}", kind_of(*cycle)).unwrap();
            }
            out.push_str("],\n");
        }
        out.push_str("];\n");
        std::fs::write("src/cpu/schedule_table.rs", out).expect("write schedule table");
    }

    fn observed_shape(opcode: u8) -> Vec<u8> {
        let mut cpu = CPU::new();
        let mut bus = RecordingBus::new();
        bus.mem[1] = 0x02;
        bus.mem[2] = 0x00;
        run(&mut cpu, &mut bus, 0, opcode);
        bus.log.iter().map(|c| kind_of(*c)).collect()
    }

    /// Every opcode's access shape must match the frozen table.
    #[test]
    fn schedule_table_matches_handlers() {
        let table = crate::cpu::schedule_table::SHAPES;
        for opcode in 0u16..=0xFF {
            let expected = table[opcode as usize];
            let actual = observed_shape(opcode as u8);
            assert_eq!(
                actual, expected,
                "opcode {opcode:#04X}: access shape changed; check the handler's \
                 cycle placement, then rerun regenerate_schedule_table if the \
                 change is intended"
            );
        }
    }
}
