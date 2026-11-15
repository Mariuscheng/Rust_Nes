use crate::bus::CpuBus;
use crate::cpu::addressing::{get_operand_address_with_page, read_operand};
use crate::cpu::core::CPU;
use crate::cpu::decoder::AddressMode;
use crate::cpu::stack_flags::update_zero_and_negative_flags;

// ===================== Load / Store / Register Transfers =====================

pub(crate) fn op_ldx(
    cpu: &mut CPU,
    bus: &mut dyn CpuBus,
    mode: AddressMode,
    current_pc: u16,
) -> u8 {
    let res = get_operand_address_with_page(cpu, &mode, bus, current_pc.wrapping_add(1));
    let value = read_operand(bus, &res);
    cpu.x = value;
    update_zero_and_negative_flags(cpu, cpu.x);
    if res.page_crossed { 1 } else { 0 }
}

pub(crate) fn op_lda(
    cpu: &mut CPU,
    bus: &mut dyn CpuBus,
    mode: AddressMode,
    current_pc: u16,
) -> u8 {
    let res = get_operand_address_with_page(cpu, &mode, bus, current_pc.wrapping_add(1));
    cpu.a = read_operand(bus, &res);
    update_zero_and_negative_flags(cpu, cpu.a);
    if res.page_crossed { 1 } else { 0 }
}

pub(crate) fn op_ldy(
    cpu: &mut CPU,
    bus: &mut dyn CpuBus,
    mode: AddressMode,
    current_pc: u16,
) -> u8 {
    let res = get_operand_address_with_page(cpu, &mode, bus, current_pc.wrapping_add(1));
    cpu.y = read_operand(bus, &res);
    update_zero_and_negative_flags(cpu, cpu.y);
    if res.page_crossed { 1 } else { 0 }
}

pub(crate) fn op_sta(cpu: &mut CPU, bus: &mut dyn CpuBus, mode: AddressMode, current_pc: u16) {
    let res = get_operand_address_with_page(cpu, &mode, bus, current_pc.wrapping_add(1));
    // Indexed addressing modes perform a dummy read before the write.
    // IndirectX already performs its dummy read at (zp+X) during address
    // calculation; no additional dummy read at the final effective address.
    if !matches!(
        mode,
        AddressMode::ZeroPage
            | AddressMode::Absolute
            | AddressMode::IndirectX
            | AddressMode::ZeroPageX
            | AddressMode::ZeroPageY
    ) {
        let dummy_addr = res.dummy_addr.unwrap_or(res.addr);
        bus.read(dummy_addr);
    }
    bus.write(res.addr, cpu.a);
}

pub(crate) fn op_stx(cpu: &mut CPU, bus: &mut dyn CpuBus, mode: AddressMode, current_pc: u16) {
    let res = get_operand_address_with_page(cpu, &mode, bus, current_pc.wrapping_add(1));
    // Indexed addressing modes (ZeroPageY) perform a dummy read before the write.
    if !matches!(
        mode,
        AddressMode::ZeroPage
            | AddressMode::Absolute
            | AddressMode::ZeroPageX
            | AddressMode::ZeroPageY
    ) {
        let dummy_addr = res.dummy_addr.unwrap_or(res.addr);
        bus.read(dummy_addr);
    }
    bus.write(res.addr, cpu.x);
}

pub(crate) fn op_sty(cpu: &mut CPU, bus: &mut dyn CpuBus, mode: AddressMode, current_pc: u16) {
    let res = get_operand_address_with_page(cpu, &mode, bus, current_pc.wrapping_add(1));
    // Indexed addressing modes (ZeroPageX) perform a dummy read before the write.
    if !matches!(
        mode,
        AddressMode::ZeroPage
            | AddressMode::Absolute
            | AddressMode::ZeroPageX
            | AddressMode::ZeroPageY
    ) {
        let dummy_addr = res.dummy_addr.unwrap_or(res.addr);
        bus.read(dummy_addr);
    }
    bus.write(res.addr, cpu.y);
}

// ===================== Register Increments / Decrements =====================

pub(crate) fn op_inx(cpu: &mut CPU, _bus: &mut dyn CpuBus, _mode: AddressMode, _current_pc: u16) {
    cpu.x = cpu.x.wrapping_add(1);
    update_zero_and_negative_flags(cpu, cpu.x);
}

pub(crate) fn op_iny(cpu: &mut CPU, _bus: &mut dyn CpuBus, _mode: AddressMode, _current_pc: u16) {
    cpu.y = cpu.y.wrapping_add(1);
    update_zero_and_negative_flags(cpu, cpu.y);
}

pub(crate) fn op_dex(cpu: &mut CPU, _bus: &mut dyn CpuBus, _mode: AddressMode, _current_pc: u16) {
    cpu.x = cpu.x.wrapping_sub(1);
    update_zero_and_negative_flags(cpu, cpu.x);
}

pub(crate) fn op_dey(cpu: &mut CPU, _bus: &mut dyn CpuBus, _mode: AddressMode, _current_pc: u16) {
    cpu.y = cpu.y.wrapping_sub(1);
    update_zero_and_negative_flags(cpu, cpu.y);
}

pub(crate) fn op_tax(cpu: &mut CPU, _bus: &mut dyn CpuBus, _mode: AddressMode, _current_pc: u16) {
    cpu.x = cpu.a;
    update_zero_and_negative_flags(cpu, cpu.x);
}

pub(crate) fn op_tay(cpu: &mut CPU, _bus: &mut dyn CpuBus, _mode: AddressMode, _current_pc: u16) {
    cpu.y = cpu.a;
    update_zero_and_negative_flags(cpu, cpu.y);
}

pub(crate) fn op_txa(cpu: &mut CPU, _bus: &mut dyn CpuBus, _mode: AddressMode, _current_pc: u16) {
    cpu.a = cpu.x;
    update_zero_and_negative_flags(cpu, cpu.a);
}

pub(crate) fn op_tya(cpu: &mut CPU, _bus: &mut dyn CpuBus, _mode: AddressMode, _current_pc: u16) {
    cpu.a = cpu.y;
    update_zero_and_negative_flags(cpu, cpu.a);
}
