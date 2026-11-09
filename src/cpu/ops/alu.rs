use crate::bus::BusInterface;
use crate::cpu::addressing::{get_operand_address_with_page, read_operand};
use crate::cpu::core::CPU;
use crate::cpu::decoder::AddressMode;
use crate::cpu::stack_flags::update_zero_and_negative_flags;

// ===================== ALU Operations =====================

pub(crate) fn op_ora(
    cpu: &mut CPU,
    bus: &mut dyn BusInterface,
    mode: AddressMode,
    current_pc: u16,
) -> u8 {
    let res = get_operand_address_with_page(cpu, &mode, bus, current_pc.wrapping_add(1));
    let value = read_operand(bus, &res);
    cpu.a |= value;
    update_zero_and_negative_flags(cpu, cpu.a);
    if res.page_crossed { 1 } else { 0 }
}

pub(crate) fn op_and(
    cpu: &mut CPU,
    bus: &mut dyn BusInterface,
    mode: AddressMode,
    current_pc: u16,
) -> u8 {
    let res = get_operand_address_with_page(cpu, &mode, bus, current_pc.wrapping_add(1));
    let value = read_operand(bus, &res);
    cpu.a &= value;
    update_zero_and_negative_flags(cpu, cpu.a);
    if res.page_crossed { 1 } else { 0 }
}

pub(crate) fn op_eor(
    cpu: &mut CPU,
    bus: &mut dyn BusInterface,
    mode: AddressMode,
    current_pc: u16,
) -> u8 {
    let res = get_operand_address_with_page(cpu, &mode, bus, current_pc.wrapping_add(1));
    let value = read_operand(bus, &res);
    cpu.a ^= value;
    update_zero_and_negative_flags(cpu, cpu.a);
    if res.page_crossed { 1 } else { 0 }
}

pub(crate) fn op_adc(
    cpu: &mut CPU,
    bus: &mut dyn BusInterface,
    mode: AddressMode,
    current_pc: u16,
) -> u8 {
    let res = get_operand_address_with_page(cpu, &mode, bus, current_pc.wrapping_add(1));
    let m = read_operand(bus, &res);
    let carry_in = if cpu.carry() { 1u16 } else { 0u16 };

    // NES 2A03 does not support BCD mode; always use binary arithmetic
    let sum = cpu.a as u16 + m as u16 + carry_in;
    let result = sum as u8;

    cpu.set_carry(sum > 0xFF);
    cpu.set_overflow(((!(cpu.a ^ m)) & (cpu.a ^ result) & 0x80) != 0);
    cpu.a = result;
    update_zero_and_negative_flags(cpu, cpu.a);

    if res.page_crossed { 1 } else { 0 }
}

pub(crate) fn op_sbc(
    cpu: &mut CPU,
    bus: &mut dyn BusInterface,
    mode: AddressMode,
    current_pc: u16,
) -> u8 {
    let res = get_operand_address_with_page(cpu, &mode, bus, current_pc.wrapping_add(1));
    let m = read_operand(bus, &res);
    let carry_in = if cpu.carry() { 1u16 } else { 0u16 };

    // NES 2A03 does not support BCD mode; always use binary arithmetic
    let sum = cpu.a as u16 + (!m as u16) + carry_in;
    let result = sum as u8;

    cpu.set_carry(sum > 0xFF);
    // Overflow for SBC uses A + (~M) + C; detect signed overflow
    let m_comp = !m;
    cpu.set_overflow(((cpu.a ^ result) & (m_comp ^ result) & 0x80) != 0);
    cpu.a = result;
    update_zero_and_negative_flags(cpu, cpu.a);

    if res.page_crossed { 1 } else { 0 }
}

// ===================== Compare Operations =====================

pub(crate) fn op_cmp(
    cpu: &mut CPU,
    bus: &mut dyn BusInterface,
    mode: AddressMode,
    current_pc: u16,
) -> u8 {
    let res = get_operand_address_with_page(cpu, &mode, bus, current_pc.wrapping_add(1));
    let value = read_operand(bus, &res);

    // Compute result first, then set Zero/Negative, then set Carry
    let result = cpu.a.wrapping_sub(value);
    update_zero_and_negative_flags(cpu, result);

    cpu.set_carry(cpu.a >= value);

    if res.page_crossed { 1 } else { 0 }
}

pub(crate) fn op_cpx(
    cpu: &mut CPU,
    bus: &mut dyn BusInterface,
    mode: AddressMode,
    current_pc: u16,
) -> u8 {
    let res = get_operand_address_with_page(cpu, &mode, bus, current_pc.wrapping_add(1));
    let value = read_operand(bus, &res);
    cpu.set_carry(cpu.x >= value);
    let result = cpu.x.wrapping_sub(value);
    update_zero_and_negative_flags(cpu, result);
    if res.page_crossed { 1 } else { 0 }
}

pub(crate) fn op_cpy(
    cpu: &mut CPU,
    bus: &mut dyn BusInterface,
    mode: AddressMode,
    current_pc: u16,
) -> u8 {
    let res = get_operand_address_with_page(cpu, &mode, bus, current_pc.wrapping_add(1));
    let value = read_operand(bus, &res);
    cpu.set_carry(cpu.y >= value);
    let result = cpu.y.wrapping_sub(value);
    update_zero_and_negative_flags(cpu, result);
    if res.page_crossed { 1 } else { 0 }
}