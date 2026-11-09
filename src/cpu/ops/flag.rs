use crate::bus::BusInterface;
use crate::cpu::core::CPU;
use crate::cpu::decoder::AddressMode;
use crate::cpu::stack_flags::update_zero_and_negative_flags;

// ===================== Status Flag Setters / Clearers =====================

pub(crate) fn op_sei(
    cpu: &mut CPU,
    _bus: &mut dyn BusInterface,
    _mode: AddressMode,
    _current_pc: u16,
) {
    let old_i = cpu.irq_disabled();
    cpu.set_irq_disable(true);
    // If I was 0→1, set the IRQ allow delay to 2: IRQ will still be
    // allowed during SEI and the next instruction (6502 samples IRQ
    // on the penultimate cycle, before the I flag write takes effect).
    if !old_i {
        cpu.irq_allow_counter = 2;
    }
}

pub(crate) fn op_sec(
    cpu: &mut CPU,
    _bus: &mut dyn BusInterface,
    _mode: AddressMode,
    _current_pc: u16,
) {
    cpu.set_carry(true);
}

pub(crate) fn op_clc(
    cpu: &mut CPU,
    _bus: &mut dyn BusInterface,
    _mode: AddressMode,
    _current_pc: u16,
) {
    cpu.set_carry(false);
}

pub(crate) fn op_cld(
    cpu: &mut CPU,
    _bus: &mut dyn BusInterface,
    _mode: AddressMode,
    _current_pc: u16,
) {
    cpu.set_decimal(false);
}

pub(crate) fn op_cli(
    cpu: &mut CPU,
    _bus: &mut dyn BusInterface,
    _mode: AddressMode,
    _current_pc: u16,
) {
    let old_i = cpu.irq_disabled();
    cpu.set_irq_disable(false);
    // 6502: interrupt is not sampled until after the *next* instruction.
    // Set counter to 2: blocks IRQ for this instruction and the next one.
    if old_i {
        cpu.irq_inhibit_counter = 2;
    }
}

pub(crate) fn op_clv(
    cpu: &mut CPU,
    _bus: &mut dyn BusInterface,
    _mode: AddressMode,
    _current_pc: u16,
) {
    cpu.set_overflow(false);
}

pub(crate) fn op_sed(
    cpu: &mut CPU,
    _bus: &mut dyn BusInterface,
    _mode: AddressMode,
    _current_pc: u16,
) {
    cpu.set_decimal(true);
}

pub(crate) fn op_txs(
    cpu: &mut CPU,
    _bus: &mut dyn BusInterface,
    _mode: AddressMode,
    _current_pc: u16,
) {
    cpu.sp = cpu.x;
}

pub(crate) fn op_tsx(
    cpu: &mut CPU,
    _bus: &mut dyn BusInterface,
    _mode: AddressMode,
    _current_pc: u16,
) {
    cpu.x = cpu.sp;
    update_zero_and_negative_flags(cpu, cpu.x);
}