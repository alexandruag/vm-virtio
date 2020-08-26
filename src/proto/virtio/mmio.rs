use std::convert::TryInto;
use std::sync::atomic::Ordering;

use vm_memory::{ByteValued, GuestAddress};

use super::device::{device_status, VirtioDevice};
use super::queue::Queue;

// required by the virtio mmio device register layout at offset 0 from base
const MMIO_MAGIC_VALUE: u32 = 0x7472_6976;

// current version specified by the mmio standard (legacy devices used 1 here)
const MMIO_VERSION: u32 = 2;

// TODO: crosvm uses 0 here(?), but IIRC virtio specified some other vendor id that should be used
const VENDOR_ID: u32 = 0;

trait VirtioMmioDevice: VirtioDevice {
    // TODO: refactor? replace? move?
    fn update_queue_field<F: FnOnce(&mut Queue)>(&mut self, f: F) {
        if self.check_device_status(
            device_status::FEATURES_OK,
            device_status::DRIVER_OK | device_status::FAILED,
        ) {
            // TODO: Some message or smt if `None`?
            self.queue_mut().map(f);
        } else {
            warn!(
                "update virtio queue in invalid state 0x{:x}",
                self.device_status()
            );
        }
    }

    fn read(&mut self, offset: u64, data: &mut [u8]) {
        match offset {
            0x00..=0xff if data.len() == 4 => {
                let v = match offset {
                    0x0 => MMIO_MAGIC_VALUE,
                    0x04 => MMIO_VERSION,
                    0x08 => self.device_type(),
                    0x0c => VENDOR_ID,
                    0x10 => {
                        let device_features_select = self.device_features_select();
                        let mut features = self.device_features();
                        // TODO: ??!
                        if device_features_select == 1 {
                            features |= 0x1; // enable support of VirtIO Version 1
                        }
                        features
                    }
                    0x34 => self.queue().map(Queue::max_size).unwrap_or(0).into(),
                    0x44 => self.queue().map(Queue::ready).unwrap_or(false).into(),
                    0x60 => self.interrupt_status().load(Ordering::SeqCst).into(),
                    0x70 => self.device_status().into(),
                    0xfc => self.config_generation().into(),
                    _ => {
                        warn!("unknown virtio mmio register read: 0x{:x}", offset);
                        return;
                    }
                };
                // This cannot panic, because we checked that `data.len() == 4`.
                data.copy_from_slice(v.to_le_bytes().as_slice());
            }
            // It's ok to use `as` here because `offset` always fits into an `usize`.
            0x100..=0xfff => self.read_config(offset as usize - 0x100, data),
            _ => {
                warn!(
                    "invalid virtio mmio read: 0x{:x}:0x{:x}",
                    offset,
                    data.len()
                );
            }
        };
    }

    fn write(&mut self, offset: u64, data: &[u8]) {
        // Hmm.
        fn hi(v: &mut GuestAddress, x: u32) {
            *v = (*v & 0xffff_ffff) | (u64::from(x) << 32)
        }
        fn lo(v: &mut GuestAddress, x: u32) {
            *v = (*v & !0xffff_ffff) | u64::from(x)
        }

        match offset {
            0x00..=0xff if data.len() == 4 => {
                // The `try_into` below attempts to convert `data` to a `[u8; 4]`, which
                // always succeeds because we previously checked that `data.len() == 4`.
                let v = u32::from_le_bytes(data.try_into().unwrap());
                match offset {
                    0x14 => self.set_device_features_select(v),
                    0x20 => {
                        if self.check_device_status(
                            device_status::DRIVER,
                            device_status::FEATURES_OK | device_status::FAILED,
                        ) {
                            self.ack_features(v);
                        } else {
                            warn!(
                                "ack virtio features in invalid state 0x{:x}",
                                self.device_status()
                            );
                        }
                    }
                    0x24 => self.set_device_features_select(v),
                    0x30 => self.set_queue_select(v),
                    0x38 => self.update_queue_field(|q| q.size = v as u16),
                    0x44 => self.update_queue_field(|q| q.ready = v == 1),
                    0x64 => {
                        if self.check_device_status(device_status::DRIVER_OK, 0) {
                            self.interrupt_status()
                                // `as` is ok here because we only care about the lower bits.
                                .fetch_and(!(v as u8), Ordering::SeqCst);
                        }
                    }
                    // `as` is ok here because we only care about the least significant byte.
                    0x70 => self.set_device_status(v as u8),
                    0x80 => self.update_queue_field(|q| lo(&mut q.desc_table, v)),
                    0x84 => self.update_queue_field(|q| hi(&mut q.desc_table, v)),
                    0x90 => self.update_queue_field(|q| lo(&mut q.avail_ring, v)),
                    0x94 => self.update_queue_field(|q| hi(&mut q.avail_ring, v)),
                    0xa0 => self.update_queue_field(|q| lo(&mut q.used_ring, v)),
                    0xa4 => self.update_queue_field(|q| hi(&mut q.used_ring, v)),
                    _ => {
                        warn!("unknown virtio mmio register write: 0x{:x}", offset);
                    }
                }
            }
            0x100..=0xfff => {
                if self.check_device_status(device_status::DRIVER, device_status::FAILED) {
                    // It's ok to use `as` here because `offset` always fits into an `usize`.
                    self.write_config(offset as usize - 0x100, data)
                } else {
                    warn!("can not write to device config data area before driver is ready");
                }
            }
            _ => {
                warn!(
                    "invalid virtio mmio write: 0x{:x}:0x{:x}",
                    offset,
                    data.len()
                );
            }
        }
    }
}
