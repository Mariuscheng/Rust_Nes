use std::cell::{Cell, RefCell, RefMut};

use crate::apu::APU;
use crate::constants::PRG_ROM_START;
use crate::joypad::Joypad;
use crate::ppu::PPU;
use crate::ram::Ram;
use crate::rom::MapperVariant;

pub trait BusInterface {
    fn read(&self, addr: u16) -> u8;
    fn write(&mut self, addr: u16, data: u8);
    fn set_cpu_pc(&mut self, pc: u16);
    fn take_immediate_stolen_cycles(&mut self) -> u16;
    /// Advance PPU by 1 CPU cycle (3 PPU dots). Returns true if NMI was
    /// asserted during this cycle. If true, the NMI flag is consumed so the
    /// caller is responsible for handling it (by reading NMI vector).
    fn advance_ppu_cycle(&mut self) -> bool;
    /// Advance PPU by `ticks` PPU dots. Returns true if NMI was asserted
    /// during this batch. Provided for backward-compatible batch ticking.
    fn advance_ppu_ticks(&mut self, ticks: u16) -> bool {
        let mut result = false;
        let cycles = ticks / 3;
        for _ in 0..cycles {
            if self.advance_ppu_cycle() {
                result = true;
            }
        }
        result
    }
}

/// 安全的唯讀記憶體視圖，供 APU 在 $4015 寫入時讀取記憶體使用
struct ImmediateMemoryBusRef<'a> {
    ram: &'a Ram,
    mapper: &'a MapperVariant,
}

impl BusInterface for ImmediateMemoryBusRef<'_> {
    fn read(&self, addr: u16) -> u8 {
        match addr {
            0x0000..=0x1FFF => self.ram.read(addr),
            0x6000..=0xFFFF => self.mapper.read_prg(addr),
            _ => 0,
        }
    }
    fn write(&mut self, _addr: u16, _data: u8) {}
    fn set_cpu_pc(&mut self, _pc: u16) {}
    fn take_immediate_stolen_cycles(&mut self) -> u16 {
        0
    }
    fn advance_ppu_cycle(&mut self) -> bool {
        false
    }
}

pub struct MainBus {
    ram: Ram,
    mapper: MapperVariant,
    ppu: RefCell<PPU>,
    joypad1: RefCell<Joypad>,
    joypad2: RefCell<Joypad>,
    apu: RefCell<APU>,
    dma_stolen_cycles: u16,
    /// Accumulated CPU cycles for DMA parity tracking.
    cpu_cycle_count: Cell<u64>,
    /// CPU open bus latch: tracks the last value driven on the CPU data bus.
    /// When reading from an unmapped/write-only address, the bus retains its
    /// previous value due to parasitic capacitance.
    open_bus_latch: Cell<u8>,
}

impl MainBus {
    pub fn new(
        mapper: MapperVariant,
        ram: Ram,
        ppu: PPU,
        joypad1: Joypad,
        joypad2: Joypad,
        apu: APU,
    ) -> Self {
        Self {
            ram,
            mapper,
            ppu: RefCell::new(ppu),
            joypad1: RefCell::new(joypad1),
            joypad2: RefCell::new(joypad2),
            apu: RefCell::new(apu),
            dma_stolen_cycles: 0,
            cpu_cycle_count: Cell::new(0),
            open_bus_latch: Cell::new(0),
        }
    }

    // ══════════════════════════════════════════════
    // PPU accessors (encapsulated)
    // ══════════════════════════════════════════════

    /// Return the PPU frame counter.
    pub fn ppu_frame(&self) -> u64 {
        self.ppu.borrow().frame
    }

    /// Tick PPU for `cpu_cycles` CPU cycles (each = 3 PPU dots).
    /// Returns the number of A12 rising edges detected.
    /// Also updates the CPU cycle counter for DMA parity tracking.
    pub fn tick_ppu_cycles(&self, cpu_cycles: u16) -> u8 {
        self.cpu_cycle_count.set(self.cpu_cycle_count.get() + cpu_cycles as u64);
        let mut ppu = self.ppu.borrow_mut();
        ppu.tick(cpu_cycles.saturating_mul(3));
        let a12_count = ppu.a12_rising_count;
        ppu.a12_rising_count = 0;
        a12_count
    }

    /// Tick mapper IRQ once (MMC3 scanline counter on A12 edge).
    /// Stores the result into ppu.mapper_irq.
    pub fn tick_mapper_irq(&mut self) {
        let mut ppu = self.ppu.borrow_mut();
        ppu.mapper_irq = self.mapper.tick_mapper_irq();
    }

    /// Consume and return the NMI flag from PPU.
    pub fn consume_nmi(&self) -> bool {
        let mut ppu = self.ppu.borrow_mut();
        if ppu.nmi_interrupt {
            ppu.nmi_interrupt = false;
            true
        } else {
            false
        }
    }

    /// Read the frame buffer into `buf`.
    pub fn read_frame_buffer(&self, buf: &mut [u32; 256 * 240]) {
        buf.copy_from_slice(self.ppu.borrow().frame_buffer.as_ref());
    }

    // ══════════════════════════════════════════════
    // APU accessors
    // ══════════════════════════════════════════════

    /// Step APU by `cpu_cycles`. Returns stolen CPU cycles (for DMC DMA).
    pub fn step_apu(&self, cpu_cycles: u16) -> u64 {
        let bus_ref: &MainBus = self;
        self.apu.borrow_mut().step(cpu_cycles, bus_ref) as u64
    }

    /// Drain accumulated audio samples.
    pub fn drain_audio(&self) -> Vec<f32> {
        self.apu.borrow_mut().drain_audio_buffer()
    }


    // ══════════════════════════════════════════════
    // Joypad accessors
    // ══════════════════════════════════════════════

    /// Borrow joypad 1 mutably (for setting button state).
    pub fn joypad1_mut(&self) -> RefMut<'_, Joypad> {
        self.joypad1.borrow_mut()
    }

    // ══════════════════════════════════════════════
    // Mapper / CHR helpers
    // ══════════════════════════════════════════════

    /// Sync CHR-ROM data to PPU chr_ram (bank switching may have changed).
    pub fn sync_chr_to_ppu(&self) {
        if self.mapper.has_chr_ram() {
            return;
        }
        let mut ppu = self.ppu.borrow_mut();
        for i in 0..8192 {
            ppu.chr_ram[i] = self.mapper.read_chr(i as u16);
        }
    }

    /// Return true if any IRQ source is currently active.
    pub fn irq_asserted(&self) -> bool {
        let mapper_irq = self.ppu.borrow().mapper_irq;
        let frame_irq = self.apu.borrow().frame_interrupt;
        let dmc_irq = self.apu.borrow().dmc_interrupt;
        frame_irq || dmc_irq || mapper_irq
    }

    // ══════════════════════════════════════════════
    // WRAM snapshot (for external observers)
    // ══════════════════════════════════════════════

    pub fn wram_snapshot(&self) -> [u8; 8192] {
        let mut snap = [0u8; 8192];
        for i in 0..8192u16 {
            snap[i as usize] = self.mapper.read_prg(0x6000 + i);
        }
        snap
    }

    // ══════════════════════════════════════════════
    // Save/Load state
    // ══════════════════════════════════════════════

    pub fn ram_data(&self) -> &[u8; 2048] {
        self.ram.raw_data()
    }

    pub fn write_ram_raw(&mut self, data: &[u8]) {
        self.ram.write_raw(data);
    }

    pub fn mapper_save_state(&self) -> Vec<u8> {
        self.mapper.save_state()
    }

    pub fn mapper_load_state(&mut self, data: &[u8]) {
        self.mapper.load_state(data);
    }

    /// Save PPU state for external serialization.
    pub fn ppu_save_state(&self) -> PpuSnapshot {
        let ppu = self.ppu.borrow();
        let mut vram = [0u8; 2048];
        vram.copy_from_slice(&ppu.vram[..2048]);
        PpuSnapshot {
            v: ppu.v,
            t: ppu.t,
            x: ppu.x,
            w: ppu.w,
            scanline: ppu.scanline,
            cycle: ppu.cycle,
            frame: ppu.frame,
            odd_frame: ppu.odd_frame,
            read_buffer: ppu.read_buffer,
            oam_addr: ppu.oam_addr,
            oam: ppu.oam,
            vram,
            palette: ppu.palette,
            chr_ram: ppu.chr_ram,
        }
    }

    /// Load PPU state.
    pub fn ppu_load_state(&self, snap: &PpuSnapshot) {
        let mut ppu = self.ppu.borrow_mut();
        ppu.v = snap.v;
        ppu.t = snap.t;
        ppu.x = snap.x;
        ppu.w = snap.w;
        ppu.scanline = snap.scanline;
        ppu.cycle = snap.cycle;
        ppu.frame = snap.frame;
        ppu.odd_frame = snap.odd_frame;
        ppu.read_buffer = snap.read_buffer;
        ppu.oam_addr = snap.oam_addr;
        ppu.oam.copy_from_slice(&snap.oam);
        ppu.vram[..2048].copy_from_slice(&snap.vram);
        ppu.palette.copy_from_slice(&snap.palette);
        ppu.chr_ram.copy_from_slice(&snap.chr_ram);
    }

    /// Save APU state for external serialization.
    pub fn apu_save_state(&self) -> Vec<u8> {
        self.apu.borrow().save_state()
    }

    /// Load APU state.
    pub fn apu_load_state(&self, data: &[u8]) {
        self.apu.borrow_mut().load_state(data);
    }
}

/// Snapshot of PPU state for save/load (avoids exposing RefCell).
pub struct PpuSnapshot {
    pub v: u16,
    pub t: u16,
    pub x: u8,
    pub w: bool,
    pub scanline: u16,
    pub cycle: u16,
    pub frame: u64,
    pub odd_frame: bool,
    pub read_buffer: u8,
    pub oam_addr: u8,
    pub oam: [u8; 256],
    pub vram: [u8; 2048],
    pub palette: [u8; 32],
    pub chr_ram: [u8; 8192],
}

impl BusInterface for MainBus {
    fn read(&self, addr: u16) -> u8 {
        match addr {
            0x0000..=0x1FFF => {
                let val = self.ram.read(addr);
                self.open_bus_latch.set(val);
                val
            }
            0x2000..=0x3FFF => {
                let reg = addr & 0x0007;
                let val = self.ppu.borrow_mut().cpu_read(reg);
                self.open_bus_latch.set(val);
                val
            }
            0x4014 => {
                // OAMDMA write-only — read returns open bus.
                self.open_bus_latch.get()
            }
            0x4015 => {
                let val = self.apu.borrow_mut().read_status();
                self.open_bus_latch.set(val);
                val
            }
            0x4016 => {
                // bit0 = controller data; bits 1-7 = open bus
                let joy_val = self.joypad1.borrow_mut().cpu_read();
                let val = (self.open_bus_latch.get() & 0xFE) | (joy_val & 0x01);
                self.open_bus_latch.set(val);
                val
            }
            0x4017 => {
                // bit0 = controller data; bits 1-7 = open bus
                let joy_val = self.joypad2.borrow_mut().cpu_read();
                let val = (self.open_bus_latch.get() & 0xFE) | (joy_val & 0x01);
                self.open_bus_latch.set(val);
                val
            }
            0x6000..=0xFFFF => {
                let val = self.mapper.read_prg(addr);
                self.open_bus_latch.set(val);
                val
            }
            // $4000-$4013 (write-only APU), $4018-$5FFF (unmapped) → open bus
            _ => self.open_bus_latch.get(),
        }
    }

    fn write(&mut self, addr: u16, data: u8) {
        self.open_bus_latch.set(data);
        match addr {
            0x0000..=0x1FFF => self.ram.write(addr, data),
            0x2000..=0x3FFF => {
                let reg = addr & 0x0007;
                self.ppu.borrow_mut().cpu_write(reg, data);
            }
            0x4014 => {
                // OAM DMA
                let base = (data as u16) << 8;
                let ppu = self.ppu.borrow();
                let mapper = &self.mapper;
                let ram = &self.ram;
                let mut oam_data = [0u8; 256];
                for i in 0..256u16 {
                    let addr = base.wrapping_add(i);
                    let val = match addr {
                        0x0000..=0x1FFF => ram.read(addr),
                        0x6000..=0x7FFF => mapper.read_prg(addr),
                        0x8000..=0xFFFF => mapper.read_prg(addr),
                        _ => self.open_bus_latch.get(),
                    };
                    oam_data[i as usize] = val;
                }
                drop(ppu);
                // DMA takes 513 or 514 CPU cycles (even/odd cycle alignment).
                // The parity is determined by the CPU cycle count before DMA starts.
                let dma_cycles = if self.cpu_cycle_count.get() & 1 == 0 { 513 } else { 514 };
                self.dma_stolen_cycles += dma_cycles;
                // cpu_cycle_count will be incremented by tick_ppu_cycles later
                // when the stolen cycles are accounted for in the main loop.
                let mut ppu = self.ppu.borrow_mut();
                for (i, &b) in oam_data.iter().enumerate() {
                    ppu.oam[i] = b;
                }
            }
            0x4000..=0x4013 => {
                self.apu.borrow_mut().write_register(addr, data, None);
            }
            0x4015 => {
                self.apu.borrow_mut().write_register(0x4015, data, Some(&ImmediateMemoryBusRef {
                    ram: &self.ram,
                    mapper: &self.mapper,
                }));
            }
            0x4016 => self.joypad1.borrow_mut().cpu_write(data),
            0x4017 => {
                self.joypad2.borrow_mut().cpu_write(data);
                self.apu.borrow_mut().write_register(0x4017, data, None);
            }
            // Extended audio registers ($5000-$5FFF) — route to mapper for VRC6/MMC5
            0x5000..=0x5FFF => {
                if self.mapper.has_extended_audio() {
                    self.mapper.write_audio_register(addr, data);
                }
            }
            // PRG RAM / mapper registers
            0x6000..=0x7FFF => {
                self.mapper.write_prg(addr, data);
            }
            PRG_ROM_START..=0xFFFF => {
                self.mapper.write_prg(addr, data);
            }
            _ => {}
        }
    }

    fn set_cpu_pc(&mut self, pc: u16) {
        // On the 6502, when the CPU transitions to a new PC, the high byte
        // of the address can couple onto the data bus through parasitic
        // capacitance, influencing the open bus latch.
        self.open_bus_latch.set((pc >> 8) as u8);
    }

    fn take_immediate_stolen_cycles(&mut self) -> u16 {
        let dma = self.dma_stolen_cycles;
        self.dma_stolen_cycles = 0;
        dma
    }

    fn advance_ppu_cycle(&mut self) -> bool {
        self.cpu_cycle_count.set(self.cpu_cycle_count.get() + 1);
        let mut ppu = self.ppu.borrow_mut();
        ppu.tick(3);
        let a12_ticks = ppu.a12_rising_count;
        ppu.a12_rising_count = 0;
        let nmi_state = ppu.nmi_interrupt;
        drop(ppu);
        for _ in 0..a12_ticks {
            self.tick_mapper_irq();
        }
        // Return NMI line state WITHOUT consuming it.
        // Consumption happens via consume_nmi() at instruction boundaries.
        nmi_state
    }
}