use crate::bus::CpuBus;
use crate::cpu::decoder::{decode_opcode, Instruction};
use crate::cpu::ops;

/// 派發無回傳值的 opcode handler（Flag set/clear、Register transfer、Stack、Jump 等）
macro_rules! dispatch_void {
    ($self:expr, $bus:expr, $opcode:expr, $pc:expr, $fn:ident) => {
        ops::$fn($self, $bus, $opcode.mode, $pc)
    };
}

/// 派發可能回傳額外 cycle（page cross penalty）的 opcode handler
macro_rules! dispatch_extra {
    ($self:expr, $bus:expr, $opcode:expr, $pc:expr, $fn:ident, $extra:expr) => {{
        let c = ops::$fn($self, $bus, $opcode.mode, $pc);
        if c > 0 {
            $extra = $extra.wrapping_add(c);
        }
    }};
}

pub struct CPU {
    pub a: u8,
    pub x: u8,
    pub y: u8,
    pub pc: u16,
    pub sp: u8,
    pub p: u8,
    /// I flag as visible to the interrupt poll at the end of the current
    /// instruction. Initialised from the live flag at instruction start;
    /// instructions that load status (PLP/RTI) or set I mid-sequence
    /// (BRK/IRQ/NMI) update it, while CLI/SEI deliberately do not — their
    /// write lands on the last cycle, so the poll still sees the old value
    /// (that is the one-instruction IRQ delay after CLI).
    pub poll_iflag: bool,
    /// How many cycles before the last one the current instruction polls the
    /// interrupt lines. 0 = last cycle (normal), 1 = taken branch without a
    /// page cross, whose extra cycle is inserted after the poll point.
    pub poll_delay: u8,
    /// Total CPU cycles consumed by the last instruction (set by step()).
    pub last_cycles: u16,
    /// Diagnostic: opcodes that needed trailing padding (internal cycles not
    /// yet placed at their hardware position). Remove with the padding itself.
    pub padded: [bool; 256],
}

/// Opcodes whose internal cycle(s) legitimately sit at the end of the
/// instruction, so the cycle budget may be topped up with a bare tick:
/// 2-cycle implied instructions, taken branches and RTS/RTI. Everything else
/// must place its internal cycles at their hardware position.
const PADDING_ALLOWED: [u8; 38] = [
    0x0A, 0x10, 0x18, 0x1A, 0x2A, 0x30, 0x38, 0x3A, 0x40, 0x4A, 0x50, 0x58, 0x5A, 0x60, 0x6A,
    0x70, 0x78, 0x7A, 0x88, 0x8A, 0x90, 0x98, 0x9A, 0xA8, 0xAA, 0xB0, 0xB8, 0xBA, 0xC8, 0xCA,
    0xD0, 0xD8, 0xDA, 0xE8, 0xEA, 0xF0, 0xF8, 0xFA,
];

impl CPU {
    pub fn new() -> Self {
        Self {
            a: 0,
            x: 0,
            y: 0,
            pc: 0,
            sp: 0xFD,
            p: 0x24,
            poll_iflag: true,
            poll_delay: 0,
            last_cycles: 0,
            padded: [false; 256],
        }
    }

    /// Execute one CPU instruction. Returns consumed CPU cycles.
    /// The caller is responsible for PPU ticking and interrupt dispatch.
    pub fn step(&mut self, bus: &mut dyn CpuBus) -> u16 {
        let current_pc = self.pc;
        let start_cycles = bus.cpu_cycles();

        bus.set_cpu_pc(current_pc);

        let opcode_byte = bus.read(current_pc);
        let opcode = decode_opcode(opcode_byte)
            .unwrap_or_else(|| panic!("未知指令: {:#04X} 位於 {:#06X}", opcode_byte, current_pc));

        let next_pc = current_pc.wrapping_add(opcode.size as u16);
        self.pc = next_pc;

        // Reset per-instruction poll state. The I flag as sampled here is what
        // the end-of-instruction poll sees; CLI/SEI deliberately leave it
        // alone because their write lands on the last cycle.
        self.poll_iflag = self.irq_disabled();
        self.poll_delay = 0;

        let mut extra_cycles: u8 = 0;
        match opcode.op {
            // ── Flag set/clear (void) ──
            Instruction::SEI => dispatch_void!(self, bus, opcode, current_pc, op_sei),
            Instruction::SEC => dispatch_void!(self, bus, opcode, current_pc, op_sec),
            Instruction::CLC => dispatch_void!(self, bus, opcode, current_pc, op_clc),
            Instruction::CLD => dispatch_void!(self, bus, opcode, current_pc, op_cld),
            Instruction::CLI => dispatch_void!(self, bus, opcode, current_pc, op_cli),
            Instruction::CLV => dispatch_void!(self, bus, opcode, current_pc, op_clv),
            Instruction::SED => dispatch_void!(self, bus, opcode, current_pc, op_sed),

            // ── Register transfer (void) ──
            Instruction::TXS => dispatch_void!(self, bus, opcode, current_pc, op_txs),
            Instruction::TSX => dispatch_void!(self, bus, opcode, current_pc, op_tsx),
            Instruction::TAX => dispatch_void!(self, bus, opcode, current_pc, op_tax),
            Instruction::TAY => dispatch_void!(self, bus, opcode, current_pc, op_tay),
            Instruction::TXA => dispatch_void!(self, bus, opcode, current_pc, op_txa),
            Instruction::TYA => dispatch_void!(self, bus, opcode, current_pc, op_tya),
            Instruction::INX => dispatch_void!(self, bus, opcode, current_pc, op_inx),
            Instruction::INY => dispatch_void!(self, bus, opcode, current_pc, op_iny),
            Instruction::DEX => dispatch_void!(self, bus, opcode, current_pc, op_dex),
            Instruction::DEY => dispatch_void!(self, bus, opcode, current_pc, op_dey),

            // ── Load (extra) ──
            Instruction::LDX => dispatch_extra!(self, bus, opcode, current_pc, op_ldx, extra_cycles),
            Instruction::LDA => dispatch_extra!(self, bus, opcode, current_pc, op_lda, extra_cycles),
            Instruction::LDY => dispatch_extra!(self, bus, opcode, current_pc, op_ldy, extra_cycles),

            // ── Store (void) ──
            Instruction::STA => dispatch_void!(self, bus, opcode, current_pc, op_sta),
            Instruction::STX => dispatch_void!(self, bus, opcode, current_pc, op_stx),
            Instruction::STY => dispatch_void!(self, bus, opcode, current_pc, op_sty),

            // ── ALU (extra) ──
            Instruction::ORA => dispatch_extra!(self, bus, opcode, current_pc, op_ora, extra_cycles),
            Instruction::AND => dispatch_extra!(self, bus, opcode, current_pc, op_and, extra_cycles),
            Instruction::EOR => dispatch_extra!(self, bus, opcode, current_pc, op_eor, extra_cycles),
            Instruction::ADC => dispatch_extra!(self, bus, opcode, current_pc, op_adc, extra_cycles),
            Instruction::SBC => dispatch_extra!(self, bus, opcode, current_pc, op_sbc, extra_cycles),

            // ── Compare (extra) ──
            Instruction::CMP => dispatch_extra!(self, bus, opcode, current_pc, op_cmp, extra_cycles),
            Instruction::CPX => dispatch_extra!(self, bus, opcode, current_pc, op_cpx, extra_cycles),
            Instruction::CPY => dispatch_extra!(self, bus, opcode, current_pc, op_cpy, extra_cycles),

            // ── Shift/Rotate (extra) ──
            Instruction::ASL => dispatch_extra!(self, bus, opcode, current_pc, op_asl, extra_cycles),
            Instruction::LSR => dispatch_extra!(self, bus, opcode, current_pc, op_lsr, extra_cycles),
            Instruction::ROL => dispatch_extra!(self, bus, opcode, current_pc, op_rol, extra_cycles),
            Instruction::ROR => dispatch_extra!(self, bus, opcode, current_pc, op_ror, extra_cycles),

            // ── Memory ops (extra) ──
            Instruction::BIT => dispatch_extra!(self, bus, opcode, current_pc, op_bit, extra_cycles),
            Instruction::DEC => dispatch_extra!(self, bus, opcode, current_pc, op_dec, extra_cycles),
            Instruction::INC => dispatch_extra!(self, bus, opcode, current_pc, op_inc, extra_cycles),

            // ── Stack (void) ──
            Instruction::PHA => dispatch_void!(self, bus, opcode, current_pc, op_pha),
            Instruction::PLA => dispatch_void!(self, bus, opcode, current_pc, op_pla),
            Instruction::PHP => dispatch_void!(self, bus, opcode, current_pc, op_php),
            Instruction::PLP => dispatch_void!(self, bus, opcode, current_pc, op_plp),

            // ── Jump/Subroutine/Interrupt (void) ──
            Instruction::JSR => dispatch_void!(self, bus, opcode, current_pc, op_jsr),
            Instruction::RTS => dispatch_void!(self, bus, opcode, current_pc, op_rts),
            Instruction::JMP => dispatch_void!(self, bus, opcode, current_pc, op_jmp),
            Instruction::BRK => dispatch_void!(self, bus, opcode, current_pc, op_brk),
            Instruction::RTI => dispatch_void!(self, bus, opcode, current_pc, op_rti),

            // ── Branch (extra) ──
            Instruction::BPL => dispatch_extra!(self, bus, opcode, current_pc, op_bpl, extra_cycles),
            Instruction::BMI => dispatch_extra!(self, bus, opcode, current_pc, op_bmi, extra_cycles),
            Instruction::BVC => dispatch_extra!(self, bus, opcode, current_pc, op_bvc, extra_cycles),
            Instruction::BVS => dispatch_extra!(self, bus, opcode, current_pc, op_bvs, extra_cycles),
            Instruction::BCC => dispatch_extra!(self, bus, opcode, current_pc, op_bcc, extra_cycles),
            Instruction::BCS => dispatch_extra!(self, bus, opcode, current_pc, op_bcs, extra_cycles),
            Instruction::BEQ => dispatch_extra!(self, bus, opcode, current_pc, op_beq, extra_cycles),
            Instruction::BNE => dispatch_extra!(self, bus, opcode, current_pc, op_bne, extra_cycles),

            // ── NOP (void) ──
            Instruction::NOP => dispatch_extra!(self, bus, opcode, current_pc, op_nop, extra_cycles),

            // ── Unofficial opcodes ──
            Instruction::SLO => dispatch_extra!(self, bus, opcode, current_pc, op_slo, extra_cycles),
            Instruction::RLA => dispatch_extra!(self, bus, opcode, current_pc, op_rla, extra_cycles),
            Instruction::SRE => dispatch_extra!(self, bus, opcode, current_pc, op_sre, extra_cycles),
            Instruction::RRA => dispatch_extra!(self, bus, opcode, current_pc, op_rra, extra_cycles),
            Instruction::SAX => dispatch_void!(self, bus, opcode, current_pc, op_sax),
            Instruction::LAX => dispatch_extra!(self, bus, opcode, current_pc, op_lax, extra_cycles),
            Instruction::DCP => dispatch_extra!(self, bus, opcode, current_pc, op_dcp, extra_cycles),
            Instruction::ISC => dispatch_extra!(self, bus, opcode, current_pc, op_isc, extra_cycles),
            Instruction::ANC => dispatch_void!(self, bus, opcode, current_pc, op_anc),
            Instruction::ALR => dispatch_void!(self, bus, opcode, current_pc, op_alr),
            Instruction::ARR => dispatch_void!(self, bus, opcode, current_pc, op_arr),
            Instruction::AXS => dispatch_void!(self, bus, opcode, current_pc, op_axs),
            Instruction::KIL => dispatch_void!(self, bus, opcode, current_pc, op_kil),
            Instruction::ANX => dispatch_void!(self, bus, opcode, current_pc, op_anx),
            Instruction::LAS => dispatch_extra!(self, bus, opcode, current_pc, op_las, extra_cycles),
            Instruction::SYA => dispatch_void!(self, bus, opcode, current_pc, op_sya),
            Instruction::SXA => dispatch_void!(self, bus, opcode, current_pc, op_sxa),
            Instruction::AHX => dispatch_void!(self, bus, opcode, current_pc, op_ahx),
            Instruction::TAS => dispatch_void!(self, bus, opcode, current_pc, op_tas),
        }

        // ── Calculate total cycles ──
        let base_cycles = opcode.cycles as u16;
        let mut total_cycles: u16 = base_cycles + extra_cycles as u16;

        // Bus accesses already advanced one cycle each while the handler ran;
        // pad the remaining internal cycles.
        let used = (bus.cpu_cycles() - start_cycles) as u16;
        debug_assert!(
            used <= total_cycles,
            "opcode {:#04X} at {:#06X} used {} cycles but declares {}",
            opcode_byte,
            current_pc,
            used,
            total_cycles
        );
        for _ in used..total_cycles {
            bus.tick();
        }
        if used < total_cycles {
            // Trailing padding: some internal cycle of this instruction is not
            // yet placed at its hardware position. Only the instructions whose
            // internal cycle genuinely is their last cycle may rely on this
            // (see docs/cycle-exact-refactor.md, P1).
            debug_assert!(
                crate::cpu::schedule::allows_trailing(opcode_byte, total_cycles - used),
                "opcode {:#04X} at {:#06X}: {} trailing idle cycles do not match \
                 the schedule table",
                opcode_byte,
                current_pc,
                total_cycles - used
            );
            self.padded[opcode_byte as usize] = true;
        }

        // DMC DMA raised during the instruction (or while padding) stalls the
        // CPU for 4 cycles; pay those now. Anything raised while paying them is
        // picked up by the next instruction.
        let stolen = bus.take_immediate_stolen_cycles();
        if stolen > 0 {
            total_cycles = total_cycles.saturating_add(stolen);
            let used = (bus.cpu_cycles() - start_cycles) as u16;
            for _ in used..total_cycles {
                bus.tick();
            }
        }

        // Decrement interrupt delay counters after instruction completes.
        // This is called here so that unit tests (which call step() directly)
        // also get correct counter behavior.
        self.last_cycles = total_cycles;
        total_cycles
    }


}