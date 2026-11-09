use std::fmt;

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Instruction {
    // Official
    ADC,
    AND,
    ASL,
    BCC,
    BCS,
    BEQ,
    BIT,
    BMI,
    BNE,
    BPL,
    BRK,
    BVC,
    BVS,
    CLC,
    CLD,
    CLI,
    CLV,
    CMP,
    CPX,
    CPY,
    DEC,
    DEX,
    DEY,
    EOR,
    INC,
    INX,
    INY,
    JMP,
    JSR,
    LDA,
    LDX,
    LDY,
    LSR,
    NOP,
    ORA,
    PHA,
    PHP,
    PLA,
    PLP,
    ROL,
    ROR,
    RTI,
    RTS,
    SBC,
    SEC,
    SED,
    SEI,
    STA,
    STX,
    STY,
    TAX,
    TAY,
    TSX,
    TXA,
    TXS,
    TYA,
    // Unofficial
    SLO, // ASL + ORA
    RLA, // ROL + AND
    SRE, // LSR + EOR
    RRA, // ROR + ADC
    SAX, // STA & STX (store A & X)
    LAX, // LDA & LDX (load A and X)
    DCP, // DEC + CMP
    ISC, // INC + SBC
    ANC, // AND + carry to bit 7
    ALR, // AND + LSR
    ARR, // AND + ROR
    AXS, // A & X -> X, then subtract imm
    ANX, // A & X & imm -> A, X (also known as ATX/XAA)
    LAS, // LDA abs,Y + TAX + TXS (opcode 0xBB)
    SYA, // Store Y & (addr+1 >> 8) at abs,X
    SXA, // Store X & (addr+1 >> 8) at abs,Y
    AHX, // Store A & X & (high byte + 1) — unstable, never takes page-cross penalty
    TAS, // A = X = SP = A & X, then unstable store of A & (high byte + 1)
    KIL, // 2-byte NOP on NES 2A03 (reads and discards operand byte)
}

impl fmt::Display for Instruction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum AddressMode {
    Implied,
    Immediate,
    ZeroPage,
    ZeroPageX,
    ZeroPageY,
    Absolute,
    AbsoluteX,
    AbsoluteY,
    IndirectX,
    Indirect,
    IndirectY,
    Relative,
    Accumulator,
}

#[derive(Copy, Clone, Debug)]
pub struct Opcode {
    pub op: Instruction,
    pub mode: AddressMode,
    pub cycles: u8,
    pub size: u8,
}

/// Complete 6502 opcode table (NES 2A03).
/// Source: https://www.nesdev.org/obelisk-6502-guidens/reference.html
/// and https://www.nesdev.org/wiki/CPU_unofficial_opcodes
const OPCODE_TABLE: [Option<Opcode>; 256] = {
    use crate::cpu::decoder::AddressMode::*;
    use crate::cpu::decoder::Instruction::*;

    let mut t: [Option<Opcode>; 256] = [const { None }; 256];

    // Helper macro to reduce repetition
    macro_rules! op {
        ($byte:expr, $op:ident, $mode:ident, $cycles:expr, $size:expr) => {
            t[$byte] = Some(Opcode {
                op: $op,
                mode: $mode,
                cycles: $cycles,
                size: $size,
            });
        };
    }

    // --- 0x00 - 0x0F ---
    op!(0x00, BRK, Implied, 7, 1);
    op!(0x01, ORA, IndirectX, 6, 2);
    op!(0x02, KIL, Immediate, 2, 2); // 2-byte NOP on 2A03
    op!(0x03, SLO, IndirectX, 8, 2);
    op!(0x04, NOP, ZeroPage, 3, 2); // d,x NOP, reads zp
    op!(0x05, ORA, ZeroPage, 3, 2);
    op!(0x06, ASL, ZeroPage, 5, 2);
    op!(0x07, SLO, ZeroPage, 5, 2);
    op!(0x08, PHP, Implied, 3, 1);
    op!(0x09, ORA, Immediate, 2, 2);
    op!(0x0A, ASL, Accumulator, 2, 1);
    op!(0x0B, ANC, Immediate, 2, 2);
    op!(0x0C, NOP, Absolute, 4, 3); // d,abs NOP
    op!(0x0D, ORA, Absolute, 4, 3);
    op!(0x0E, ASL, Absolute, 6, 3);
    op!(0x0F, SLO, Absolute, 6, 3);

    // --- 0x10 - 0x1F ---
    op!(0x10, BPL, Relative, 2, 2);
    op!(0x11, ORA, IndirectY, 5, 2);
    op!(0x12, KIL, Immediate, 2, 2); // 2-byte NOP on 2A03
    op!(0x13, SLO, IndirectY, 8, 2);
    op!(0x14, NOP, ZeroPageX, 4, 2); // d,x NOP
    op!(0x15, ORA, ZeroPageX, 4, 2);
    op!(0x16, ASL, ZeroPageX, 6, 2);
    op!(0x17, SLO, ZeroPageX, 6, 2);
    op!(0x18, CLC, Implied, 2, 1);
    op!(0x19, ORA, AbsoluteY, 4, 3);
    op!(0x1A, NOP, Implied, 2, 1); // unofficial NOP
    op!(0x1B, SLO, AbsoluteY, 7, 3);
    op!(0x1C, NOP, AbsoluteX, 4, 3); // d,x NOP (was wrongly ORA)
    op!(0x1D, ORA, AbsoluteX, 4, 3);
    op!(0x1E, ASL, AbsoluteX, 7, 3);
    op!(0x1F, SLO, AbsoluteX, 7, 3);

    // --- 0x20 - 0x2F ---
    op!(0x20, JSR, Absolute, 6, 3);
    op!(0x21, AND, IndirectX, 6, 2);
    op!(0x22, KIL, Immediate, 2, 2); // 2-byte NOP on 2A03
    op!(0x23, RLA, IndirectX, 8, 2);
    op!(0x24, BIT, ZeroPage, 3, 2);
    op!(0x25, AND, ZeroPage, 3, 2);
    op!(0x26, ROL, ZeroPage, 5, 2);
    op!(0x27, RLA, ZeroPage, 5, 2);
    op!(0x28, PLP, Implied, 4, 1);
    op!(0x29, AND, Immediate, 2, 2);
    op!(0x2A, ROL, Accumulator, 2, 1);
    op!(0x2B, ANC, Immediate, 2, 2);
    op!(0x2C, BIT, Absolute, 4, 3);
    op!(0x2D, AND, Absolute, 4, 3);
    op!(0x2E, ROL, Absolute, 6, 3);
    op!(0x2F, RLA, Absolute, 6, 3);

    // --- 0x30 - 0x3F ---
    op!(0x30, BMI, Relative, 2, 2);
    op!(0x31, AND, IndirectY, 5, 2);
    op!(0x32, KIL, Immediate, 2, 2); // 2-byte NOP on 2A03
    op!(0x33, RLA, IndirectY, 8, 2);
    op!(0x34, NOP, ZeroPageX, 4, 2); // d,x NOP
    op!(0x35, AND, ZeroPageX, 4, 2);
    op!(0x36, ROL, ZeroPageX, 6, 2);
    op!(0x37, RLA, ZeroPageX, 6, 2);
    op!(0x38, SEC, Implied, 2, 1);
    op!(0x39, AND, AbsoluteY, 4, 3);
    op!(0x3A, NOP, Implied, 2, 1); // unofficial NOP
    op!(0x3B, RLA, AbsoluteY, 7, 3);
    op!(0x3C, NOP, AbsoluteX, 4, 3); // d,x NOP
    op!(0x3D, AND, AbsoluteX, 4, 3);
    op!(0x3E, ROL, AbsoluteX, 7, 3);
    op!(0x3F, RLA, AbsoluteX, 7, 3);

    // --- 0x40 - 0x4F ---
    op!(0x40, RTI, Implied, 6, 1);
    op!(0x41, EOR, IndirectX, 6, 2);
    op!(0x42, KIL, Immediate, 2, 2); // 2-byte NOP on 2A03
    op!(0x43, SRE, IndirectX, 8, 2);
    op!(0x44, NOP, ZeroPage, 3, 2); // d,zp NOP
    op!(0x45, EOR, ZeroPage, 3, 2);
    op!(0x46, LSR, ZeroPage, 5, 2);
    op!(0x47, SRE, ZeroPage, 5, 2);
    op!(0x48, PHA, Implied, 3, 1);
    op!(0x49, EOR, Immediate, 2, 2);
    op!(0x4A, LSR, Accumulator, 2, 1);
    op!(0x4B, ALR, Immediate, 2, 2);
    op!(0x4C, JMP, Absolute, 3, 3);
    op!(0x4D, EOR, Absolute, 4, 3);
    op!(0x4E, LSR, Absolute, 6, 3);
    op!(0x4F, SRE, Absolute, 6, 3);

    // --- 0x50 - 0x5F ---
    op!(0x50, BVC, Relative, 2, 2);
    op!(0x51, EOR, IndirectY, 5, 2);
    op!(0x52, KIL, Immediate, 2, 2); // 2-byte NOP on 2A03
    op!(0x53, SRE, IndirectY, 8, 2);
    op!(0x54, NOP, ZeroPageX, 4, 2); // d,x NOP
    op!(0x55, EOR, ZeroPageX, 4, 2);
    op!(0x56, LSR, ZeroPageX, 6, 2);
    op!(0x57, SRE, ZeroPageX, 6, 2);
    op!(0x58, CLI, Implied, 2, 1); // official CLI
    op!(0x59, EOR, AbsoluteY, 4, 3);
    op!(0x5A, NOP, Implied, 2, 1); // unofficial NOP
    op!(0x5B, SRE, AbsoluteY, 7, 3);
    op!(0x5C, NOP, AbsoluteX, 4, 3); // d,x NOP
    op!(0x5D, EOR, AbsoluteX, 4, 3);
    op!(0x5E, LSR, AbsoluteX, 7, 3);
    op!(0x5F, SRE, AbsoluteX, 7, 3);

    // --- 0x60 - 0x6F ---
    op!(0x60, RTS, Implied, 6, 1);
    op!(0x61, ADC, IndirectX, 6, 2);
    op!(0x62, KIL, Immediate, 2, 2); // 2-byte NOP on 2A03
    op!(0x63, RRA, IndirectX, 8, 2);
    op!(0x64, NOP, ZeroPage, 3, 2); // d,zp NOP
    op!(0x65, ADC, ZeroPage, 3, 2);
    op!(0x66, ROR, ZeroPage, 5, 2);
    op!(0x67, RRA, ZeroPage, 5, 2);
    op!(0x68, PLA, Implied, 4, 1);
    op!(0x69, ADC, Immediate, 2, 2);
    op!(0x6A, ROR, Accumulator, 2, 1);
    op!(0x6B, ARR, Immediate, 2, 2);
    op!(0x6C, JMP, Indirect, 5, 3);
    op!(0x6D, ADC, Absolute, 4, 3);
    op!(0x6E, ROR, Absolute, 6, 3);
    op!(0x6F, RRA, Absolute, 6, 3);

    // --- 0x70 - 0x7F ---
    op!(0x70, BVS, Relative, 2, 2);
    op!(0x71, ADC, IndirectY, 5, 2);
    op!(0x72, KIL, Immediate, 2, 2); // 2-byte NOP on 2A03
    op!(0x73, RRA, IndirectY, 8, 2);
    op!(0x74, NOP, ZeroPageX, 4, 2); // d,x NOP
    op!(0x75, ADC, ZeroPageX, 4, 2);
    op!(0x76, ROR, ZeroPageX, 6, 2);
    op!(0x77, RRA, ZeroPageX, 6, 2);
    op!(0x78, SEI, Implied, 2, 1);
    op!(0x79, ADC, AbsoluteY, 4, 3);
    op!(0x7A, NOP, Implied, 2, 1); // unofficial NOP
    op!(0x7B, RRA, AbsoluteY, 7, 3);
    op!(0x7C, NOP, AbsoluteX, 4, 3); // d,x NOP
    op!(0x7D, ADC, AbsoluteX, 4, 3);
    op!(0x7E, ROR, AbsoluteX, 7, 3);
    op!(0x7F, RRA, AbsoluteX, 7, 3);

    // --- 0x80 - 0x8F ---
    op!(0x80, NOP, Immediate, 2, 2); // d,imm NOP
    op!(0x81, STA, IndirectX, 6, 2);
    op!(0x82, NOP, Immediate, 2, 2); // d,imm NOP
    op!(0x83, SAX, IndirectX, 6, 2);
    op!(0x84, STY, ZeroPage, 3, 2);
    op!(0x85, STA, ZeroPage, 3, 2);
    op!(0x86, STX, ZeroPage, 3, 2);
    op!(0x87, SAX, ZeroPage, 3, 2);
    op!(0x88, DEY, Implied, 2, 1);
    op!(0x89, NOP, Immediate, 2, 2); // d,imm NOP
    op!(0x8A, TXA, Implied, 2, 1);
    op!(0x8B, NOP, Immediate, 2, 2); // XAA (unstable, treat as NOP)
    op!(0x8C, STY, Absolute, 4, 3);
    op!(0x8D, STA, Absolute, 4, 3);
    op!(0x8E, STX, Absolute, 4, 3);
    op!(0x8F, SAX, Absolute, 4, 3);

    // --- 0x90 - 0x9F ---
    op!(0x90, BCC, Relative, 2, 2);
    op!(0x91, STA, IndirectY, 6, 2);
    op!(0x92, KIL, Immediate, 2, 2); // 2-byte NOP on 2A03
    op!(0x93, AHX, IndirectY, 6, 2); // AHX (unstable, fixed 6 cycles — no page-cross penalty)
    op!(0x94, STY, ZeroPageX, 4, 2);
    op!(0x95, STA, ZeroPageX, 4, 2);
    op!(0x96, STX, ZeroPageY, 4, 2);
    op!(0x97, SAX, ZeroPageY, 4, 2);
    op!(0x98, TYA, Implied, 2, 1);
    op!(0x99, STA, AbsoluteY, 5, 3);
    op!(0x9A, TXS, Implied, 2, 1);
    op!(0x9B, TAS, AbsoluteY, 5, 3); // TAS (unstable, fixed 5 cycles — no page-cross penalty)
    op!(0x9C, SYA, AbsoluteX, 5, 3);
    op!(0x9D, STA, AbsoluteX, 5, 3);
    op!(0x9E, SXA, AbsoluteY, 5, 3);
    op!(0x9F, AHX, AbsoluteY, 5, 3); // AHX (unstable, fixed 5 cycles — no page-cross penalty)

    // --- 0xA0 - 0xAF ---
    op!(0xA0, LDY, Immediate, 2, 2);
    op!(0xA1, LDA, IndirectX, 6, 2);
    op!(0xA2, LDX, Immediate, 2, 2);
    op!(0xA3, LAX, IndirectX, 6, 2);
    op!(0xA4, LDY, ZeroPage, 3, 2);
    op!(0xA5, LDA, ZeroPage, 3, 2);
    op!(0xA6, LDX, ZeroPage, 3, 2);
    op!(0xA7, LAX, ZeroPage, 3, 2);
    op!(0xA8, TAY, Implied, 2, 1);
    op!(0xA9, LDA, Immediate, 2, 2);
    op!(0xAA, TAX, Implied, 2, 1);
    op!(0xAB, ANX, Immediate, 2, 2);
    op!(0xAC, LDY, Absolute, 4, 3);
    op!(0xAD, LDA, Absolute, 4, 3);
    op!(0xAE, LDX, Absolute, 4, 3);
    op!(0xAF, LAX, Absolute, 4, 3);

    // --- 0xB0 - 0xBF ---
    op!(0xB0, BCS, Relative, 2, 2);
    op!(0xB1, LDA, IndirectY, 5, 2);
    op!(0xB2, KIL, Immediate, 2, 2); // 2-byte NOP on 2A03
    op!(0xB3, LAX, IndirectY, 5, 2);
    op!(0xB4, LDY, ZeroPageX, 4, 2);
    op!(0xB5, LDA, ZeroPageX, 4, 2);
    op!(0xB6, LDX, ZeroPageY, 4, 2);
    op!(0xB7, LAX, ZeroPageY, 4, 2);
    op!(0xB8, CLV, Implied, 2, 1);
    op!(0xB9, LDA, AbsoluteY, 4, 3);
    op!(0xBA, TSX, Implied, 2, 1);
    op!(0xBB, LAS, AbsoluteY, 4, 3); // LAS (LDA abs,Y + TAX + TXS)
    op!(0xBC, LDY, AbsoluteX, 4, 3);
    op!(0xBD, LDA, AbsoluteX, 4, 3);
    op!(0xBE, LDX, AbsoluteY, 4, 3);
    op!(0xBF, LAX, AbsoluteY, 4, 3);

    // --- 0xC0 - 0xCF ---
    op!(0xC0, CPY, Immediate, 2, 2);
    op!(0xC1, CMP, IndirectX, 6, 2);
    op!(0xC2, NOP, Immediate, 2, 2); // d,imm NOP
    op!(0xC3, DCP, IndirectX, 8, 2);
    op!(0xC4, CPY, ZeroPage, 3, 2);
    op!(0xC5, CMP, ZeroPage, 3, 2);
    op!(0xC6, DEC, ZeroPage, 5, 2);
    op!(0xC7, DCP, ZeroPage, 5, 2);
    op!(0xC8, INY, Implied, 2, 1);
    op!(0xC9, CMP, Immediate, 2, 2);
    op!(0xCA, DEX, Implied, 2, 1);
    op!(0xCB, AXS, Immediate, 2, 2);
    op!(0xCC, CPY, Absolute, 4, 3);
    op!(0xCD, CMP, Absolute, 4, 3);
    op!(0xCE, DEC, Absolute, 6, 3);
    op!(0xCF, DCP, Absolute, 6, 3);

    // --- 0xD0 - 0xDF ---
    op!(0xD0, BNE, Relative, 2, 2);
    op!(0xD1, CMP, IndirectY, 5, 2);
    op!(0xD2, KIL, Immediate, 2, 2); // 2-byte NOP on 2A03
    op!(0xD3, DCP, IndirectY, 8, 2);
    op!(0xD4, NOP, ZeroPageX, 4, 2); // d,x NOP
    op!(0xD5, CMP, ZeroPageX, 4, 2);
    op!(0xD6, DEC, ZeroPageX, 6, 2);
    op!(0xD7, DCP, ZeroPageX, 6, 2);
    op!(0xD8, CLD, Implied, 2, 1);
    op!(0xD9, CMP, AbsoluteY, 4, 3);
    op!(0xDA, NOP, Implied, 2, 1); // unofficial NOP
    op!(0xDB, DCP, AbsoluteY, 7, 3);
    op!(0xDC, NOP, AbsoluteX, 4, 3); // d,x NOP
    op!(0xDD, CMP, AbsoluteX, 4, 3);
    op!(0xDE, DEC, AbsoluteX, 7, 3);
    op!(0xDF, DCP, AbsoluteX, 7, 3);

    // --- 0xE0 - 0xEF ---
    op!(0xE0, CPX, Immediate, 2, 2);
    op!(0xE1, SBC, IndirectX, 6, 2);
    op!(0xE2, NOP, Immediate, 2, 2); // d,imm NOP
    op!(0xE3, ISC, IndirectX, 8, 2);
    op!(0xE4, CPX, ZeroPage, 3, 2);
    op!(0xE5, SBC, ZeroPage, 3, 2);
    op!(0xE6, INC, ZeroPage, 5, 2);
    op!(0xE7, ISC, ZeroPage, 5, 2);
    op!(0xE8, INX, Implied, 2, 1);
    op!(0xE9, SBC, Immediate, 2, 2);
    op!(0xEA, NOP, Implied, 2, 1);
    op!(0xEB, SBC, Immediate, 2, 2); // unofficial NOP (same as E9 on NES)
    op!(0xEC, CPX, Absolute, 4, 3);
    op!(0xED, SBC, Absolute, 4, 3);
    op!(0xEE, INC, Absolute, 6, 3);
    op!(0xEF, ISC, Absolute, 6, 3);

    // --- 0xF0 - 0xFF ---
    op!(0xF0, BEQ, Relative, 2, 2);
    op!(0xF1, SBC, IndirectY, 5, 2);
    op!(0xF2, KIL, Immediate, 2, 2); // 2-byte NOP on 2A03
    op!(0xF3, ISC, IndirectY, 8, 2);
    op!(0xF4, NOP, ZeroPageX, 4, 2); // d,x NOP
    op!(0xF5, SBC, ZeroPageX, 4, 2);
    op!(0xF6, INC, ZeroPageX, 6, 2);
    op!(0xF7, ISC, ZeroPageX, 6, 2);
    op!(0xF8, SED, Implied, 2, 1);
    op!(0xF9, SBC, AbsoluteY, 4, 3);
    op!(0xFA, NOP, Implied, 2, 1); // unofficial NOP
    op!(0xFB, ISC, AbsoluteY, 7, 3);
    op!(0xFC, NOP, AbsoluteX, 4, 3); // d,x NOP (was wrongly SBC)
    op!(0xFD, SBC, AbsoluteX, 4, 3);
    op!(0xFE, INC, AbsoluteX, 7, 3);
    op!(0xFF, ISC, AbsoluteX, 7, 3);

    t
};

/// Decode a byte into an Opcode. Returns `Some` for all 256 bytes.
pub fn decode_opcode(byte: u8) -> Option<Opcode> {
    OPCODE_TABLE[byte as usize]
}