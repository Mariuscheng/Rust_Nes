use crate::bus::CpuBus;
use crate::cpu::addressing::{get_operand_address_with_page, read_operand_rmw};
use crate::cpu::core::CPU;
use crate::cpu::decoder::AddressMode;
use crate::cpu::stack_flags::update_zero_and_negative_flags;

// ===================== Shift / Rotate =====================

pub(crate) fn op_asl(
    cpu: &mut CPU,
    bus: &mut dyn CpuBus,
    mode: AddressMode,
    current_pc: u16,
) -> u8 {
    if mode == AddressMode::Accumulator {
        cpu.set_carry((cpu.a & 0x80) != 0);
        cpu.a <<= 1;
        update_zero_and_negative_flags(cpu, cpu.a);
        0
    } else {
        let res = get_operand_address_with_page(cpu, &mode, bus, current_pc.wrapping_add(1));
        let v = read_operand_rmw(bus, &res, &mode);
        cpu.set_carry((v & 0x80) != 0);
        // RMW: write original value first (6502 dummy write)
        bus.write(res.addr, v);
        let new_v = v << 1;
        bus.write(res.addr, new_v);
        update_zero_and_negative_flags(cpu, new_v);
        // RMW instructions never incur page-cross penalty on real 6502
        0
    }
}

pub(crate) fn op_lsr(
    cpu: &mut CPU,
    bus: &mut dyn CpuBus,
    mode: AddressMode,
    current_pc: u16,
) -> u8 {
    if mode == AddressMode::Accumulator {
        cpu.set_carry((cpu.a & 0x01) != 0);
        cpu.a >>= 1;
        update_zero_and_negative_flags(cpu, cpu.a);
        0
    } else {
        let res = get_operand_address_with_page(cpu, &mode, bus, current_pc.wrapping_add(1));
        let value = read_operand_rmw(bus, &res, &mode);
        cpu.set_carry((value & 0x01) != 0);
        // RMW: write original value first (6502 dummy write)
        bus.write(res.addr, value);
        let new_value = value >> 1;
        bus.write(res.addr, new_value);
        update_zero_and_negative_flags(cpu, new_value);
        // RMW instructions never incur page-cross penalty on real 6502
        0
    }
}

pub(crate) fn op_rol(
    cpu: &mut CPU,
    bus: &mut dyn CpuBus,
    mode: AddressMode,
    current_pc: u16,
) -> u8 {
    let old_carry = cpu.carry();
    if mode == AddressMode::Accumulator {
        cpu.set_carry((cpu.a & 0x80) != 0);
        cpu.a = (cpu.a << 1) | (if old_carry { 1 } else { 0 });
        update_zero_and_negative_flags(cpu, cpu.a);
        0
    } else {
        let res = get_operand_address_with_page(cpu, &mode, bus, current_pc.wrapping_add(1));
        let value = read_operand_rmw(bus, &res, &mode);
        cpu.set_carry((value & 0x80) != 0);
        // RMW: write original value first (6502 dummy write)
        bus.write(res.addr, value);
        let new_value = (value << 1) | (if old_carry { 1 } else { 0 });
        bus.write(res.addr, new_value);
        update_zero_and_negative_flags(cpu, new_value);
        // RMW instructions never incur page-cross penalty on real 6502
        0
    }
}

pub(crate) fn op_ror(
    cpu: &mut CPU,
    bus: &mut dyn CpuBus,
    mode: AddressMode,
    current_pc: u16,
) -> u8 {
    let old_carry = cpu.carry();
    if mode == AddressMode::Accumulator {
        let new_carry = (cpu.a & 0x01) != 0;
        cpu.a >>= 1;
        if old_carry {
            cpu.a |= 0x80;
        }
        cpu.set_carry(new_carry);
        update_zero_and_negative_flags(cpu, cpu.a);
        0
    } else {
        let res = get_operand_address_with_page(cpu, &mode, bus, current_pc.wrapping_add(1));
        let value = read_operand_rmw(bus, &res, &mode);
        let new_carry = (value & 0x01) != 0;
        // RMW: write original value first (6502 dummy write)
        bus.write(res.addr, value);
        let mut new_value = value >> 1;
        if old_carry {
            new_value |= 0x80;
        }
        cpu.set_carry(new_carry);
        bus.write(res.addr, new_value);
        update_zero_and_negative_flags(cpu, new_value);
        // RMW instructions never incur page-cross penalty on real 6502
        0
    }
}