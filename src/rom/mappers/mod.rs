// src/rom/mappers/mod.rs
// Individual mapper implementations
pub mod mapper0;
pub mod mapper1;
pub mod mapper2;
pub mod mapper3;
pub mod mapper4;
pub mod mapper5;
pub mod mapper7;
pub mod vrc6;
pub mod fme7;

use super::mapper::Mapper;
use super::rom::Mirroring;

pub use mapper0::Mapper0;
pub use mapper1::Mapper1;
pub use mapper2::Mapper2;
pub use mapper3::Mapper3;
pub use mapper4::Mapper4;
pub use mapper5::Mapper5;
pub use mapper7::Mapper7;
pub use vrc6::Vrc6;
pub use fme7::Fme7;

// ── MapperVariant: static enum 取代 Box<dyn Mapper> ──

macro_rules! delegate {
    ($self:expr, $method:ident $(, $arg:expr)*) => {
        match $self {
            MapperVariant::Mapper0(m) => m.$method($($arg),*),
            MapperVariant::Mapper1(m) => m.$method($($arg),*),
            MapperVariant::Mapper2(m) => m.$method($($arg),*),
            MapperVariant::Mapper3(m) => m.$method($($arg),*),
            MapperVariant::Mapper4(m) => m.$method($($arg),*),
            MapperVariant::Mapper5(m) => m.$method($($arg),*),
            MapperVariant::Mapper7(m) => m.$method($($arg),*),
            MapperVariant::Vrc6(m) => m.$method($($arg),*),
            MapperVariant::Fme7(m) => m.$method($($arg),*),
        }
    };
}

pub enum MapperVariant {
    Mapper0(Mapper0),
    Mapper1(Mapper1),
    Mapper2(Mapper2),
    Mapper3(Mapper3),
    Mapper4(Mapper4),
    Mapper5(Mapper5),
    Mapper7(Mapper7),
    Vrc6(Vrc6),
    Fme7(Fme7),
}

impl MapperVariant {
    pub fn read_prg(&self, addr: u16) -> u8 {
        delegate!(self, read_prg, addr)
    }

    pub fn read_chr(&self, addr: u16) -> u8 {
        delegate!(self, read_chr, addr)
    }

    pub fn write_prg(&mut self, addr: u16, data: u8) {
        delegate!(self, write_prg, addr, data)
    }

    pub fn has_chr_ram(&self) -> bool {
        delegate!(self, has_chr_ram)
    }

    pub fn tick_mapper_irq(&mut self) -> bool {
        delegate!(self, tick_mapper_irq)
    }

    pub fn save_state(&self) -> Vec<u8> {
        delegate!(self, save_state)
    }

    pub fn load_state(&mut self, data: &[u8]) {
        delegate!(self, load_state, data)
    }

    pub fn write_audio_register(&mut self, addr: u16, data: u8) {
        delegate!(self, write_audio_register, addr, data)
    }

    pub fn has_extended_audio(&self) -> bool {
        delegate!(self, has_extended_audio)
    }
}