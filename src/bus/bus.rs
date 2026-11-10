use crate::apu::APU;
use crate::constants::PRG_ROM_START;
use crate::joypad::Joypad;
use crate::ppu::PPU;
use crate::ram::Ram;
use crate::rom::MapperVariant;

/// The view of the machine the CPU is allowed to see.
///
/// Split out of [`BusInterface`] on purpose: the CPU may read/write memory,
/// burn a cycle, learn how many cycles have elapsed, be told its new PC (an
/// open-bus side effect) and consume a latched NMI edge. Everything else —
/// DMA requests, PPU line history, mapper hooks — is system bookkeeping the
/// CPU has no business touching, and keeping it out of this trait makes the
/// dependency visible at the type level.
pub trait CpuBus {
    /// Read one byte. The machine advances one CPU cycle first.
    fn read(&mut self, addr: u16) -> u8;
    /// Write one byte (same cycle semantics as [`CpuBus::read`]).
    fn write(&mut self, addr: u16, data: u8);
    /// One CPU cycle with no bus access (implied-instruction internals,
    /// interrupt sequences). Returns true when the PPU NMI line is asserted.
    fn tick(&mut self) -> bool;
    /// Open-bus bookkeeping: the CPU is about to fetch from `pc`.
    fn set_cpu_pc(&mut self, pc: u16);
    /// DMC stalls incurred since the last call (paid as extra cycles).
    fn take_immediate_stolen_cycles(&mut self) -> u16;
    /// Cycles elapsed so far, used to top an instruction up to its budget.
    fn cpu_cycles(&self) -> u64;
    /// Consume a latched NMI edge. The NMI input is edge-sensitive, so an edge
    /// seen by the CPU survives until the interrupt is serviced.
    fn take_nmi_edge(&mut self) -> bool;
}

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

    /// Advance the machine by one CPU cycle while distributing the PPU dots
    /// explicitly (C1: lets a PPU-register access sit inside its cycle).
    /// Default keeps test buses working: dots plain ticks.
    fn tick_with_dots(&mut self, dots: u16) -> bool {
        let mut nmi = false;
        for _ in 0..dots {
            nmi |= BusInterface::tick(self);
        }
        nmi
    }

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

    /// Consume a latched NMI edge, if any. The NMI input is edge-sensitive, so
    /// an edge seen by the CPU survives until the interrupt is serviced.
    fn take_nmi_edge(&mut self) -> bool {
        false
    }

    /// NMI line state `delay` cycles before the most recent tick, i.e. the
    /// state the 6502 samples on the polling cycle of the current instruction.
    /// `delay == 0`: the penultimate-cycle sample used by almost every
    /// instruction; `delay == 1`: taken branches, whose extra cycle comes
    /// after the poll point.
    fn nmi_line_before_tick(&self, _delay: u8) -> bool {
        false
    }

    /// IRQ line state at the same sampling point.
    fn irq_asserted_before_tick(&self, _delay: u8) -> bool {
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
    /// NMI line state at the previous cycle boundary (edge detection).
    nmi_line_prev_sample: bool,
    /// NMI edge latched by the CPU: a rising edge of the NMI line survives
    /// until the next poll, even if the game clears $2002 in between.
    nmi_edge_seen: bool,
    /// Value of `nmi_edge_seen` at the previous cycle boundary. The 6502 polls
    /// interrupts on the penultimate cycle, so an edge seen during the last
    /// cycle of an instruction is only serviced after the next instruction.
    nmi_edge_seen_prev: bool,
    /// D1 dot-offset plumbing: dots recorded by the PPU at the end of the last
    /// two CPU cycles: [vbl_set, vbl_clear, nmi_enable, nmi_disable].
    dot_history: [[u32; 4]; 2],
    dot_history_idx: usize,
    /// NMI/IRQ line state sampled just before the most recent tick: this is
    /// the penultimate-cycle state the 6502 uses to decide about interrupts.
    nmi_before_tick: bool,
    irq_before_tick: bool,
    /// Same, one cycle earlier: taken branches poll one cycle before their
    /// last cycle (see `CPU::poll_delay`).
    nmi_before_prev_tick: bool,
    irq_before_prev_tick: bool,
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
            nmi_line_prev_sample: false,
            nmi_edge_seen: false,
            nmi_edge_seen_prev: false,
            dot_history: [[0; 4]; 2],
            dot_history_idx: 0,
            nmi_before_tick: false,
            irq_before_tick: false,
            nmi_before_prev_tick: false,
            irq_before_prev_tick: false,
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
        let extra = ((self.cpu_cycle_count & 1) ^ 1) as u16;

        // Halt/alignment cycles before the first byte.
        BusInterface::tick(self);
        if extra == 1 {
            BusInterface::tick(self);
        }

        for i in 0..256u16 {
            BusInterface::tick(self); // read cycle
            let val = self.peek(base.wrapping_add(i));
            BusInterface::tick(self); // write cycle
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

    /// Copy the mapper's CHR data into the PPU's pattern memory.
    ///
    /// Called at boot and after every mapper-register write, so mid-frame CHR
    /// bank switching (MMC3, VRC6, MMC5) takes effect on the next tile fetch.
    /// CHR-RAM cartridges are skipped: the PPU owns that data.
    ///
    /// Only the 1 KiB blocks the mapper reports as dirty are copied. Mappers
    /// that cannot narrow it down return 0xFF and get the old full refresh.
    pub fn refresh_chr(&mut self) {
        if self.mapper.has_chr_ram() {
            return;
        }
        let dirty = self.mapper.take_chr_dirty_blocks();
        for block in 0..8 {
            if dirty & (1 << block) == 0 {
                continue;
            }
            let base = block * 1024;
            for i in 0..1024 {
                self.ppu.chr_ram[base + i] = self.mapper.read_chr((base + i) as u16);
            }
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

impl CpuBus for MainBus {
    fn read(&mut self, addr: u16) -> u8 {
        BusInterface::read(self, addr)
    }
    fn write(&mut self, addr: u16, data: u8) {
        BusInterface::write(self, addr, data)
    }
    fn tick(&mut self) -> bool {
        BusInterface::tick(self)
    }
    fn set_cpu_pc(&mut self, pc: u16) {
        BusInterface::set_cpu_pc(self, pc)
    }
    fn take_immediate_stolen_cycles(&mut self) -> u16 {
        BusInterface::take_immediate_stolen_cycles(self)
    }
    fn cpu_cycles(&self) -> u64 {
        BusInterface::cpu_cycles(self)
    }
    fn take_nmi_edge(&mut self) -> bool {
        BusInterface::take_nmi_edge(self)
    }
}
impl BusInterface for MainBus {
    fn read(&mut self, addr: u16) -> u8 {
        BusInterface::tick(self);
        match addr {
            0x0000..=0x1FFF => {
                let val = self.ram.read(addr);
                self.open_bus_latch = val;
                val
            }
            0x2000..=0x3FFF => {
                let reg = addr & 0x0007;
                let val = self.ppu.cpu_read(reg);
                if reg == 2 {
                    // Reading $2002 clears the VBL flag. If the NMI edge was
                    // seen in this very CPU cycle it has not been committed
                    // yet, so the read cancels it; an edge that was already
                    // visible at the previous boundary still fires.
                    self.dot_history[self.dot_history_idx] = [
            self.ppu.vbl_set_dot,
            self.ppu.vbl_clear_dot,
            self.ppu.nmi_enable_dot,
            self.ppu.nmi_disable_dot,
        ];
        self.dot_history_idx ^= 1;
        self.nmi_line = self.ppu.nmi_interrupt;
                    if self.nmi_edge_seen && !self.nmi_edge_seen_prev {
                        self.nmi_edge_seen = false;
                    }
                }
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
        BusInterface::tick(self);
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
                // CHR banks may have changed mid-frame.
                self.refresh_chr();
            }
            _ => {}
        }
    }

    fn tick(&mut self) -> bool {
        self.tick_with_dots(3)
    }

    fn tick_with_dots(&mut self, dots: u16) -> bool {
        self.nmi_before_prev_tick = self.nmi_before_tick;
        self.irq_before_prev_tick = self.irq_before_tick;
        self.nmi_before_tick = self.nmi_line;
        self.nmi_edge_seen_prev = self.nmi_edge_seen;
        let irq = self.irq_asserted();
        self.irq_before_tick = irq;
        self.cpu_cycle_count += 1;
        self.ppu.tick(dots);
        self.clock_apu();
        let a12_ticks = self.ppu.a12_rising_count;
        self.ppu.a12_rising_count = 0;
        for _ in 0..a12_ticks {
            self.tick_mapper_irq();
        }
        self.dot_history[self.dot_history_idx] = [
            self.ppu.vbl_set_dot,
            self.ppu.vbl_clear_dot,
            self.ppu.nmi_enable_dot,
            self.ppu.nmi_disable_dot,
        ];
        self.dot_history_idx ^= 1;
        self.nmi_line = self.ppu.nmi_interrupt;
        // The CPU's NMI input is edge-sensitive: latch the rising edge so a
        // later $2002 read cannot swallow an NMI that has already been seen.
        if self.nmi_line && !self.nmi_line_prev_sample {
            self.nmi_edge_seen = true;
        }
        self.nmi_line_prev_sample = self.nmi_line;
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

    fn take_nmi_edge(&mut self) -> bool {
        let fired = self.nmi_edge_seen_prev;
        if fired {
            self.nmi_edge_seen = false;
            self.nmi_edge_seen_prev = false;
        }
        fired
    }

    fn nmi_line(&self) -> bool {
        self.nmi_line
    }

    /// NMI line state `delay` cycles before the most recent tick.
    /// `delay == 0` is the penultimate-cycle sample.
    fn nmi_line_before_tick(&self, delay: u8) -> bool {
        if delay == 0 {
            self.nmi_before_tick
        } else {
            self.nmi_before_prev_tick
        }
    }

    fn irq_asserted_before_tick(&self, delay: u8) -> bool {
        if delay == 0 {
            self.irq_before_tick
        } else {
            self.irq_before_prev_tick
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::bus::BusInterface as _;
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
            BusInterface::write(&mut bus, 0x0200 + i, i as u8);
        }

        // Writing $4014 only queues the transfer; the CPU keeps running.
        BusInterface::write(&mut bus, 0x4014, 0x02);
        assert_eq!(bus.take_dma_request(), Some(0x02));
        assert!(bus.ppu.oam.iter().all(|&b| b == 0));

        // Servicing it halts the CPU for 513/514 cycles and copies OAM.
        let before = BusInterface::cpu_cycles(&bus);
        let cycles = bus.run_dma(0x02);
        assert!(cycles == 513 || cycles == 514, "unexpected DMA length {cycles}");
        assert_eq!(BusInterface::cpu_cycles(&bus) - before, cycles as u64);
        for i in 0..256usize {
            assert_eq!(bus.ppu.oam[i], i as u8, "OAM byte {i}");
        }
    }

    #[test]
    fn chr_refreshes_after_mapper_register_write() {
        let mut bus = mmc3_test_bus();
        bus.refresh_chr();
        assert_eq!(bus.ppu.chr_ram[0x1000], 0);

        // MMC3 CHR mode 0: select CHR register 2, then point it at bank 5.
        BusInterface::write(&mut bus, 0x8000, 2);
        BusInterface::write(&mut bus, 0x8001, 5);
        assert_eq!(bus.ppu.chr_ram[0x1000], 5);
    }

    /// 16 KB CHR-ROM where 1 KB bank `k` is filled with byte `k`.
    fn mmc3_test_bus() -> MainBus {
        use crate::rom::mappers::Mapper4;

        let mut chr = Vec::with_capacity(16 * 1024);
        for bank in 0..16u8 {
            chr.extend(std::iter::repeat(bank).take(1024));
        }
        let mapper = MapperVariant::Mapper4(Mapper4::new(
            vec![0u8; 32 * 1024],
            chr,
            Mirroring::HORIZONTAL,
            false,
        ));
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
    fn chr_refresh_only_copies_banks_a_write_can_touch() {
        let mut bus = mmc3_test_bus();
        bus.refresh_chr();

        // Poke both halves of the pattern table. A real copy overwrites the
        // pokes, so they double as a marker for "did this write refresh?".
        bus.ppu.chr_ram[0x0000] = 0xAA;
        bus.ppu.chr_ram[0x1000] = 0xAA;

        // IRQ latch / reload / enable never change CHR banks: no copy at all.
        BusInterface::write(&mut bus, 0xC000, 0x10);
        BusInterface::write(&mut bus, 0xC001, 0);
        BusInterface::write(&mut bus, 0xE001, 0);
        assert_eq!(bus.ppu.chr_ram[0x0000], 0xAA);
        assert_eq!(bus.ppu.chr_ram[0x1000], 0xAA);

        // Selecting a register changes nothing by itself.
        BusInterface::write(&mut bus, 0x8000, 2);
        assert_eq!(bus.ppu.chr_ram[0x0000], 0xAA);
        assert_eq!(bus.ppu.chr_ram[0x1000], 0xAA);

        // R2 feeds $1000-$1FFF, so only that half is re-copied.
        BusInterface::write(&mut bus, 0x8001, 5);
        assert_eq!(bus.ppu.chr_ram[0x1000], 5);
        assert_eq!(bus.ppu.chr_ram[0x0000], 0xAA);
    }
}
