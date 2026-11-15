use crate::bus::CpuBus;
use crate::cpu::addressing::{get_operand_address_with_page, read_operand, read_operand_rmw};
use crate::cpu::core::CPU;
use crate::cpu::decoder::AddressMode;
use crate::cpu::stack_flags::update_zero_and_negative_flags;

// ===================== Memory Operations (BIT, DEC, INC) =====================

pub(crate) fn op_bit(
    cpu: &mut CPU,
    bus: &mut dyn CpuBus,
    mode: AddressMode,
    current_pc: u16,
) -> u8 {
    let res = get_operand_address_with_page(cpu, &mode, bus, current_pc.wrapping_add(1));
    let value = read_operand(bus, &res);
    cpu.set_zero((value & cpu.a) == 0);
    cpu.set_negative((value & 0x80) != 0);
    cpu.set_overflow((value & 0x40) != 0);
    if res.page_crossed { 1 } else { 0 }
}

pub(crate) fn op_dec(
    cpu: &mut CPU,
    bus: &mut dyn CpuBus,
    mode: AddressMode,
    current_pc: u16,
) -> u8 {
    let res = get_operand_address_with_page(cpu, &mode, bus, current_pc.wrapping_add(1));
    let original = read_operand_rmw(bus, &res, &mode);
    let dec_value = original.wrapping_sub(1);
    // RMW: write original value first (6502 dummy write)
    bus.write(res.addr, original);
    bus.write(res.addr, dec_value);
    update_zero_and_negative_flags(cpu, dec_value);
    // RMW instructions never incur page-cross penalty on real 6502
    0
}

pub(crate) fn op_inc(
    cpu: &mut CPU,
    bus: &mut dyn CpuBus,
    mode: AddressMode,
    current_pc: u16,
) -> u8 {
    let res = get_operand_address_with_page(cpu, &mode, bus, current_pc.wrapping_add(1));
    let original = read_operand_rmw(bus, &res, &mode);
    let inc_value = original.wrapping_add(1);
    // RMW: write original value first (6502 dummy write)
    bus.write(res.addr, original);
    bus.write(res.addr, inc_value);
    update_zero_and_negative_flags(cpu, inc_value);
    // RMW instructions never incur page-cross penalty on real 6502
    0
}

// ===================== NOP variants =====================

pub(crate) fn op_nop(
    cpu: &mut CPU,
    bus: &mut dyn CpuBus,
    mode: AddressMode,
    current_pc: u16,
) -> u8 {
    match mode {
        AddressMode::Implied => 0,
        AddressMode::Immediate => {
            // Dummy read of the immediate operand byte (the byte after the opcode)
            bus.read(current_pc.wrapping_add(1));
            0
        }
        _ => {
            // All memory-addressing modes (ZeroPage, ZeroPageX, Absolute, AbsoluteX,
            // AbsoluteY, IndirectY): compute effective address and perform dummy read.
            let res = get_operand_address_with_page(cpu, &mode, bus, current_pc.wrapping_add(1));
            // Dummy read from the effective address — the value is discarded.
            // Use read_operand to also perform page-cross dummy read.
            read_operand(bus, &res);
            if res.page_crossed { 1 } else { 0 }
        }
    }
}
