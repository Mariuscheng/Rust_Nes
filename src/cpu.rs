//! CPU (6502): decoding, addressing, instruction handlers and interrupt entry.
pub mod addressing;
pub mod core;
pub mod decoder;
mod interrupt;
pub mod ops;
mod registers;
pub mod stack_flags;
pub use core::CPU;
