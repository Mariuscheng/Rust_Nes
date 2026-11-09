use crate::bus::BusInterface;
use crate::cpu::core::CPU;
use crate::cpu::decoder::AddressMode;

/// 結果封裝：有效位址 + 是否跨頁 + 跨頁前 dummy read 位址
#[derive(Copy, Clone, Debug)]
pub struct AddrResult {
    pub addr: u16,
    pub page_crossed: bool,
    /// Pre-correction address for page-cross dummy read.
    /// Only set when page_crossed is true for AbsoluteX/AbsoluteY/IndirectY.
    pub dummy_addr: Option<u16>,
}

impl AddrResult {
    pub fn new(addr: u16, page_crossed: bool, dummy_addr: Option<u16>) -> Self {
        Self { addr, page_crossed, dummy_addr }
    }
}

/// Perform page-cross dummy read (if applicable), then read actual operand.
/// On the 6502, indexed addressing modes that cross a page first read from
/// the wrong (pre-correction) address. That dummy read has side effects
/// (e.g. clearing PPU_STATUS latch).
pub(crate) fn read_operand(bus: &mut dyn BusInterface, res: &AddrResult) -> u8 {
    if let Some(dummy) = res.dummy_addr {
        bus.read(dummy);
    }
    bus.read(res.addr)
}

/// Operand read for read-modify-write instructions. Indexed absolute modes
/// always spend a cycle on the un-indexed address, even when the index does not
/// cross a page, so the dummy read happens for every RMW access.
pub(crate) fn read_operand_rmw(
    bus: &mut dyn BusInterface,
    res: &AddrResult,
    mode: &AddressMode,
) -> u8 {
    if let Some(dummy) = res.dummy_addr {
        bus.read(dummy);
    } else if matches!(
        mode,
        AddressMode::AbsoluteX | AddressMode::AbsoluteY | AddressMode::IndirectY
    ) {
        bus.read(res.addr);
    }
    bus.read(res.addr)
}

// 提供進階 API：回傳 address 與 page_cross 資訊（高階封裝，供需要計時正確性的實作使用）
pub(crate) fn get_operand_address_with_page(
    cpu: &CPU,
    mode: &AddressMode,
    bus: &mut dyn BusInterface,
    param_ptr: u16,
) -> AddrResult {
    let res = match mode {
        AddressMode::Implied => AddrResult::new(param_ptr, false, None),
        AddressMode::Immediate => AddrResult::new(param_ptr, false, None),
        AddressMode::ZeroPage => AddrResult::new(bus.read(param_ptr) as u16, false, None),
        AddressMode::ZeroPageX => {
            let base = bus.read(param_ptr);
            // Cycle 3 always reads the un-indexed zero-page address, even when
            // the indexed address stays on the same page.
            let _ = bus.read(base as u16);
            AddrResult::new(base.wrapping_add(cpu.x) as u16, false, None)
        }
        AddressMode::ZeroPageY => {
            let base = bus.read(param_ptr);
            let _ = bus.read(base as u16);
            AddrResult::new(base.wrapping_add(cpu.y) as u16, false, None)
        }
        AddressMode::Absolute => {
            let lo = bus.read(param_ptr) as u16;
            let hi = bus.read(param_ptr.wrapping_add(1)) as u16;
            AddrResult::new((hi << 8) | lo, false, None)
        }
        AddressMode::AbsoluteY => {
            let lo = bus.read(param_ptr) as u16;
            let hi = bus.read(param_ptr.wrapping_add(1)) as u16;
            let base = (hi << 8) | lo;
            let addr = base.wrapping_add(cpu.y as u16);
            let page_crossed = (base & 0xFF00) != (addr & 0xFF00);
            let dummy_addr = if page_crossed {
                Some((hi << 8) | ((lo + cpu.y as u16) & 0xFF))
            } else {
                None
            };
            AddrResult::new(addr, page_crossed, dummy_addr)
        }
        AddressMode::Relative => {
            let offset = bus.read(param_ptr) as i8;
            let next_pc = param_ptr.wrapping_add(1); // PC after the branch operand
            let target = (next_pc as i32 + offset as i32) as u16;
            let page_crossed = (next_pc & 0xFF00) != (target & 0xFF00);
            AddrResult::new(target, page_crossed, None)
        }
        AddressMode::AbsoluteX => {
            let lo = bus.read(param_ptr) as u16;
            let hi = bus.read(param_ptr.wrapping_add(1)) as u16;
            let base = (hi << 8) | lo;
            let addr = base.wrapping_add(cpu.x as u16);
            let page_crossed = (base & 0xFF00) != (addr & 0xFF00);
            let dummy_addr = if page_crossed {
                Some((hi << 8) | ((lo + cpu.x as u16) & 0xFF))
            } else {
                None
            };
            AddrResult::new(addr, page_crossed, dummy_addr)
        }
        AddressMode::IndirectX => {
            let zp = bus.read(param_ptr);
            // Cycle 3 reads the un-indexed zero-page address.
            let _ = bus.read(zp as u16);
            let base = zp.wrapping_add(cpu.x) as u16;
            let lo = bus.read(base) as u16;
            let hi = bus.read(base.wrapping_add(1) & 0x00FF) as u16;
            AddrResult::new((hi << 8) | lo, false, None)
        }
        AddressMode::IndirectY => {
            let zero_page_addr = bus.read(param_ptr) as u16;
            let lo = bus.read(zero_page_addr) as u16;
            let hi = bus.read(zero_page_addr.wrapping_add(1) & 0xFF) as u16;
            let base = (hi << 8) | lo;
            let addr = base.wrapping_add(cpu.y as u16);
            let page_crossed = (base & 0xFF00) != (addr & 0xFF00);
            let dummy_addr = if page_crossed {
                Some((base & 0xFF00) | ((base as u8).wrapping_add(cpu.y) as u16))
            } else {
                None
            };
            AddrResult::new(addr, page_crossed, dummy_addr)
        }
        AddressMode::Indirect => {
            let ptr_lo = bus.read(param_ptr) as u16;
            let ptr_hi = bus.read(param_ptr.wrapping_add(1)) as u16;
            let ptr = (ptr_hi << 8) | ptr_lo;
            let lo = bus.read(ptr) as u16;
            let hi_addr = if (ptr & 0x00FF) == 0x00FF {
                ptr & 0xFF00
            } else {
                ptr.wrapping_add(1)
            };
            let hi = bus.read(hi_addr) as u16;
            AddrResult::new((hi << 8) | lo, false, None)
        }
        AddressMode::Accumulator => {
            AddrResult::new(param_ptr, false, None)
        }
    };

    res
}

// 保持向後相容的簡單 API：僅回傳地址（舊呼叫不需改動）
pub(crate) fn get_operand_address(
    cpu: &CPU,
    mode: &AddressMode,
    bus: &mut dyn BusInterface,
    param_ptr: u16,
) -> u16 {
    get_operand_address_with_page(cpu, mode, bus, param_ptr).addr
}