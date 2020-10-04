use vm_device::bus::MmioAddress;
use vm_device::MutDeviceMmio;
use vm_memory::GuestAddressSpace;

use crate::devices::{VirtioConfig, VirtioMmioDevice, WithDeviceOps, WithVirtioConfig};

// `M` has to be here until we sort out the corresponding situation for `Queue<M>`.
struct SomeDevice<M: GuestAddressSpace> {
    cfg: VirtioConfig<M>,
    // Other fields follow ...
}

impl<M: GuestAddressSpace> SomeDevice<M> {
    fn _new() -> Self {
        // An actual implementation would build the object here and set up device-specific
        // configuration, such as populating `self.cfg.device_features`.
        unimplemented!();
    }
}

impl<M: GuestAddressSpace> WithVirtioConfig<M> for SomeDevice<M> {
    fn device_type(&self) -> u32 {
        // Let's pretend to be a balloon device.
        5
    }

    fn virtio_config(&self) -> &VirtioConfig<M> {
        &self.cfg
    }

    fn virtio_config_mut(&mut self) -> &mut VirtioConfig<M> {
        &mut self.cfg
    }
}

impl<M: GuestAddressSpace> WithDeviceOps for SomeDevice<M> {
    type E = ();

    fn activate(&mut self) -> Result<(), Self::E> {
        // Add device-specific activation logic here.
        Ok(())
    }

    fn reset(&mut self) -> Result<(), Self::E> {
        // Add device-specific reset logic here (or do something simple if we don't intend to
        // support reset functionality for this device).
        Ok(())
    }
}

// At this point, `SomeDevice` implements `VirtioDevice` due to the automatic implementations
// enabled by `WithVirtioConfig` and `WithDeviceOps`. We can easily implement `VirtioMmioDevice`
// now as well.

impl<M: GuestAddressSpace + 'static> VirtioMmioDevice<M> for SomeDevice<M> {
    // We don't override `VirtioMmioDevice::queue_notify` if we don't need to.
}

// Since `SomeDevice` implements `VirtioMmioDevice`, we can add a MMIO bus device
// implementation to it as well. We need to do this explicitly, instead of automatically
// implementing `MutDeviceMmio` like for the other traits, because we're no longer working
// with a trait that's defined as part of the same crate.

// Adding a `static bound to simplify lifetime handling.
impl<M: GuestAddressSpace + 'static> MutDeviceMmio for SomeDevice<M> {
    fn mmio_read(&mut self, _base: MmioAddress, offset: u64, data: &mut [u8]) {
        self.read(offset, data)
    }

    fn mmio_write(&mut self, _base: MmioAddress, offset: u64, data: &[u8]) {
        self.write(offset, data)
    }
}
