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
    // The I write lands on the last cycle, so the poll at the end of *this*
    // instruction still sees the old value: a pending IRQ is taken right after
    // SEI. `poll_iflag` was sampled at instruction start and is left alone.
    cpu.set_irq_disable(true);
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
    // Same timing as SEI, other direction: the poll at the end of CLI still
    // sees I=1, so a pending IRQ is serviced one instruction later.
    cpu.set_irq_disable(false);
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