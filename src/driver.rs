/// Driver trait system — generic abstractions for MMIO devices.
///
/// Pure module: no unsafe, no MMIO access. Implementations
/// in virtio.rs and other driver modules provide the unsafe glue.
///
/// Phase B: Networking + driver traits.

/// Result type for driver operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriverError {
    /// Device not found during probe.
    NotFound,
    /// Device initialization failed.
    InitFailed,
    /// Device not ready for operation.
    NotReady,
    /// No space to enqueue a transmit.
    BufferFull,
    /// No data available to receive.
    BufferEmpty,
    /// Operation invalid in current device state.
    InvalidState,
}

/// Generic MMIO device trait.
///
/// Implementors map an MMIO region and provide probe/init/interrupt handling.
pub trait Device {
    /// Probe the device at the given MMIO base. Returns true if recognized.
    fn probe(base: usize) -> bool where Self: Sized;

    /// Initialize the device after probing. Called once.
    fn init(&mut self) -> Result<(), DriverError>;

    /// Handle an interrupt from this device.
    fn handle_interrupt(&mut self);
}

/// Network device trait — extends Device with packet I/O.
///
/// Fixed-size buffer interface (no allocation).
/// Callers provide `&mut [u8]` slices for both TX and RX.
pub trait NetworkDevice: Device {
    /// Maximum transmission unit (bytes of payload the device accepts).
    fn mtu(&self) -> usize;

    /// Get the device MAC address (6 bytes).
    fn mac_address(&self) -> [u8; 6];

    /// Transmit a packet. `data` contains a complete Ethernet frame.
    /// Returns Ok(()) if queued, Err(BufferFull) if no TX descriptors available.
    fn transmit(&mut self, data: &[u8]) -> Result<(), DriverError>;

    /// Receive a packet into `buf`. Returns Ok(len) with the number
    /// of bytes written, or Err(BufferEmpty) if no packet is available.
    fn receive(&mut self, buf: &mut [u8]) -> Result<usize, DriverError>;

    /// Check if the device can accept a transmit.
    fn can_transmit(&self) -> bool;

    /// Check if a received packet is available.
    fn can_receive(&self) -> bool;
}
