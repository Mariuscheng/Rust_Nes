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
    // BRK cycle sequence (7 cycles; the opcode fetch already ticked in step()):
    //   c2: read dummy byte at PC+1 (automatic fetch of the next byte)
    //   c3: push PCH
    //   c4: push PCL
    //   c5: push P (with B=1, bit5=1)
    //   c6: penultimate — NMI is sampled here (vector low fetch)
    //   c7: set I flag, fetch vector high
    //
    // If NMI fires on the penultimate cycle (c6), it preempts BRK:
    // the status pushed in c5 has B=1 and the original I, and the NMI vector
    // is read instead of the IRQ/BRK vector. The BRK frame stays on the stack.

    // c2: dummy fetch of the byte following the opcode.
    let _ = bus.read(current_pc.wrapping_add(1));

    // c3-c5: push PC+2 and P.
    crate::cpu::stack_flags::stack_push_u16(cpu, bus, current_pc.wrapping_add(2));
    crate::cpu::stack_flags::stack_push_u8(cpu, bus, cpu.p | 0x30);

    // c6: NMI is sampled on the penultimate cycle. BRK selects the NMI vector
    // when that poll sees a pending NMI; the frame it pushed keeps B=1.
    let nmi_taken = bus.take_nmi_edge();

    cpu.p |= 0x04;
    cpu.poll_iflag = true;

    if nmi_taken {
        // NMI preempted BRK: the BRK frame (PC+2, B=1) stays, NMI vector is used.
        let lo = bus.read(0xFFFA) as u16;
        let hi = bus.read(0xFFFB) as u16;
        cpu.pc = (hi << 8) | lo;
    } else {
        // Normal BRK: read the IRQ/BRK vector.
        let lo = bus.read(0xFFFE) as u16;
        let hi = bus.read(0xFFFF) as u16;
        cpu.pc = (hi << 8) | lo;
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
    // RTI restores I three cycles before the poll, so the poll sees the new value.
    cpu.poll_iflag = cpu.irq_disabled();
    let addr = crate::cpu::stack_flags::stack_pop_u16(cpu, bus);
    cpu.pc = addr;
}

// ===================== Branch Helper =====================

/// Generic branch helper: returns extra cycles (0 if not taken, 1 if taken, +1 if page crossed)
///
/// A taken branch without a page cross inserts its extra cycle *after* the
/// cycle the CPU polls interrupts on, so the poll uses the line state from one
/// cycle earlier (that is why a taken branch delays an IRQ by one instruction).
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
        cpu.poll_delay = 1;
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