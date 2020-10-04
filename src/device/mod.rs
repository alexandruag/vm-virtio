// Copyright 2018 The Chromium OS Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE-BSD-3-Clause file.
//
// Copyright 2020 Amazon.com, Inc. or its affiliates. All Rights Reserved.
//
// SPDX-License-Identifier: Apache-2.0 OR BSD-3-Clause

//! A module that offers building blocks for virtio devices.

mod mmio;
mod virtio_config;

use vm_memory::GuestAddressSpace;

use crate::Queue;

pub use mmio::VirtioMmioDevice;
pub use virtio_config::{VirtioConfig, WithVirtioConfig};

/// When the driver initializes the device, it lets the device know about the completed stages
/// using the Device Status field.
///
/// These following consts are defined in the order in which the bits would typically be set by
/// the driver. `RESET` -> `ACKNOWLEDGE` -> `DRIVER` and so on. This module is a 1:1 mapping for
/// the Device Status field in the virtio 1.1 specification, section 2.1 (except for the `RESET`
/// value, which is not explicitly defined there as such). The status flag descriptions (except
/// `RESET`) are taken from the standard.
pub mod device_status {
    /// The initial status of the device.
    pub const RESET: u8 = 0;
    /// Indicates that the guest OS has found the device and recognized it as a valid
    /// virtio device.
    pub const ACKNOWLEDGE: u8 = 1;
    /// Indicates that the guest OS knows how to drive the device.
    pub const DRIVER: u8 = 2;
    /// Indicates that something went wrong in the guest, and it has given up on the device.
    /// This could be an internal error, or the driver didn’t like the device for some reason,
    /// or even a fatal error during device operation.
    pub const FAILED: u8 = 128;
    /// Indicates that the driver has acknowledged all the features it understands, and feature
    /// negotiation is complete.
    pub const FEATURES_OK: u8 = 8;
    /// Indicates that the driver is set up and ready to drive the device.
    pub const DRIVER_OK: u8 = 4;
    /// Indicates that the device has experienced an error from which it can’t recover.
    pub const DEVICE_NEEDS_RESET: u8 = 64;
}

// Adding a `M: GuestAddressSpace` generic type parameter here as well until we sort out the
// current discussion about how a memory object/reference gets passed to a queue.
// We might end up with the queue type as an associated type here in the future, if it makes
// sense to define an interface for queues which abstracts away whether they are split or packed.
/// A common interface for Virtio devices, shared by all transports.
pub trait VirtioDevice<M: GuestAddressSpace> {
    /// The virtio device type.
    fn device_type(&self) -> u32;

    /// The maximum number of queues supported by the device.
    fn num_queues(&self) -> u16;

    /// Set the index of the queue currently selected by the driver.
    fn set_queue_select(&mut self, value: u16);

    /// Return a reference to the queue currently selected by the driver, or `None` for an
    /// invalid selection.
    fn queue(&self) -> Option<&Queue<M>>;

    /// Return a mutable reference to the queue currently selected by the driver, or `None`
    /// for an invalid selection.
    fn queue_mut(&mut self) -> Option<&mut Queue<M>>;

    /// Set the index of the currently selected device features page.
    fn set_device_features_select(&mut self, value: u32);

    /// Return the features exposed by the device from the device feature page currently
    /// selected by the driver.
    fn device_features(&self) -> u32;

    /// Set the index of the currently selected page for driver features acknowledgement.
    fn set_driver_features_select(&mut self, value: u32);

    /// Acknowledge the driver provided feature flags for the currently selected page.
    fn ack_features(&mut self, value: u32);

    /// Return the current device status flags.
    fn device_status(&self) -> u8;

    /// Acknowledge a status update from the driver, based on the provided value. This method
    /// is not just a simple accessor, but rather is expected to handle virtio device status
    /// transitions (which may involve things such as calling activation or reset logic).
    // TODO: Should we handle writing `DEVICE_NEEDS_RESET` (which is usually done by the
    // device) as part of this method as well?
    fn set_device_status(&mut self, value: u8);

    /// Validate the current device status with respect to a group of flags that must be set,
    /// and another group that must be cleared.
    fn check_device_status(&self, set: u8, cleared: u8) -> bool {
        self.device_status() & (set | cleared) == set
    }

    /// Return the current config generation value.
    fn config_generation(&self) -> u8;

    /// Read from the configuration space associated with the device into `data`,
    /// starting at `offset`.
    fn read_config(&self, offset: usize, data: &mut [u8]);

    /// Write to the configuration space associated with the device at `offset`, using
    /// input from `data`.
    fn write_config(&mut self, offset: usize, data: &[u8]);
}
