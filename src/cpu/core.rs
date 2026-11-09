use crate::bus::BusInterface;
use crate::constants::{
    IRQ_VECTOR_HI, IRQ_VECTOR_LO, NMI_VECTOR_HI, NMI_VECTOR_LO,
    RESET_VECTOR_HI, RESET_VECTOR_LO,
};
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
    /// IRQ inhibit counter for CLI/PLP delay (clears I flag).
    /// Set to 2, decremented each step. IRQ blocked while counter > 0.
    pub irq_inhibit_counter: u8,
    /// IRQ allow counter for SEI/PLP delay (sets I flag).
    /// Set to 2, decremented each step. IRQ allowed regardless of I while counter > 0.
    pub irq_allow_counter: u8,
    /// NMI edge-detection: previous state of the NMI line.
    pub prev_nmi_line: bool,
    /// NMI latched (edge detected). Consumed by poll_nmi() before each instruction.
    pub nmi_latch: bool,
    /// Skip IRQ polling after current instruction (taken branch, no page-cross).
    pub skip_irq: bool,
    /// Total CPU cycles consumed by the last instruction (set by step()).
    pub last_cycles: u16,
    /// PPU ticks already consumed during this instruction (e.g., BRK pre-check).
    /// The main loop subtracts this from (last_cycles * 3) to avoid double-ticking.
    pub ppu_ticks_consumed: u16,
}

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
    pub fn carry(&self) -> bool { (self.p & FLAG_C) != 0 }
    #[inline]
    pub fn set_carry(&mut self, v: bool) { if v { self.p |= FLAG_C; } else { self.p &= !FLAG_C; } }

    #[inline]
    pub fn zero(&self) -> bool { (self.p & FLAG_Z) != 0 }
    #[inline]
    pub fn set_zero(&mut self, v: bool) { if v { self.p |= FLAG_Z; } else { self.p &= !FLAG_Z; } }

    #[inline]
    pub fn irq_disabled(&self) -> bool { (self.p & FLAG_I) != 0 }
    #[inline]
    pub fn set_irq_disable(&mut self, v: bool) { if v { self.p |= FLAG_I; } else { self.p &= !FLAG_I; } }

    #[inline]
    pub fn set_decimal(&mut self, v: bool) { if v { self.p |= FLAG_D; } else { self.p &= !FLAG_D; } }

    #[inline]
    pub fn overflow(&self) -> bool { (self.p & FLAG_V) != 0 }
    #[inline]
    pub fn set_overflow(&mut self, v: bool) { if v { self.p |= FLAG_V; } else { self.p &= !FLAG_V; } }

    #[inline]
    pub fn negative(&self) -> bool { (self.p & FLAG_N) != 0 }
    #[inline]
    pub fn set_negative(&mut self, v: bool) { if v { self.p |= FLAG_N; } else { self.p &= !FLAG_N; } }

    /// For PHP: push P with B=1 and bit5=1 (6502 convention).
    #[inline]
    pub fn for_push_brk(&self) -> u8 { self.p | FLAG_B | FLAG_U }

    /// Restore P from stack (PLP), clearing B and setting bit5 unconditionally.
    #[inline]
    pub fn restore_from_stack(&mut self, v: u8) { self.p = (v & !FLAG_B) | FLAG_U; }

    /// Serialize P to u8.
    #[inline]
    pub fn as_u8(&self) -> u8 { self.p }
}

impl CPU {
    pub fn new() -> Self {
        Self {
            a: 0,
            x: 0,
            y: 0,
            pc: 0,
            sp: 0xFD,
            p: 0x24,
            irq_inhibit_counter: 0,
            irq_allow_counter: 0,
            prev_nmi_line: false,
            nmi_latch: false,
            skip_irq: false,
            last_cycles: 0,
            ppu_ticks_consumed: 0,
        }
    }

    pub fn reset(&mut self, bus: &mut impl BusInterface) {
        let lo = bus.read(RESET_VECTOR_LO) as u16;
        let hi = bus.read(RESET_VECTOR_HI) as u16;
        self.pc = (hi << 8) | lo;
    }

    // ── NMI line interface ──

    /// Feed the NMI line state from PPU. Edge-triggered: 0→1 latches NMI.
    pub fn set_nmi_line(&mut self, state: bool) {
        if state && !self.prev_nmi_line {
            self.nmi_latch = true;
        }
        self.prev_nmi_line = state;
    }

    /// Check and consume the NMI latch. Returns true if NMI should fire now.
    pub fn poll_nmi(&mut self) -> bool {
        if self.nmi_latch {
            self.nmi_latch = false;
            return true;
        }
        false
    }

    /// Check whether IRQ should be serviced (respects I flag and delay counters).
    /// Does NOT check skip_irq — caller must check that separately.
    pub fn irq_pending(&self) -> bool {
        let i_set = (self.p & 0x04) != 0;
        if i_set && self.irq_allow_counter > 0 {
            return true; // SEI/PLP delay: allow IRQ despite I=1
        }
        if !i_set && self.irq_inhibit_counter > 0 {
            return false; // CLI/PLP delay: block IRQ despite I=0
        }
        !i_set
    }

    /// Decrement interrupt delay counters (call after each instruction).
    pub fn tick_counters(&mut self) {
        if self.irq_inhibit_counter > 0 {
            self.irq_inhibit_counter -= 1;
        }
        if self.irq_allow_counter > 0 {
            self.irq_allow_counter -= 1;
        }
    }

    /// Execute one CPU instruction. Returns consumed CPU cycles.
    /// The caller is responsible for PPU ticking and interrupt dispatch.
    pub fn step(&mut self, bus: &mut dyn BusInterface) -> u16 {
        let current_pc = self.pc;

        bus.set_cpu_pc(current_pc);

        let opcode_byte = bus.read(current_pc);
        let opcode = decode_opcode(opcode_byte)
            .unwrap_or_else(|| panic!("未知指令: {:#04X} 位於 {:#06X}", opcode_byte, current_pc));

        let next_pc = current_pc.wrapping_add(opcode.size as u16);
        self.pc = next_pc;

        // Reset per-instruction flags
        self.skip_irq = false;
        self.ppu_ticks_consumed = 0;

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
        let stolen = bus.take_immediate_stolen_cycles();
        total_cycles = total_cycles.wrapping_add(stolen);

        // Decrement interrupt delay counters after instruction completes.
        // This is called here so that unit tests (which call step() directly)
        // also get correct counter behavior.
        self.tick_counters();

        self.last_cycles = total_cycles;
        total_cycles
    }

    /// Execute NMI (non-maskable interrupt). Returns 7 for the CPU cycles consumed.
    pub fn nmi(&mut self, bus: &mut dyn BusInterface) -> u16 {
        let pc = self.pc;
        self.stack_push_u16(bus, pc);
        self.stack_push_u8(bus, (self.p & 0xEF) | 0x20);

        let lo = bus.read(NMI_VECTOR_LO) as u16;
        let hi = bus.read(NMI_VECTOR_HI) as u16;
        self.pc = (hi << 8) | lo;

        self.p |= 0x04;
        self.irq_inhibit_counter = 0;
        self.irq_allow_counter = 0;
        self.skip_irq = false;
        self.ppu_ticks_consumed = 0;

        self.last_cycles = 7;
        7
    }

    /// IRQ: maskable interrupt. Only taken if I flag allows and no inhibit delay.
    /// Caller must check skip_irq before calling.
    pub fn irq(&mut self, bus: &mut dyn BusInterface) {
        let i_blocked = (self.p & 0x04) != 0;
        if (i_blocked && self.irq_allow_counter == 0) || (!i_blocked && self.irq_inhibit_counter > 0) {
            return;
        }

        // IRQ acknowledged: clear delay counters first.
        self.irq_inhibit_counter = 0;
        self.irq_allow_counter = 0;

        let pc = self.pc;
        self.stack_push_u16(bus, pc);
        self.stack_push_u8(bus, (self.p & 0xEF) | 0x20);

        // Now set I flag — prevents further IRQs during handler.
        self.p |= 0x04;

        // NMI can preempt IRQ vectoring at the penultimate cycle (c6).
        // IRQ sequence (7 CPU cycles):
        //   c1-c3: push PC+P (3 cycles)
        //   c4-c5: internal (2 cycles)
        //   c6:    fetch vector low ← PENULTIMATE, NMI sampled here
        //   c7:    fetch vector high
        //
        // Advance PPU by 18 dots (6 CPU cycles) to reach c6.
        let nmi_fired = bus.advance_ppu_ticks(18);
        self.ppu_ticks_consumed = 18;
        if nmi_fired {
            self.set_nmi_line(true);
        }
        let nmi_taken = self.poll_nmi();

        // Read the appropriate interrupt vector.
        // Vector fetch spans c6 (low) and c7 (high).
        if nmi_taken {
            // NMI preempts: IRQ frame (B=0) is already on stack.
            // NMI only changes the vector — no extra frame is pushed.
            let lo = bus.read(NMI_VECTOR_LO) as u16;
            let hi = bus.read(NMI_VECTOR_HI) as u16;
            self.pc = (hi << 8) | lo;
        } else {
            let lo = bus.read(IRQ_VECTOR_LO) as u16;
            let hi = bus.read(IRQ_VECTOR_HI) as u16;
            self.pc = (hi << 8) | lo;
        }

        // Advance PPU for the final cycle c7 (vector high fetch, 3 dots).
        bus.advance_ppu_ticks(3);
        self.ppu_ticks_consumed += 3;
        // Total: 21 PPU dots = 7 CPU cycles.
        self.last_cycles = 7;
    }

    fn stack_push_u16(&mut self, bus: &mut dyn BusInterface, data: u16) {
        crate::cpu::stack_flags::stack_push_u16(self, bus, data);
    }

    fn stack_push_u8(&mut self, bus: &mut dyn BusInterface, data: u8) {
        crate::cpu::stack_flags::stack_push_u8(self, bus, data);
    }
}