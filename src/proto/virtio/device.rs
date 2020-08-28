use std::cmp;
use std::convert::{TryFrom, TryInto};
use std::io::Write;
use std::sync::atomic::AtomicU8;
use std::sync::Arc;

use vm_memory::GuestMemoryMmap;
use vmm_sys_util::eventfd::EventFd;

use super::queue::Queue;

/// When the driver initializes the device, it lets the device know about the
/// completed stages using the Device Status Field.
///
/// These following consts are defined in the order in which the bits would
/// typically be set by the driver. INIT -> ACKNOWLEDGE -> DRIVER and so on.
///
/// This module is a 1:1 mapping for the Device Status Field in the virtio 1.0
/// specification, section 2.1.
pub mod device_status {
    pub const INIT: u8 = 0;
    pub const ACKNOWLEDGE: u8 = 1;
    pub const DRIVER: u8 = 2;
    pub const FAILED: u8 = 128;
    pub const FEATURES_OK: u8 = 8;
    pub const DRIVER_OK: u8 = 4;
}

pub const TYPE_NET: u32 = 1;
pub const TYPE_BLOCK: u32 = 2;

pub trait VirtioDevice {
    /// The virtio device type.
    fn device_type(&self) -> u32;

    /// The number of queues supported by the device.
    fn num_queues(&self) -> u16;

    /// Return a reference to the selected queue, or `None` for an invalid index.
    fn queue(&self) -> Option<&Queue>;

    /// Return a mutable reference to the selected queue, or `None` for an invalid index.
    fn queue_mut(&mut self) -> Option<&mut Queue>;

    ///
    fn set_queue_select(&mut self, value: u32);

    /// Return the currently selected set of device features.
    fn device_features(&self) -> u32;

    ///
    fn device_features_select(&self) -> u32;

    ///
    fn set_device_features_select(&mut self, value: u32);

    /// Acknowledges the provided driver feature flags for the currently selected set.
    fn ack_features(&mut self, value: u32);

    ///
    fn set_driver_features_select(&mut self, value: u32);

    /// Device interrupt status.
    fn interrupt_status(&self) -> &Arc<AtomicU8>;

    ///
    fn device_status(&self) -> u8;

    ///
    fn set_device_status(&mut self, value: u8);

    /// ?!?!?
    fn check_device_status(&self, set: u8, clr: u8) -> bool {
        self.device_status() & (set | clr) == set
    }

    fn config_generation(&self) -> u8;

    /// Reads this device configuration space at `offset`.
    fn read_config(&self, offset: usize, data: &mut [u8]);

    /// Writes to this device configuration space at `offset`.
    fn write_config(&mut self, offset: usize, data: &[u8]);

    /// Optionally deactivates this device and returns ownership of the guest memory map, interrupt
    /// event, and queue events.
    fn reset(&mut self) -> Option<(EventFd, Vec<EventFd>)>;
}

pub struct VirtioState {
    device_features: u64,
    acked_features: u64,
    features_select: u32,
    acked_features_select: u32,
    interrupt_status: Arc<AtomicU8>,
    device_status: u8,
    queue_select: u32,
    queues: Vec<Queue>,
    config_generation: u8,
    config_space: Vec<u8>,
    device_activated: bool,
}

impl VirtioState {
    pub fn queues(&self) -> &[Queue] {
        self.queues.as_slice()
    }
}

pub trait WithVirtioState {
    fn device_type(&self) -> u32;
    fn virtio_state(&self) -> &VirtioState;
    fn virtio_state_mut(&mut self) -> &mut VirtioState;

    // Hmm AddressSpace at some point?
    fn mem(&self) -> &GuestMemoryMmap;

    fn are_queues_valid(&self) -> bool {
        self.virtio_state()
            .queues
            .iter()
            .all(|q| q.is_valid(&self.mem()))
    }

    fn activate(&mut self);
    // fn reset(&mut self);
}

impl<T: WithVirtioState> VirtioDevice for T {
    fn device_type(&self) -> u32 {
        <Self as WithVirtioState>::device_type(self)
    }

    fn num_queues(&self) -> u16 {
        // It's a serious error if `queues.len()` does not fit within an `u16`.
        self.virtio_state().queues.len().try_into().unwrap()
    }

    fn queue(&self) -> Option<&Queue> {
        let index = self.virtio_state().queue_select;
        // The unwrap only fails if the `u32` value is wider than a `usize`.
        self.virtio_state()
            .queues
            .get(usize::try_from(index).unwrap())
    }

    fn queue_mut(&mut self) -> Option<&mut Queue> {
        let index = self.virtio_state().queue_select;
        // The unwrap only fails if the `u32` value is wider than a `usize`.
        self.virtio_state_mut()
            .queues
            .get_mut(usize::try_from(index).unwrap())
    }

    fn set_queue_select(&mut self, value: u32) {
        self.virtio_state_mut().queue_select = value;
    }

    fn device_features(&self) -> u32 {
        let device_features = self.virtio_state().device_features;
        let page = self.virtio_state().features_select;
        match page {
            // Get the lower 32-bits of the features bitfield.
            0 => device_features as u32,
            // Get the upper 32-bits of the features bitfield.
            1 => (device_features >> 32) as u32,
            _ => {
                warn!("Received request for unknown features page.");
                0u32
            }
        }
    }

    fn device_features_select(&self) -> u32 {
        self.virtio_state().features_select
    }

    fn set_device_features_select(&mut self, value: u32) {
        self.virtio_state_mut().features_select = value;
    }

    fn ack_features(&mut self, value: u32) {
        let page = self.virtio_state().acked_features_select;
        let mut v = match page {
            0 => u64::from(value),
            1 => u64::from(value) << 32,
            _ => {
                warn!("Cannot acknowledge unknown features page: {}", page);
                0u64
            }
        };

        // Check if the guest is ACK'ing a feature that we didn't claim to have.
        let device_features = self.virtio_state().device_features;
        let unrequested_features = v & !device_features;
        if unrequested_features != 0 {
            warn!("Received acknowledge request for unknown feature: {:x}", v);
            // Don't count these features as acked.
            v &= !unrequested_features;
        }
        let acked_features = self.virtio_state().acked_features;
        self.virtio_state_mut().acked_features = acked_features | v;
    }

    fn set_driver_features_select(&mut self, value: u32) {
        self.virtio_state_mut().acked_features_select = value;
    }

    fn interrupt_status(&self) -> &Arc<AtomicU8> {
        &self.virtio_state().interrupt_status
    }

    fn device_status(&self) -> u8 {
        self.virtio_state().device_status
    }

    // TODO: ?!?!
    #[allow(unused_assignments)]
    fn set_device_status(&mut self, status: u8) {
        use device_status::*;
        let device_status = self.device_status();
        // match changed bits
        match !device_status & status {
            ACKNOWLEDGE if device_status == INIT => {
                self.virtio_state_mut().device_status = status;
            }
            DRIVER if device_status == ACKNOWLEDGE => {
                self.virtio_state_mut().device_status = status;
            }
            FEATURES_OK if device_status == (ACKNOWLEDGE | DRIVER) => {
                self.virtio_state_mut().device_status = status;
            }
            DRIVER_OK if device_status == (ACKNOWLEDGE | DRIVER | FEATURES_OK) => {
                self.virtio_state_mut().device_status = status;
                let device_activated = self.virtio_state().device_activated;
                if !device_activated && self.are_queues_valid() {
                    self.activate()
                }
            }
            _ if (status & FAILED) != 0 => {
                // TODO: notify backend driver to stop the device
                self.virtio_state_mut().device_status |= FAILED;
            }
            // TODO: ?!?!?!?
            _ if status == 0 => {
                // ?!?!?
                unreachable!()
                // if self.virtio_state().device_activated {
                //     let mut device_status = self.device_status;
                //     let reset_result = self.locked_device().reset();
                //     match reset_result {
                //         Some((_interrupt_evt, mut _queue_evts)) => {}
                //         None => {
                //             device_status |= FAILED;
                //         }
                //     }
                //     self.device_status = device_status;
                // }
                //
                // // If the backend device driver doesn't support reset,
                // // just leave the device marked as FAILED.
                // if self.device_status & FAILED == 0 {
                //     self.reset();
                // }
            }
            _ => {
                warn!(
                    "invalid virtio driver status transition: 0x{:x} -> 0x{:x}",
                    self.device_status(),
                    status
                );
            }
        }
    }

    fn config_generation(&self) -> u8 {
        self.virtio_state().config_generation
    }

    fn read_config(&self, offset: usize, mut data: &mut [u8]) {
        let config_space = &self.virtio_state().config_space;
        let config_len = config_space.len();
        if offset >= config_len {
            error!("Failed to read config space");
            // METRICS.block.cfg_fails.inc();
            return;
        }
        if let Some(end) = offset.checked_add(data.len()) {
            // TODO: are partial reads ok?
            // This write can't fail, offset and end are checked against config_len.
            data.write_all(&config_space[offset..cmp::min(end, config_len)])
                .unwrap();
        }
    }

    fn write_config(&mut self, offset: usize, data: &[u8]) {
        let config_space = &mut self.virtio_state_mut().config_space;
        let data_len = data.len();
        let config_len = config_space.len();

        if let Some(end) = offset.checked_add(data_len) {
            if end > config_len {
                error!("Failed to write config space");
                // METRICS.block.cfg_fails.inc();
                return;
            }
            config_space[offset..end].copy_from_slice(data);
        }
    }

    fn reset(&mut self) -> Option<(EventFd, Vec<EventFd>)> {
        unimplemented!()
    }
}
