//! Instruction handlers, grouped by operation family.
//!
//! Every module exports `pub(crate)` functions with the `op_*` naming scheme;
//! `CPU::step` dispatches to them after decoding. Handlers perform their bus
//! accesses in hardware order and return any additional cycles they needed.

// Load / Store / Register transfers / Increments / Decrements
mod load_store;
pub(crate) use load_store::*;

// ALU operations (ORA, AND, EOR, ADC, SBC) + Compare (CMP, CPX, CPY)
mod alu;
pub(crate) use alu::*;

// Shift / Rotate operations (ASL, LSR, ROL, ROR)
mod shift_rotate;
pub(crate) use shift_rotate::*;

// Memory operations (BIT, DEC, INC) + NOP
mod memory;
pub(crate) use memory::*;

// Stack push / pop (PHA, PLA, PHP, PLP)
mod stack;
pub(crate) use stack::*;

// Status flag setters / clearers (SEI, SEC, CLC, CLD, CLI, CLV, SED, TXS, TSX)
mod flag;
pub(crate) use flag::*;

// Branch / Jump / Subroutine / Interrupt (JSR, RTS, BRK, RTI, JMP, branch helpers)
mod branch_jump;
pub(crate) use branch_jump::*;

// Unofficial / illegal opcodes (SLO, RLA, SRE, RRA, SAX, LAX, DCP, ISC, ANC, ALR, ARR, AXS, KIL)
mod unofficial;
pub(crate) use unofficial::*;
