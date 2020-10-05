//! A library for implementhing spi with support for async and without depending on std or alloc.

#![no_std]
mod common;
pub use common::*;

#[cfg(feature = "stm32l4x6")]
mod stm32l4x6;
