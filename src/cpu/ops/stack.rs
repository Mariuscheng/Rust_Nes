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
    // PHA: fetch, then an internal cycle while SP is driven, then the write.
    bus.tick();
    crate::cpu::stack_flags::stack_push_u8(cpu, bus, cpu.a);
}

pub(crate) fn op_pla(
    cpu: &mut CPU,
    bus: &mut dyn BusInterface,
    _mode: AddressMode,
    _current_pc: u16,
) {
    // PLA: fetch, internal, pull, then an internal cycle to update SP.
    bus.tick();
    cpu.a = crate::cpu::stack_flags::stack_pop_u8(cpu, bus);
    bus.tick();
    update_zero_and_negative_flags(cpu, cpu.a);
}

pub(crate) fn op_php(
    cpu: &mut CPU,
    bus: &mut dyn BusInterface,
    _mode: AddressMode,
    _current_pc: u16,
) {
    // 6502: PHP pushes P with both B flag (bit 4) and unused bit (bit 5) set
    bus.tick();
    crate::cpu::stack_flags::stack_push_u8(cpu, bus, cpu.for_push_brk());
}

pub(crate) fn op_plp(
    cpu: &mut CPU,
    bus: &mut dyn BusInterface,
    _mode: AddressMode,
    _current_pc: u16,
) {
    bus.tick();
    let v = crate::cpu::stack_flags::stack_pop_u8(cpu, bus);
    bus.tick();
    cpu.restore_from_stack(v);
    // The poll lands on the same cycle the pulled byte is transferred to P, so
    // it still sees the old I flag: PLP delays an IRQ by one instruction, just
    // like CLI/SEI (`poll_iflag` was sampled at instruction start).
}