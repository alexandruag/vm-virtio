use std::result;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

use event_manager::{self, EventOps, Events, MutEventSubscriber, RemoteEndpoint, SubscriberId};
use kvm_ioctls::{IoEventAddress, VmFd};
use vm_device::bus::MmioAddress;
use vm_device::MutDeviceMmio;
use vm_memory::GuestMemoryMmap;
use vmm_sys_util::epoll::EventSet;
use vmm_sys_util::eventfd::{EventFd, EFD_NONBLOCK};

use crate::proto::virtio::block::handler::process_queue;
use crate::proto::virtio::block::BlockBackend;
use crate::proto::virtio::{
    Queue, VirtioDevice, VirtioMmioDevice, VirtioState, WithVirtioState, TYPE_BLOCK,
    VIRTIO_MMIO_INT_VRING,
};

// TODO: add actual constructor !!!
// TODO: connect to mmio bus !!!
pub struct ExampleBlockDevice<B> {
    mem: GuestMemoryMmap,
    endpoint: RemoteEndpoint<Box<dyn MutEventSubscriber>>,
    vmfd: Arc<VmFd>,

    virtio_state: VirtioState,
    mmio_addr: u64,
    irq: u32,

    backend: Option<B>,
}

impl<B: BlockBackend + Send + 'static> WithVirtioState for ExampleBlockDevice<B> {
    fn device_type(&self) -> u32 {
        TYPE_BLOCK
    }

    fn virtio_state(&self) -> &VirtioState {
        &self.virtio_state
    }

    fn virtio_state_mut(&mut self) -> &mut VirtioState {
        &mut self.virtio_state
    }

    fn mem(&self) -> &GuestMemoryMmap {
        &self.mem
    }

    fn activate(&mut self) {
        let queue_evt = EventFd::new(EFD_NONBLOCK).unwrap();
        let interrupt_evt = EventFd::new(EFD_NONBLOCK).unwrap();

        self.vmfd
            .register_ioevent(&queue_evt, &IoEventAddress::Mmio(self.mmio_addr), 0u16)
            .unwrap();
        self.vmfd.register_irqfd(&interrupt_evt, self.irq).unwrap();

        let queue = self.virtio_state.queues()[0].clone();

        let handler = Asdfer {
            queue,
            queue_evt,
            interrupt_status: self.interrupt_status().clone(),
            interrupt_evt,
            mem: self.mem.clone(),
            backend: self.backend.take().unwrap(),
        };

        self.endpoint
            .call_blocking(
                move |ops| -> result::Result<SubscriberId, event_manager::Error> {
                    Ok(ops.add_subscriber(Box::new(handler)))
                },
            )
            .unwrap();
    }
}

impl<B: BlockBackend + Send + 'static> VirtioMmioDevice for ExampleBlockDevice<B> {}

impl<B: BlockBackend + Send + 'static> MutDeviceMmio for ExampleBlockDevice<B> {
    fn mmio_read(&mut self, _base: MmioAddress, offset: u64, data: &mut [u8]) {
        self.read(offset, data)
    }

    fn mmio_write(&mut self, _base: MmioAddress, offset: u64, data: &[u8]) {
        self.write(offset, data)
    }
}

struct Asdfer<B> {
    queue: Queue,
    queue_evt: EventFd,
    interrupt_status: Arc<AtomicU8>,
    interrupt_evt: EventFd,
    mem: GuestMemoryMmap,
    backend: B,
}

impl<B> Asdfer<B> {
    fn signal_used_queue(&self) {
        self.interrupt_status
            .fetch_or(VIRTIO_MMIO_INT_VRING as u8, Ordering::SeqCst);

        if let Err(e) = self.interrupt_evt.write(1) {
            error!("Failed to signal used queue: {:?}", e);
            // METRICS.block.event_fails.inc();
        };
    }
}

impl<B: BlockBackend> MutEventSubscriber for Asdfer<B> {
    fn process(&mut self, _events: Events, _ops: &mut EventOps) {
        // TODO: make sure there's no error condition on Events? Can that happen for an EventFd?

        // TODO: make it nice
        let _ = self.queue_evt.read();

        if process_queue(&self.mem, &mut self.queue, &mut self.backend) {
            self.signal_used_queue();
        }
    }

    fn init(&mut self, ops: &mut EventOps) {
        ops.add(Events::new(&self.queue_evt, EventSet::IN));
    }
}
