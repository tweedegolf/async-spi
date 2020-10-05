use core::cell::UnsafeCell;
use core::mem::MaybeUninit;
use core::ptr::null_mut;

use async_heapless::Oneshot;

// Hardware management of NSS is not sufficient: It drives the pin low when SPE is enabled but does
// not drive the pin high when it is disabled, so it ends up floating low.

#[derive(Clone, Copy, Debug)]
pub enum Error {
    BadFrameFormat,
    Overrun,
    ModeFault,
    BadChecksum,
    Uninitialized,
}

pub trait SPIHardware {
    /// Read a data byte from the SPI peripheral. Return `Ok(None)` if no byte is ready yet. This
    /// method gets called from the interrupt handler and must always clear the cause of the
    /// interrupt.
    fn read(&self) -> Result<Option<u8>, Error>;
    /// Write a data byte to the SPI peripheral.
    fn write(&self, x: u8);
}

pub struct SPI<H: 'static> {
    handler: &'static SPIHandler<H>,
}

struct Buffer {
    start: *mut u8,
    end: *mut u8,
    /// Whether the SPI will read bytes into the buffer after writing.
    read: bool,
}

impl Buffer {
    const fn empty() -> Self {
        Self {
            start: null_mut(),
            end: null_mut(),
            read: false,
        }
    }
}

pub struct SPIHandler<H> {
    hardware: UnsafeCell<MaybeUninit<H>>,
    buf: UnsafeCell<Buffer>,
    // When the oneshot is empty, the hardware and buf are owned by the interrupt handler,
    // otherwise they are owned by the SPI struct. The interrupt handler controls the sending end
    // of the Oneshot while the SPI struct controls the receiving end.
    result: Oneshot<Result<(), Error>>,
}

unsafe impl<H> Sync for SPIHandler<H> {}

impl<H> SPIHandler<H> {
    // NOTE(uninit): Before the init function is run, no SPI exists so its methods can't be called.
    // The handle_interrupt can't be safely called manually. It will be called if the interrupt
    // handler triggers, but it should only trigger as a result of SPI::transmit.
    pub const fn new() -> Self {
        Self {
            hardware: UnsafeCell::new(MaybeUninit::uninit()),
            buf: UnsafeCell::new(Buffer::empty()),
            result: Oneshot::new(),
        }
    }
}

impl<H: SPIHardware> SPIHandler<H> {
    pub fn init(&'static self, hardware: H) -> SPI<H> {
        // Can only be run once because only one instance of H can be safely obtained from the HAL.
        unsafe { *self.hardware.get() = MaybeUninit::new(hardware) };
        SPI { handler: self }
    }

    /// NOTE(unsafe): Must only be called in the corresponding interrupt handler.
    pub unsafe fn handle_interrupt(&self) {
        // This interrupt handler should only be triggered by operations started by itself or the
        // SPI::transmit method. In either case, the result should be empty which indicates this
        // interrupt handler has ownership over the hardware and buf fields. The interupt handler
        // always has control over the result.put method. The SPI::transmit method must also
        // guarantee that the buffer is nonempty.
        debug_assert!(self.result.is_empty());
        let hardware = &mut *(&mut *self.hardware.get()).as_mut_ptr();
        let buf = &mut *self.buf.get();
        match hardware.read() {
            Err(e) => {
                self.result.put(Err(e));
            }

            Ok(None) => panic!("SPIHandler::handle_interrupt triggered without new data or error."),
            Ok(Some(b)) => {
                debug_assert!(buf.start != buf.end);
                if buf.read {
                    *buf.start = b;
                }
                buf.start = buf.start.wrapping_add(1);
                if buf.start != buf.end {
                    hardware.write(*buf.start);
                } else {
                    self.result.put(Ok(()));
                }
            }
        }
    }
}

/// A `SPI` can be obtained by calling `init` on a static `SPIHandler`.
impl<H: SPIHardware> SPI<H> {
    async fn begin(&mut self, buf: Buffer) -> Result<(), Error> {
        if buf.start == buf.end {
            return Ok(());
        }

        let recv = unsafe {
            self.handler.result.take();
            self.handler.result.recv()
        };
        unsafe {
            let hardware = &mut *(&mut *self.handler.hardware.get()).as_mut_ptr();
            let start = buf.start;
            *self.handler.buf.get() = buf;
            // Transfer control to the interrupt handler by starting the first byte transmission
            // which will trigger the interrupt when finished. This must be the last operation
            // before awaiting the reception of the result.
            hardware.write(*start);
        }
        recv.await
    }

    pub async fn transmit(&mut self, xs: &mut [u8]) -> Result<(), Error> {
        let start = xs.as_mut_ptr();
        let end = start.wrapping_add(xs.len());
        self.begin(Buffer{start, end, read: true}).await
    }

    pub async fn write(&mut self, xs: &[u8]) -> Result<(), Error> {
        let start = xs.as_ptr() as *mut u8;
        let end = start.wrapping_add(xs.len());
        self.begin(Buffer{start, end, read: false}).await
    }
}
