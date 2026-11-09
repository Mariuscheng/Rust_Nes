use crate::apu::APU;
use crate::constants::PRG_ROM_START;
use crate::joypad::Joypad;
use crate::ppu::PPU;
use crate::ram::Ram;
use crate::rom::MapperVariant;

pub trait BusInterface {
    /// Read one byte from the bus.
    ///
    /// The machine advances one CPU cycle first, then the access happens —
    /// hardware samples the bus at the end of the cycle.
    fn read(&mut self, addr: u16) -> u8;

    /// Write one byte to the bus (same cycle semantics as `read`).
    fn write(&mut self, addr: u16, data: u8);

    /// Advance the machine by one CPU cycle that performs no bus access
    /// (the internal cycles of implied instructions, interrupt sequences…).
    /// Returns true when the PPU's NMI line is asserted.
    ///
    /// TODO(phase 1c): also step the APU by one cycle here.
    fn tick(&mut self) -> bool;

    fn set_cpu_pc(&mut self, pc: u16);

    fn take_immediate_stolen_cycles(&mut self) -> u16;

    /// Number of CPU cycles elapsed so far (used by `CPU::step` to pad the
    /// remaining cycles of an instruction).
    fn cpu_cycles(&self) -> u64 {
        0
    }

    /// Current state of the PPU NMI line.
    fn nmi_line(&self) -> bool {
        false
    }
}

pub struct MainBus {
    ram: Ram,
    mapper: MapperVariant,
    ppu: PPU,
    joypad1: Joypad,
    joypad2: Joypad,
    apu: APU,
    dmc_stolen_cycles: u16,
    /// Accumulated CPU cycles (DMA parity, future scheduling).
    cpu_cycle_count: u64,
    /// CPU open bus latch: the last value driven on the CPU data bus.
    open_bus_latch: u8,
    /// PPU NMI line state, refreshed by `tick()`.
    nmi_line: bool,
    /// Pending $4014 OAM DMA page, serviced after the current instruction.
    dma_request: Option<u8>,
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
            ppu,
            joypad1,
            joypad2,
            apu,
            dmc_stolen_cycles: 0,
            cpu_cycle_count: 0,
            open_bus_latch: 0,
            nmi_line: false,
            dma_request: None,
        }
    }

    // ══════════════════════════════════════════════
    // PPU accessors
    // ══════════════════════════════════════════════

    /// Return the PPU frame counter.
    pub fn ppu_frame(&self) -> u64 {
        self.ppu.frame
    }

    /// Tick mapper IRQ once (MMC3 scanline counter on A12 edge).
    pub fn tick_mapper_irq(&mut self) {
        let irq = self.mapper.tick_mapper_irq();
        self.ppu.mapper_irq = irq;
    }

    /// Read the frame buffer into `buf`.
    pub fn read_frame_buffer(&self, buf: &mut [u32; 256 * 240]) {
        buf.copy_from_slice(self.ppu.frame_buffer.as_ref());
    }

    // ══════════════════════════════════════════════
    // APU accessors
    // ══════════════════════════════════════════════

    /// Step the APU one cycle and handle a pending DMC sample request.
    ///
    /// The DMC never reads memory itself: it exposes an address and the bus
    /// fetches the byte. Each fetched byte stalls the CPU for 4 cycles, which
    /// is accumulated in `dmc_stolen_cycles` and paid by `CPU::step`.
    fn clock_apu(&mut self) {
        if let Some(addr) = self.apu.tick_one() {
            let byte = self.peek(addr);
            self.apu.fill_sample(byte);
            self.dmc_stolen_cycles = self.dmc_stolen_cycles.saturating_add(4);
        }
    }

    /// Read memory without advancing the clock (DMC sample fetch).
    fn peek(&self, addr: u16) -> u8 {
        match addr {
            0x0000..=0x1FFF => self.ram.read(addr),
            0x6000..=0xFFFF => self.mapper.read_prg(addr),
            _ => 0,
        }
    }

    /// Drain accumulated audio samples.
    pub fn drain_audio(&mut self) -> Vec<f32> {
        self.apu.drain_audio_buffer()
    }

    // ══════════════════════════════════════════════
    // OAM DMA ($4014)
    // ══════════════════════════════════════════════

    /// Take the pending $4014 OAM DMA request (page number), if any.
    pub fn take_dma_request(&mut self) -> Option<u8> {
        self.dma_request.take()
    }

    /// Run an OAM DMA transfer, halting the CPU.
    ///
    /// Takes 513 cycles, or 514 when the request lands on an odd CPU cycle.
    /// One byte is transferred every two cycles (read + write), like the real
    /// $2004 DMA. Returns the number of CPU cycles consumed.
    pub fn run_dma(&mut self, page: u8) -> u16 {
        let base = (page as u16) << 8;
        let extra = (self.cpu_cycle_count & 1) as u16;

        // Halt/alignment cycles before the first byte.
        self.tick();
        if extra == 1 {
            self.tick();
        }

        for i in 0..256u16 {
            self.tick(); // read cycle
            let val = self.peek(base.wrapping_add(i));
            self.tick(); // write cycle
            self.ppu.oam[i as usize] = val;
        }

        513 + extra
    }

    // ══════════════════════════════════════════════
    // Joypad accessors
    // ══════════════════════════════════════════════

    /// Borrow joypad 1 mutably (for setting button state).
    pub fn joypad1_mut(&mut self) -> &mut Joypad {
        &mut self.joypad1
    }

    // ══════════════════════════════════════════════
    // Mapper / CHR helpers
    // ══════════════════════════════════════════════

    /// Sync CHR-ROM data to PPU chr_ram (bank switching may have changed).
    ///
    /// TODO: read CHR through the mapper at fetch time instead (mid-frame
    /// bank switching support), then delete this copy.
    pub fn sync_chr_to_ppu(&mut self) {
        if self.mapper.has_chr_ram() {
            return;
        }
        for i in 0..8192 {
            self.ppu.chr_ram[i] = self.mapper.read_chr(i as u16);
        }
    }

    /// Return true if any IRQ source is currently active.
    pub fn irq_asserted(&self) -> bool {
        self.ppu.mapper_irq || self.apu.frame_interrupt || self.apu.dmc_interrupt
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
        let mut vram = [0u8; 2048];
        vram.copy_from_slice(&self.ppu.vram[..2048]);
        PpuSnapshot {
            v: self.ppu.v,
            t: self.ppu.t,
            x: self.ppu.x,
            w: self.ppu.w,
            scanline: self.ppu.scanline,
            cycle: self.ppu.cycle,
            frame: self.ppu.frame,
            odd_frame: self.ppu.odd_frame,
            read_buffer: self.ppu.read_buffer,
            oam_addr: self.ppu.oam_addr,
            oam: self.ppu.oam,
            vram,
            palette: self.ppu.palette,
            chr_ram: self.ppu.chr_ram,
        }
    }

    /// Load PPU state.
    pub fn ppu_load_state(&mut self, snap: &PpuSnapshot) {
        self.ppu.v = snap.v;
        self.ppu.t = snap.t;
        self.ppu.x = snap.x;
        self.ppu.w = snap.w;
        self.ppu.scanline = snap.scanline;
        self.ppu.cycle = snap.cycle;
        self.ppu.frame = snap.frame;
        self.ppu.odd_frame = snap.odd_frame;
        self.ppu.read_buffer = snap.read_buffer;
        self.ppu.oam_addr = snap.oam_addr;
        self.ppu.oam.copy_from_slice(&snap.oam);
        self.ppu.vram[..2048].copy_from_slice(&snap.vram);
        self.ppu.palette.copy_from_slice(&snap.palette);
        self.ppu.chr_ram.copy_from_slice(&snap.chr_ram);
    }

    /// Save APU state for external serialization.
    pub fn apu_save_state(&self) -> Vec<u8> {
        self.apu.save_state()
    }

    /// Load APU state.
    pub fn apu_load_state(&mut self, data: &[u8]) {
        self.apu.load_state(data);
    }
}

/// Snapshot of PPU state for save/load (avoids exposing internals).
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
    fn read(&mut self, addr: u16) -> u8 {
        self.tick();
        match addr {
            0x0000..=0x1FFF => {
                let val = self.ram.read(addr);
                self.open_bus_latch = val;
                val
            }
            0x2000..=0x3FFF => {
                let reg = addr & 0x0007;
                let val = self.ppu.cpu_read(reg);
                self.open_bus_latch = val;
                val
            }
            0x4014 => {
                // OAMDMA write-only — read returns open bus.
                self.open_bus_latch
            }
            0x4015 => {
                let val = self.apu.read_status();
                self.open_bus_latch = val;
                val
            }
            0x4016 => {
                // bit0 = controller data; bits 1-7 = open bus
                let joy_val = self.joypad1.cpu_read();
                let val = (self.open_bus_latch & 0xFE) | (joy_val & 0x01);
                self.open_bus_latch = val;
                val
            }
            0x4017 => {
                // bit0 = controller data; bits 1-7 = open bus
                let joy_val = self.joypad2.cpu_read();
                let val = (self.open_bus_latch & 0xFE) | (joy_val & 0x01);
                self.open_bus_latch = val;
                val
            }
            0x6000..=0xFFFF => {
                let val = self.mapper.read_prg(addr);
                self.open_bus_latch = val;
                val
            }
            // $4000-$4013 (write-only APU), $4018-$5FFF (unmapped) → open bus
            _ => self.open_bus_latch,
        }
    }

    fn write(&mut self, addr: u16, data: u8) {
        self.tick();
        self.open_bus_latch = data;
        match addr {
            0x0000..=0x1FFF => self.ram.write(addr, data),
            0x2000..=0x3FFF => {
                let reg = addr & 0x0007;
                self.ppu.cpu_write(reg, data);
            }
            0x4014 => {
                // OAM DMA: the transfer itself runs after this instruction
                // (see `run_dma`), halting the CPU for 513/514 cycles.
                self.dma_request = Some(data);
            }
            0x4000..=0x4013 => {
                let _ = self.apu.write_register(addr, data);
            }
            0x4015 => {
                if let Some(addr) = self.apu.write_register(0x4015, data) {
                    let byte = self.peek(addr);
                    self.apu.fill_sample(byte);
                }
            }
            0x4016 => self.joypad1.cpu_write(data),
            0x4017 => {
                self.joypad2.cpu_write(data);
                let _ = self.apu.write_register(0x4017, data);
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

    fn tick(&mut self) -> bool {
        self.cpu_cycle_count += 1;
        self.ppu.tick(3);
        self.clock_apu();
        let a12_ticks = self.ppu.a12_rising_count;
        self.ppu.a12_rising_count = 0;
        for _ in 0..a12_ticks {
            self.tick_mapper_irq();
        }
        self.nmi_line = self.ppu.nmi_interrupt;
        self.nmi_line
    }

    fn set_cpu_pc(&mut self, pc: u16) {
        // On the 6502, when the CPU transitions to a new PC, the high byte
        // of the address can couple onto the data bus through parasitic
        // capacitance, influencing the open bus latch.
        self.open_bus_latch = (pc >> 8) as u8;
    }

    fn take_immediate_stolen_cycles(&mut self) -> u16 {
        let dma = self.dmc_stolen_cycles;
        self.dmc_stolen_cycles = 0;
        dma
    }

    fn cpu_cycles(&self) -> u64 {
        self.cpu_cycle_count
    }

    fn nmi_line(&self) -> bool {
        self.nmi_line
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rom::Mirroring;
    use crate::rom::mappers::Mapper0;

    fn test_bus() -> MainBus {
        let mapper = MapperVariant::Mapper0(Mapper0::new(vec![0u8; 16 * 1024], vec![0u8; 8 * 1024]));
        MainBus::new(
            mapper,
            Ram::new(),
            PPU::new(Mirroring::HORIZONTAL),
            Joypad::new(),
            Joypad::new(),
            APU::new(44_100),
        )
    }

    #[test]
    fn oam_dma_is_queued_then_transfers_page() {
        let mut bus = test_bus();

        for i in 0..256u16 {
            bus.write(0x0200 + i, i as u8);
        }

        // Writing $4014 only queues the transfer; the CPU keeps running.
        bus.write(0x4014, 0x02);
        assert_eq!(bus.take_dma_request(), Some(0x02));
        assert!(bus.ppu.oam.iter().all(|&b| b == 0));

        // Servicing it halts the CPU for 513/514 cycles and copies OAM.
        let before = bus.cpu_cycles();
        let cycles = bus.run_dma(0x02);
        assert!(cycles == 513 || cycles == 514, "unexpected DMA length {cycles}");
        assert_eq!(bus.cpu_cycles() - before, cycles as u64);
        for i in 0..256usize {
            assert_eq!(bus.ppu.oam[i], i as u8, "OAM byte {i}");
        }
    }
}