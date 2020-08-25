pub mod queue;

use vmm_sys_util::eventfd::EventFd;

pub trait VirtioDevice {
    /// The virtio device type.
    fn device_type(&self) -> u32;

    /// The maximum size of each queue that this device supports.
    fn queue_max_sizes(&self) -> &[u16];

    /// The set of feature bits shifted by `page * 32`.
    fn features(&self, page: u32) -> u32 {
        let _ = page;
        0
    }

    /// Acknowledges that this set of features should be enabled.
    fn ack_features(&mut self, page: u32, value: u32);

    /// Reads this device configuration space at `offset`.
    fn read_config(&self, offset: u64, data: &mut [u8]);

    /// Writes to this device configuration space at `offset`.
    fn write_config(&mut self, offset: u64, data: &[u8]);

    /// Optionally deactivates this device and returns ownership of the guest memory map, interrupt
    /// event, and queue events.
    fn reset(&mut self) -> Option<(EventFd, Vec<EventFd>)> {
        None
    }
}

struct VirtioState {
    avail_features: u64,
    acked_features: u64,
    features_select: u32,
    acked_features_select: u32,
    interrupt_status: usize,
    driver_status: u32,
    queue_select: u32,
    queues: Vec<QueueState>,
    config_generation: u32,
    config_space: Vec<u8>,
    device_activated: bool,
}