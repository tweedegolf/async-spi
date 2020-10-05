//! This is an example implementation for STM32L4x6.
use stm32l4xx_hal::gpio::gpioa;
use stm32l4xx_hal::{gpio, stm32};

use stm32::interrupt;

use crate::{Error, SPIHandler, SPIHardware};

type AF = gpio::Alternate<gpio::AF5, gpio::Input<gpio::Floating>>;
type SCK = gpioa::PA5<AF>;
type MISO = gpioa::PA6<AF>;
type MOSI = gpioa::PA7<AF>;

type Pins = (SCK, MISO, MOSI);
type Regs = stm32::SPI1;

pub struct SPI1Hardware {
    #[allow(unused)]
    pins: Pins,
    regs: Regs,
}

pub static SPI1_HANDLER: SPIHandler<SPI1Hardware> = SPIHandler::new();

impl SPI1Hardware {
    pub fn new(pins: Pins, regs: Regs) -> Self {
        regs.cr1.write(|w| unsafe {
            w.br().bits(0b011); // f_PCLK / 16
            w.cpol().clear_bit(); // CK to 0 when idle
            w.cpha().set_bit(); // data capture on falling edges
            w.mstr().set_bit(); // we are master
            w.ssm().set_bit(); // software NSS management
            w.ssi().set_bit(); // pretend NSS is always high so no other master is detected
            w
        });

        regs.cr2.write(|w| unsafe {
            w.ds().bits(0b0111); // 8-bit data transfer
            w.frxth().set_bit(); // 8-bit fifo access
            w.rxneie().set_bit(); // enable receive queue not empty interrupt
            w.errie().set_bit(); // enable error interrupts
            w
        });

        regs.cr1.modify(|_, w| w.spe().set_bit());

        Self { pins, regs }
    }
}

impl SPI1Hardware {
    /// Accessing the data register through the register block causes 32-bit reads and writes which
    /// are interpreted as two bytes by the peripheral. This pointer will access single bytes
    /// instead.
    const DR: *mut u8 = 0x4001300c as *mut u8;

    fn status(&self) -> Result<stm32::spi1::sr::R, Error> {
        use Error::*;
        let sr = self.regs.sr.read();
        if sr.tifrfe().bit() {
            Err(BadFrameFormat)
        } else if sr.ovr().bit() {
            Err(Overrun)
        } else if sr.modf().bit() {
            Err(ModeFault)
        } else if sr.crcerr().bit() {
            Err(BadChecksum)
        } else {
            Ok(sr)
        }
    }
}

impl SPIHardware for SPI1Hardware {
    fn write(&self, x: u8) {
        unsafe { Self::DR.write_volatile(x) }
    }

    fn read(&self) -> Result<Option<u8>, Error> {
        if self.status()?.rxne().bit() {
            Ok(Some(unsafe { Self::DR.read_volatile() }))
        } else {
            Ok(None)
        }
    }
}

#[interrupt]
fn SPI1() {
    // NOTE(unsafe): Must be and is called in the interrupt handler.
    unsafe { SPI1_HANDLER.handle_interrupt() };
}
