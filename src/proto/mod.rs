pub mod queue;

use std::cmp;
use std::convert::TryInto;
use std::io::Write;

use vmm_sys_util::eventfd::EventFd;

use queue::Queue;

pub trait VirtioDevice {
    /// The virtio device type.
    fn device_type(&self) -> u32;

    /// The number of queues supported by the device.
    fn num_queues(&self) -> u16;

    /// The maximum size of the specified queue. Returns `None` for an invalid index.
    fn queue_max_size(&self, index: usize) -> Option<u16>;

    /// The set of feature bits shifted by `page * 32`.
    fn device_features(&self, page: u32) -> u32;

    /// Acknowledges that this set of features should be enabled.
    fn ack_features(&mut self, page: u32, value: u32);

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
    interrupt_status: usize,
    driver_status: u32,
    queue_select: u32,
    queues: Vec<Queue>,
    config_generation: u32,
    config_space: Vec<u8>,
    device_activated: bool,
}

impl VirtioState {
    fn queues(&self) -> &[Queue] {
        self.queues.as_slice()
    }
}

pub trait WithVirtioState {
    fn device_type(&self) -> u32;
    fn virtio_state(&self) -> &VirtioState;
    fn virtio_state_mut(&mut self) -> &mut VirtioState;
}

impl<T: WithVirtioState> VirtioDevice for T {
    fn device_type(&self) -> u32 {
        <Self as WithVirtioState>::device_type(self)
    }

    fn num_queues(&self) -> u16 {
        // It's a serious error if `queues.len()` does not fit within an `u16`.
        self.virtio_state().queues.len().try_into().unwrap()
    }

    fn queue_max_size(&self, index: usize) -> Option<u16> {
        self.virtio_state().queues.get(index).map(Queue::max_size)
    }

    fn device_features(&self, page: u32) -> u32 {
        let device_features = self.virtio_state().device_features;
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

    fn ack_features(&mut self, page: u32, value: u32) {
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
