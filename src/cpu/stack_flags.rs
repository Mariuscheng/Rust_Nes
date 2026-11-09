use crate::bus::BusInterface;
use crate::cpu::core::CPU;

// Stack and flag helpers extracted from `mos6502.rs`.
// Keep crate-level visibility so CPU implementation can call them.
pub(crate) fn stack_pop_u8(cpu: &mut CPU, bus: &mut dyn BusInterface) -> u8 {
    cpu.sp = cpu.sp.wrapping_add(1);
    bus.read(0x0100 + cpu.sp as u16)
}

pub(crate) fn stack_pop_u16(cpu: &mut CPU, bus: &mut dyn BusInterface) -> u16 {
    let lo = stack_pop_u8(cpu, bus) as u16;
    let hi = stack_pop_u8(cpu, bus) as u16;
    (hi << 8) | lo
}

pub(crate) fn stack_push_u8(cpu: &mut CPU, bus: &mut dyn BusInterface, data: u8) {
    bus.write(0x0100 + cpu.sp as u16, data);
    cpu.sp = cpu.sp.wrapping_sub(1);
}

pub(crate) fn stack_push_u16(cpu: &mut CPU, bus: &mut dyn BusInterface, data: u16) {
    let hi = (data >> 8) as u8;
    let lo = (data & 0xFF) as u8;
    bus.write(0x0100 + cpu.sp as u16, hi);
    cpu.sp = cpu.sp.wrapping_sub(1);
    bus.write(0x0100 + cpu.sp as u16, lo);
    cpu.sp = cpu.sp.wrapping_sub(1);
}

pub(crate) fn update_zero_and_negative_flags(cpu: &mut CPU, result: u8) {
    if result == 0 {
        cpu.p |= 0x02;
    } else {
        cpu.p &= !0x02;
    }
    if (result & 0x80) != 0 {
        cpu.p |= 0x80;
    } else {
        cpu.p &= !0x80;
    }
}
