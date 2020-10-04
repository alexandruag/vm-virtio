// Copyright 2020 Amazon.com, Inc. or its affiliates. All Rights Reserved.
//
// SPDX-License-Identifier: Apache-2.0 OR BSD-3-Clause

use std::cmp;
use std::sync::atomic::AtomicU8;
use std::sync::Arc;

use vm_memory::GuestAddressSpace;

use crate::device::{device_status, VirtioDevice, VirtioMmioDevice};
use crate::Queue;

/// An object that provides a common virtio device configuration representation. It is not part
/// of the main `vm-virtio` set of interfaces, but rather can be used as a helper object in
/// conjunction with the `WithVirtioConfig` trait (provided in the same module), to enable the
/// automatic implementation of other traits such as `VirtioDevice` and `VirtioMmioDevice`.
// Adding the `M` generic parameter that's also required by `VirtioDevice` for the time being.
// The various members have `pub` visibility until we determine whether it makes sense to drop
// this in favor of adding accessors.
pub struct VirtioConfig<M: GuestAddressSpace> {
    /// The set of features exposed by the device.
    pub device_features: u64,
    /// The set of features acknowledged by the driver.
    pub driver_features: u64,
    /// Index of the current device features page.
    pub device_features_select: u32,
    /// Index of the current driver acknowledgement device features page.
    pub driver_features_select: u32,
    /// Device status flags.
    pub device_status: u8,
    /// Index of the queue currently selected by the driver.
    pub queue_select: u16,
    /// Queues associated with the device.
    pub queues: Vec<Queue<M>>,
    /// Configuration space generation number.
    pub config_generation: u8,
    /// Contents of the device configuration space.
    pub config_space: Vec<u8>,
    /// Represents whether the device has been activated or not.
    pub device_activated: bool,
    /// Device interrupt status.
    pub interrupt_status: Arc<AtomicU8>,
}

/// Helper trait which can be implemented by types that hold a `VirtioConfig` object, which then
/// allows the automatic implementation of `VirtioDevice`, `VirtioMmioDevice`, and others (PCI)
/// in the future as well.
pub trait WithVirtioConfig<M: GuestAddressSpace> {
    /// Return the virtio device type.
    fn device_type(&self) -> u32;

    /// Return a reference to the inner `VirtioConfig` object.
    fn virtio_config(&self) -> &VirtioConfig<M>;

    /// Return a mutable reference to the inner `VirtioConfig` object.
    fn virtio_config_mut(&mut self) -> &mut VirtioConfig<M>;

    /// Invoke the logic associated with activating this device.
    fn activate(&mut self);

    /// Invoke the logic associated with resetting this device.
    fn reset(&mut self);

    /// The implementor can override the trivial default implementation to provide an alternative
    /// to be used when automatically implementing `VirtioMmioDevice` for `T: WithVirtioConfig`.
    fn queue_notify(&mut self, _val: u32) {
        // Do nothing by default.
    }

    // TODO: This method assumes all queues are intended for use. We probably need to tweak it
    // for devices that support multiple queues which might not all be configured/activated by
    // the driver.
    /// Helper method which checks whether all queues are valid.
    fn queues_valid(&self) -> bool {
        self.virtio_config().queues.iter().all(Queue::is_valid)
    }
}

// We can automatically implement the `VirtioDevice` trait for objects that only explicitly
// implement `WithVirtioConfig`.
impl<M, T> VirtioDevice<M> for T
where
    M: GuestAddressSpace,
    T: WithVirtioConfig<M>,
{
    fn device_type(&self) -> u32 {
        // Avoid infinite recursion.
        <Self as WithVirtioConfig<M>>::device_type(self)
    }

    fn num_queues(&self) -> u16 {
        // It's invalid for the number of queues to exceed `u16::MAX`.
        self.virtio_config().queues.len() as u16
    }

    fn set_queue_select(&mut self, value: u16) {
        self.virtio_config_mut().queue_select = value;
    }

    fn queue(&self) -> Option<&Queue<M>> {
        let index = self.virtio_config().queue_select;
        self.virtio_config().queues.get(usize::from(index))
    }

    fn queue_mut(&mut self) -> Option<&mut Queue<M>> {
        let index = self.virtio_config().queue_select;
        self.virtio_config_mut().queues.get_mut(usize::from(index))
    }

    fn set_device_features_select(&mut self, value: u32) {
        self.virtio_config_mut().device_features_select = value;
    }

    fn device_features(&self) -> u32 {
        let device_features = self.virtio_config().device_features;
        let page = self.virtio_config().device_features_select;
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

    fn set_driver_features_select(&mut self, value: u32) {
        self.virtio_config_mut().driver_features_select = value;
    }

    fn ack_features(&mut self, value: u32) {
        let page = self.virtio_config().driver_features_select;
        let mut v = match page {
            0 => u64::from(value),
            1 => u64::from(value) << 32,
            _ => {
                warn!("Cannot acknowledge unknown features page: {}", page);
                0u64
            }
        };

        // Check if the guest is ACK'ing a feature that we didn't claim to have.
        let device_features = self.virtio_config().device_features;
        let unrequested_features = v & !device_features;
        if unrequested_features != 0 {
            // TODO: For now, we simply issue a warning here and disregard the unknown features.
            // The standard might require a stronger response; let's clear this up going forward.

            warn!("Received acknowledge request for unknown feature: {:x}", v);
            // Don't count these features as acked.
            v &= !unrequested_features;
        }
        let acked_features = self.virtio_config().driver_features;
        self.virtio_config_mut().driver_features = acked_features | v;
    }

    fn device_status(&self) -> u8 {
        self.virtio_config().device_status
    }

    fn set_device_status(&mut self, status: u8) {
        // TODO: Setting `DEVICE_NEEDS_RESET` not handled here at this point. There is a more
        // generic question regarding where this should happen with respect to the semantics of
        // `VirtioDevice::set_device_status`.

        use device_status::*;
        let device_status = self.device_status();

        // Match changed bits.
        match !device_status & status {
            ACKNOWLEDGE if device_status == RESET => {
                self.virtio_config_mut().device_status = status;
            }
            DRIVER if device_status == ACKNOWLEDGE => {
                self.virtio_config_mut().device_status = status;
            }
            FEATURES_OK if device_status == (ACKNOWLEDGE | DRIVER) => {
                self.virtio_config_mut().device_status = status;
            }
            DRIVER_OK if device_status == (ACKNOWLEDGE | DRIVER | FEATURES_OK) => {
                self.virtio_config_mut().device_status = status;
                let device_activated = self.virtio_config().device_activated;
                if !device_activated && self.queues_valid() {
                    self.activate();
                }
            }
            _ if (status & FAILED) != 0 => {
                self.virtio_config_mut().device_status |= FAILED;
            }
            // The driver writes a zero to the status register to request a device reset.
            _ if status == 0 => {
                self.reset();
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
        self.virtio_config().config_generation
    }

    fn read_config(&self, offset: usize, data: &mut [u8]) {
        let config_space = &self.virtio_config().config_space;
        let config_len = config_space.len();
        if offset >= config_len {
            error!("Failed to read from config space");
            return;
        }

        // TODO: Are partial reads ok?
        let end = cmp::min(offset.saturating_add(data.len()), config_len);
        let read_len = end - offset;
        // Cannot fail because the lengths are identical and we do bounds checking beforehand.
        data[..read_len].copy_from_slice(&config_space[offset..end])
    }

    fn write_config(&mut self, offset: usize, data: &[u8]) {
        let config_space = &mut self.virtio_config_mut().config_space;
        let config_len = config_space.len();
        if offset >= config_len {
            error!("Failed to write to config space");
            return;
        }

        // TODO: Are partial writes ok?
        let end = cmp::min(offset.saturating_add(data.len()), config_len);
        let write_len = end - offset;
        // Cannot fail because the lengths are identical and we do bounds checking beforehand.
        config_space[offset..end].copy_from_slice(&data[..write_len]);
    }
}

// TODO: There might be certain downsides when adding automatic implementations directly, as the
// following trait does. For example, any object that implements `WithVirtioConfig` will invariably
// get a `VirtioMmioDevice` implementation. We can have finer control over the auto implementations
// by using additional marker traits which have to be explicitly added to an object for any
// automatic impl to take place.

impl<M, T> VirtioMmioDevice<M> for T
where
    // Added a `static bound here while `M` is around to simplify dealing with lifetimes.
    M: GuestAddressSpace + 'static,
    T: WithVirtioConfig<M> + VirtioDevice<M>,
{
    fn interrupt_status(&self) -> &Arc<AtomicU8> {
        &self.virtio_config().interrupt_status
    }

    fn queue_notify(&mut self, val: u32) {
        <Self as WithVirtioConfig<M>>::queue_notify(self, val)
    }
}
