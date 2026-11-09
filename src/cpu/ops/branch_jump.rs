use crate::bus::BusInterface;
use crate::cpu::addressing::get_operand_address;
use crate::cpu::core::CPU;
use crate::cpu::decoder::AddressMode;

// ===================== Subroutine / Jump =====================

pub(crate) fn op_jsr(
    cpu: &mut CPU,
    bus: &mut dyn BusInterface,
    mode: AddressMode,
    current_pc: u16,
) {
    let target = get_operand_address(cpu, &mode, bus, current_pc.wrapping_add(1));
    crate::cpu::stack_flags::stack_push_u16(cpu, bus, current_pc.wrapping_add(2));
    cpu.pc = target;
}

pub(crate) fn op_rts(
    cpu: &mut CPU,
    bus: &mut dyn BusInterface,
    _: AddressMode,
    current_pc: u16,
) {
    // RTS does a dummy read of the byte immediately following the opcode
    let _ = bus.read(current_pc.wrapping_add(1));
    let return_addr = crate::cpu::stack_flags::stack_pop_u16(cpu, bus);
    cpu.pc = return_addr.wrapping_add(1);
}

pub(crate) fn op_brk(
    cpu: &mut CPU,
    bus: &mut dyn BusInterface,
    _mode: AddressMode,
    current_pc: u16,
) {
    // BRK cycle sequence (7 cycles):
    //   c1: read opcode ($00)          [handled by step(); we tick PPU here]
    //   c2: read dummy byte at PC+1    [BRK does an automatic fetch of the next byte]
    //   c3: push PCH
    //   c4: push PCL
    //   c5: push P (with B=1, bit5=1)
    //   c6: poll NMI (penultimate) / fetch vector low
    //   c7: set I flag, fetch vector high
    //
    // If NMI fires on the penultimate cycle (c6), it preempts BRK:
    // the status pushed in c5 has B=1 and original I, and NMI vector
    // is read instead of IRQ/BRK vector. The BRK frame stays on stack.

    // Cycle 1: PPU tick for opcode fetch (step() reads the opcode but doesn't tick PPU).
    tick_ppu_cycle(cpu, bus);

    // Cycle 2: dummy fetch of byte following opcode
    let _ = bus.read(current_pc.wrapping_add(1));
    tick_ppu_cycle(cpu, bus);

    // Cycles 3-4: push PCH, then PCL
    crate::cpu::stack_flags::stack_push_u16(cpu, bus, current_pc.wrapping_add(2));
    tick_ppu_cycle(cpu, bus); // c3: push PCH
    tick_ppu_cycle(cpu, bus); // c4: push PCL

    // Cycle 5: push P with B=1 and bit5=1, I unchanged from before BRK
    crate::cpu::stack_flags::stack_push_u8(cpu, bus, cpu.p | 0x30);
    tick_ppu_cycle(cpu, bus); // c5: push P

    // Cycle 6: penultimate — NMI is sampled here (also vector low fetch on real HW)
    tick_ppu_cycle(cpu, bus); // c6: poll NMI
    let nmi_taken = cpu.poll_nmi();

    // Cycle 7: set I flag and read interrupt vector
    // On real 6502, I is set and vector high is fetched on cycle 7.
    cpu.p |= 0x04; // Set I flag
    cpu.irq_inhibit_counter = 0;
    cpu.irq_allow_counter = 0;
    cpu.skip_irq = false;

    if nmi_taken {
        // NMI preempted BRK on the penultimate cycle.
        // BRK already pushed its full stack frame (PC+2, P with B=1).
        // NMI vector is used instead of IRQ/BRK vector.
        let lo = bus.read(0xFFFA) as u16;
        let hi = bus.read(0xFFFB) as u16;
        cpu.pc = (hi << 8) | lo;
    } else {
        // Normal BRK: read IRQ/BRK vector.
        // BRK sets I flag immediately (unlike CLI/SEI/PLP, there is no 1-instruction delay).
        // No counter needed — the I flag itself blocks IRQ.
        let lo = bus.read(0xFFFE) as u16;
        let hi = bus.read(0xFFFF) as u16;
        cpu.pc = (hi << 8) | lo;
    }

    // Cycle 7 PPU tick (vector high fetch).
    tick_ppu_cycle(cpu, bus);

    // 7 CPU cycles total for BRK, all 7 ticked above.
    cpu.ppu_ticks_consumed = 21; // 7 * 3 PPU dots
}

/// Tick one CPU cycle worth of PPU dots and feed NMI line state to CPU.
#[inline]
fn tick_ppu_cycle(cpu: &mut CPU, bus: &mut dyn BusInterface) {
    if bus.advance_ppu_cycle() {
        cpu.set_nmi_line(true);
    }
}

pub(crate) fn op_rti(
    cpu: &mut CPU,
    bus: &mut dyn BusInterface,
    _: AddressMode,
    current_pc: u16,
) {
    // RTI does a dummy fetch of the byte immediately following the opcode
    let _ = bus.read(current_pc.wrapping_add(1));
    let status = crate::cpu::stack_flags::stack_pop_u8(cpu, bus);
    cpu.p = (status & 0xEF) | 0x20;
    // RTI restores I flag immediately — no CLI/PLP/SEI-style delay.
    cpu.irq_inhibit_counter = 0;
    cpu.irq_allow_counter = 0;
    let addr = crate::cpu::stack_flags::stack_pop_u16(cpu, bus);
    cpu.pc = addr;
}

// ===================== Branch Helper =====================

/// Generic branch helper: returns extra cycles (0 if not taken, 1 if taken, +1 if page crossed)
/// Sets cpu.skip_irq = true for taken non-page-crossing branches
/// (6502 does not sample IRQ on the last cycle of such branches).
pub(crate) fn op_branch(
    cpu: &mut CPU,
    bus: &mut dyn BusInterface,
    current_pc: u16,
    condition: bool,
) -> u8 {
    if !condition {
        return 0; // pc already advanced by size in step()
    }
    let offset = bus.read(current_pc.wrapping_add(1)) as i8;
    let next_pc = current_pc.wrapping_add(2);
    cpu.pc = (next_pc as i32 + offset as i32) as u16; // override pc

    let mut cycles = 1;
    let page_crossed = (next_pc & 0xFF00) != (cpu.pc & 0xFF00);
    if page_crossed {
        cycles += 1;
    } else {
        // Taken branch, no page-cross: skip IRQ on last cycle
        cpu.skip_irq = true;
    }

    cycles
}

pub(crate) fn op_bpl(
    cpu: &mut CPU,
    bus: &mut dyn BusInterface,
    _mode: AddressMode,
    current_pc: u16,
) -> u8 {
    op_branch(cpu, bus, current_pc, !cpu.negative())
}

pub(crate) fn op_bmi(
    cpu: &mut CPU,
    bus: &mut dyn BusInterface,
    _mode: AddressMode,
    current_pc: u16,
) -> u8 {
    op_branch(cpu, bus, current_pc, cpu.negative())
}

pub(crate) fn op_bvc(
    cpu: &mut CPU,
    bus: &mut dyn BusInterface,
    _mode: AddressMode,
    current_pc: u16,
) -> u8 {
    op_branch(cpu, bus, current_pc, !cpu.overflow())
}

pub(crate) fn op_bne(
    cpu: &mut CPU,
    bus: &mut dyn BusInterface,
    _mode: AddressMode,
    current_pc: u16,
) -> u8 {
    op_branch(cpu, bus, current_pc, !cpu.zero())
}

pub(crate) fn op_bcs(
    cpu: &mut CPU,
    bus: &mut dyn BusInterface,
    _mode: AddressMode,
    current_pc: u16,
) -> u8 {
    op_branch(cpu, bus, current_pc, cpu.carry())
}

pub(crate) fn op_beq(
    cpu: &mut CPU,
    bus: &mut dyn BusInterface,
    _mode: AddressMode,
    current_pc: u16,
) -> u8 {
    op_branch(cpu, bus, current_pc, cpu.zero())
}

pub(crate) fn op_bvs(
    cpu: &mut CPU,
    bus: &mut dyn BusInterface,
    _mode: AddressMode,
    current_pc: u16,
) -> u8 {
    op_branch(cpu, bus, current_pc, cpu.overflow())
}

pub(crate) fn op_bcc(
    cpu: &mut CPU,
    bus: &mut dyn BusInterface,
    _mode: AddressMode,
    current_pc: u16,
) -> u8 {
    op_branch(cpu, bus, current_pc, !cpu.carry())
}

pub(crate) fn op_jmp(
    cpu: &mut CPU,
    bus: &mut dyn BusInterface,
    mode: AddressMode,
    current_pc: u16,
) {
    let target = get_operand_address(cpu, &mode, bus, current_pc.wrapping_add(1));
    cpu.pc = target;
}