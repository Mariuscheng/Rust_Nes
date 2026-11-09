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
    let old_i = cpu.irq_disabled();
    cpu.restore_from_stack(v);
    let new_i = cpu.irq_disabled();
    // If PLP clears I flag (1→0), delay IRQ blocking by 2 instructions (same as CLI)
    if old_i && !new_i {
        cpu.irq_inhibit_counter = 2;
    }
    // If PLP sets I flag (0→1), delay IRQ blocking by 2 instructions (same as SEI)
    if !old_i && new_i {
        cpu.irq_allow_counter = 2;
    }
}