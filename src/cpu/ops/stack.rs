use crate::bus::BusInterface;
use crate::cpu::core::CPU;
use crate::cpu::decoder::AddressMode;
use crate::cpu::stack_flags::update_zero_and_negative_flags;

// ===================== Stack Push / Pop =====================

pub(crate) fn op_pha(
    cpu: &mut CPU,
    bus: &mut dyn BusInterface,
    _mode: AddressMode,
    _current_pc: u16,
) {
    crate::cpu::stack_flags::stack_push_u8(cpu, bus, cpu.a);
}

pub(crate) fn op_pla(
    cpu: &mut CPU,
    bus: &mut dyn BusInterface,
    _mode: AddressMode,
    _current_pc: u16,
) {
    cpu.a = crate::cpu::stack_flags::stack_pop_u8(cpu, bus);
    update_zero_and_negative_flags(cpu, cpu.a);
}

pub(crate) fn op_php(
    cpu: &mut CPU,
    bus: &mut dyn BusInterface,
    _mode: AddressMode,
    _current_pc: u16,
) {
    // 6502: PHP pushes P with both B flag (bit 4) and unused bit (bit 5) set
    crate::cpu::stack_flags::stack_push_u8(cpu, bus, cpu.for_push_brk());
}

pub(crate) fn op_plp(
    cpu: &mut CPU,
    bus: &mut dyn BusInterface,
    _mode: AddressMode,
    _current_pc: u16,
) {
    let v = crate::cpu::stack_flags::stack_pop_u8(cpu, bus);
    cpu.restore_from_stack(v);
    // The poll happens on the penultimate cycle, which is the same cycle the
    // pulled byte is transferred to P — so it still sees the old I flag and
    // the IRQ is delayed by one instruction, exactly like CLI/SEI.
    // (`poll_iflag` was sampled at instruction start and is left alone.)
}