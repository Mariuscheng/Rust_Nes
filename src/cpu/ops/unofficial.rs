use crate::bus::BusInterface;
use crate::cpu::addressing::{get_operand_address_with_page, read_operand};
use crate::cpu::core::CPU;
use crate::cpu::decoder::AddressMode;
use crate::cpu::stack_flags::update_zero_and_negative_flags;

// ===================== Unofficial Opcode Handlers =====================

/// SLO: ASL memory, then ORA A,memory
pub(crate) fn op_slo(
    cpu: &mut CPU,
    bus: &mut dyn BusInterface,
    mode: AddressMode,
    current_pc: u16,
) -> u8 {
    let res = get_operand_address_with_page(cpu, &mode, bus, current_pc.wrapping_add(1));
    let mut value = read_operand(bus, &res);
    // ASL
    cpu.set_carry((value & 0x80) != 0);
    value <<= 1;
    bus.write(res.addr, value);
    // ORA
    cpu.a |= value;
    update_zero_and_negative_flags(cpu, cpu.a);
    // RMW instructions never incur page-cross penalty on real 6502
    0
}

/// RLA: ROL memory, then AND A,memory
pub(crate) fn op_rla(
    cpu: &mut CPU,
    bus: &mut dyn BusInterface,
    mode: AddressMode,
    current_pc: u16,
) -> u8 {
    let res = get_operand_address_with_page(cpu, &mode, bus, current_pc.wrapping_add(1));
    let mut value = read_operand(bus, &res);
    let old_carry = cpu.carry();
    // ROL
    cpu.set_carry((value & 0x80) != 0);
    value = (value << 1) | (if old_carry { 1 } else { 0 });
    bus.write(res.addr, value);
    // AND
    cpu.a &= value;
    update_zero_and_negative_flags(cpu, cpu.a);
    // RMW instructions never incur page-cross penalty on real 6502
    0
}

/// SRE: LSR memory, then EOR A,memory
pub(crate) fn op_sre(
    cpu: &mut CPU,
    bus: &mut dyn BusInterface,
    mode: AddressMode,
    current_pc: u16,
) -> u8 {
    let res = get_operand_address_with_page(cpu, &mode, bus, current_pc.wrapping_add(1));
    let mut value = read_operand(bus, &res);
    // LSR
    cpu.set_carry((value & 0x01) != 0);
    value >>= 1;
    bus.write(res.addr, value);
    // EOR
    cpu.a ^= value;
    update_zero_and_negative_flags(cpu, cpu.a);
    // RMW instructions never incur page-cross penalty on real 6502
    0
}

/// RRA: ROR memory, then ADC A,memory
pub(crate) fn op_rra(
    cpu: &mut CPU,
    bus: &mut dyn BusInterface,
    mode: AddressMode,
    current_pc: u16,
) -> u8 {
    let res = get_operand_address_with_page(cpu, &mode, bus, current_pc.wrapping_add(1));
    let mut value = read_operand(bus, &res);
    let old_carry = cpu.carry();
    // ROR
    let new_carry = (value & 0x01) != 0;
    value >>= 1;
    if old_carry {
        value |= 0x80;
    }
    cpu.set_carry(new_carry);
    bus.write(res.addr, value);
    // ADC
    let carry_in = if cpu.carry() { 1u16 } else { 0u16 };
    let sum = cpu.a as u16 + value as u16 + carry_in;
    let result = sum as u8;
    cpu.set_carry(sum > 0xFF);
    cpu.set_overflow(((!(cpu.a ^ value)) & (cpu.a ^ result) & 0x80) != 0);
    cpu.a = result;
    update_zero_and_negative_flags(cpu, cpu.a);
    // RMW instructions never incur page-cross penalty on real 6502
    0
}

/// SAX: Store A & X to memory
pub(crate) fn op_sax(
    cpu: &mut CPU,
    bus: &mut dyn BusInterface,
    mode: AddressMode,
    current_pc: u16,
) {
    let res = get_operand_address_with_page(cpu, &mode, bus, current_pc.wrapping_add(1));
    // Indexed addressing modes perform a dummy read before the write.
    // IndirectX already performs its dummy read at (zp+X) during address
    // calculation; no additional dummy read at the final effective address.
    if !matches!(mode, AddressMode::ZeroPage | AddressMode::Absolute | AddressMode::IndirectX | AddressMode::ZeroPageX | AddressMode::ZeroPageY) {
        let dummy_addr = res.dummy_addr.unwrap_or(res.addr);
        bus.read(dummy_addr);
    }
    bus.write(res.addr, cpu.a & cpu.x);
}

/// LAX: Load A and X from memory (both get the same value)
pub(crate) fn op_lax(
    cpu: &mut CPU,
    bus: &mut dyn BusInterface,
    mode: AddressMode,
    current_pc: u16,
) -> u8 {
    let res = get_operand_address_with_page(cpu, &mode, bus, current_pc.wrapping_add(1));
    let value = read_operand(bus, &res);
    cpu.a = value;
    cpu.x = value;
    update_zero_and_negative_flags(cpu, cpu.a);
    if res.page_crossed { 1 } else { 0 }
}

/// DCP: DEC memory, then CMP A,memory
pub(crate) fn op_dcp(
    cpu: &mut CPU,
    bus: &mut dyn BusInterface,
    mode: AddressMode,
    current_pc: u16,
) -> u8 {
    let res = get_operand_address_with_page(cpu, &mode, bus, current_pc.wrapping_add(1));
    // DEC memory
    let dec_value = read_operand(bus, &res).wrapping_sub(1);
    bus.write(res.addr, dec_value);
    // CMP A vs dec_value
    cpu.set_carry(cpu.a >= dec_value);
    let result = cpu.a.wrapping_sub(dec_value);
    update_zero_and_negative_flags(cpu, result);
    // RMW instructions never incur page-cross penalty on real 6502
    0
}

/// ISC: INC memory, then SBC A,memory
pub(crate) fn op_isc(
    cpu: &mut CPU,
    bus: &mut dyn BusInterface,
    mode: AddressMode,
    current_pc: u16,
) -> u8 {
    let res = get_operand_address_with_page(cpu, &mode, bus, current_pc.wrapping_add(1));
    let addr = res.addr;

    // INC memory
    let new_m = read_operand(bus, &res).wrapping_add(1);
    bus.write(addr, new_m);

    // SBC with m = new_m
    let carry_in = if cpu.carry() { 1u16 } else { 0u16 };
    let sum = cpu.a as u16 + (!new_m as u16) + carry_in;
    let result = sum as u8;
    cpu.set_carry(sum > 0xFF);
    cpu.set_overflow((((cpu.a ^ new_m) & (cpu.a ^ result)) & 0x80) != 0);
    cpu.a = result;
    update_zero_and_negative_flags(cpu, cpu.a);

    // RMW instructions never incur page-cross penalty on real 6502
    0
}

/// ANC: AND immediate, then copy bit 7 to carry
pub(crate) fn op_anc(
    cpu: &mut CPU,
    bus: &mut dyn BusInterface,
    _mode: AddressMode,
    current_pc: u16,
) {
    let imm = bus.read(current_pc.wrapping_add(1));
    cpu.a &= imm;
    update_zero_and_negative_flags(cpu, cpu.a);
    // Copy N flag to C flag
    cpu.set_carry(cpu.negative());
}

/// ALR: AND immediate, then LSR A
pub(crate) fn op_alr(
    cpu: &mut CPU,
    bus: &mut dyn BusInterface,
    _mode: AddressMode,
    current_pc: u16,
) {
    let imm = bus.read(current_pc.wrapping_add(1));
    cpu.a &= imm;
    // LSR A
    cpu.set_carry((cpu.a & 0x01) != 0);
    cpu.a >>= 1;
    update_zero_and_negative_flags(cpu, cpu.a);
}

/// ARR: AND immediate, then ROR A (with special carry/overflow behavior)
pub(crate) fn op_arr(
    cpu: &mut CPU,
    bus: &mut dyn BusInterface,
    _mode: AddressMode,
    current_pc: u16,
) {
    let imm = bus.read(current_pc.wrapping_add(1));
    cpu.a &= imm;
    let old_carry = cpu.carry();
    let new_carry = (cpu.a & 0x01) != 0;
    cpu.a >>= 1;
    if old_carry {
        cpu.a |= 0x80;
    }
    cpu.set_carry(new_carry);
    // ARR has special flag behavior: bit 5 and bit 6 of result set C and V
    cpu.set_overflow(((cpu.a >> 6) & 1) != ((cpu.a >> 5) & 1));
    cpu.set_carry(((cpu.a >> 6) & 1) != 0);
    update_zero_and_negative_flags(cpu, cpu.a);
}

/// AXS: A & X, store into X, then subtract immediate (store result in X)
pub(crate) fn op_axs(
    cpu: &mut CPU,
    bus: &mut dyn BusInterface,
    _mode: AddressMode,
    current_pc: u16,
) {
    let imm = bus.read(current_pc.wrapping_add(1));
    let and_val = cpu.a & cpu.x;
    let result = and_val.wrapping_sub(imm);
    cpu.x = result;
    cpu.set_carry(and_val >= imm);
    update_zero_and_negative_flags(cpu, cpu.x);
}

/// ANX: A & X & imm -> A, X (also known as ATX/XAA)
pub(crate) fn op_anx(
    cpu: &mut CPU,
    bus: &mut dyn BusInterface,
    _mode: AddressMode,
    current_pc: u16,
) {
    let imm = bus.read(current_pc.wrapping_add(1));
    cpu.a = imm;
    cpu.x = imm;
    update_zero_and_negative_flags(cpu, imm);
}

/// SYA: Store Y & (hi + 1) at abs,X (opcode 0x9C)
/// Address does NOT cross page: hi stays as operand hi byte, only lo + X wraps.
pub(crate) fn op_sya(
    cpu: &mut CPU,
    bus: &mut dyn BusInterface,
    _mode: AddressMode,
    current_pc: u16,
) {
    let lo = bus.read(current_pc.wrapping_add(1));
    let hi = bus.read(current_pc.wrapping_add(2));
    let addr = ((hi as u16) << 8) | (lo.wrapping_add(cpu.x) as u16);
    // abs,X store: dummy read of the effective address before write
    bus.read(addr);
    let value = cpu.y & hi.wrapping_add(1);
    bus.write(addr, value);
}

/// SXA: Store X & (hi + 1) at abs,Y (opcode 0x9E)
/// Address does NOT cross page: hi stays as operand hi byte, only lo + Y wraps.
pub(crate) fn op_sxa(
    cpu: &mut CPU,
    bus: &mut dyn BusInterface,
    _mode: AddressMode,
    current_pc: u16,
) {
    let lo = bus.read(current_pc.wrapping_add(1));
    let hi = bus.read(current_pc.wrapping_add(2));
    let addr = ((hi as u16) << 8) | (lo.wrapping_add(cpu.y) as u16);
    // abs,Y store: dummy read of the effective address before write
    bus.read(addr);
    let value = cpu.x & hi.wrapping_add(1);
    bus.write(addr, value);
}

/// AHX/SHA (0x93, 0x9F): unstable store of `A & X & (high byte + 1)`.
///
/// On real hardware the address high byte is NOT incremented when the index
/// crosses a page — the value is simply lost — which is also why these
/// opcodes never take the page-cross cycle penalty.
pub(crate) fn op_ahx(
    cpu: &mut CPU,
    bus: &mut dyn BusInterface,
    mode: AddressMode,
    current_pc: u16,
) {
    let value = cpu.a & cpu.x;
    let (addr, hi) = match mode {
        AddressMode::IndirectY => {
            let zp = bus.read(current_pc.wrapping_add(1)) as u16;
            let lo = bus.read(zp);
            let hi = bus.read(zp.wrapping_add(1) & 0x00FF);
            (((hi as u16) << 8) | lo.wrapping_add(cpu.y) as u16, hi)
        }
        AddressMode::AbsoluteY => {
            let lo = bus.read(current_pc.wrapping_add(1));
            let hi = bus.read(current_pc.wrapping_add(2));
            (((hi as u16) << 8) | lo.wrapping_add(cpu.y) as u16, hi)
        }
        _ => return,
    };
    // Dummy read of the (possibly broken) address before the write, as SYA/SXA do.
    bus.read(addr);
    bus.write(addr, value & hi.wrapping_add(1));
}

/// TAS/SHS (0x9B): `A = X = SP = A & X`, then an unstable store of
/// `A & (high byte + 1)` at abs,Y with the same broken-address behaviour.
pub(crate) fn op_tas(
    cpu: &mut CPU,
    bus: &mut dyn BusInterface,
    _mode: AddressMode,
    current_pc: u16,
) {
    cpu.a &= cpu.x;
    cpu.x = cpu.a;
    cpu.sp = cpu.a;

    let lo = bus.read(current_pc.wrapping_add(1));
    let hi = bus.read(current_pc.wrapping_add(2));
    let addr = ((hi as u16) << 8) | lo.wrapping_add(cpu.y) as u16;
    bus.read(addr);
    bus.write(addr, cpu.a & hi.wrapping_add(1));
}

/// LAS: LDA abs,Y + TAX + TXS (opcode 0xBB)
pub(crate) fn op_las(
    cpu: &mut CPU,
    bus: &mut dyn BusInterface,
    mode: AddressMode,
    current_pc: u16,
) -> u8 {
    let res = get_operand_address_with_page(cpu, &mode, bus, current_pc.wrapping_add(1));
    let value = read_operand(bus, &res);
    let result = value & cpu.sp;
    cpu.a = result;
    cpu.x = result;
    cpu.sp = result;
    update_zero_and_negative_flags(cpu, cpu.a);
    if res.page_crossed { 1 } else { 0 }
}

/// KIL: On NES 2A03, KIL opcodes are 2-byte NOPs that read and discard the operand byte.
/// Unlike the original NMOS 6502, they do NOT halt the CPU.
pub(crate) fn op_kil(
    _cpu: &mut CPU,
    bus: &mut dyn BusInterface,
    _mode: AddressMode,
    current_pc: u16,
) {
    // Dummy read of the operand byte (same as NOP Immediate)
    bus.read(current_pc.wrapping_add(1));
}