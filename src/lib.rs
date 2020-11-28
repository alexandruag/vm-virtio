// Copyright 2018 Amazon.com, Inc. or its affiliates. All Rights Reserved.
//
// Portions Copyright 2017 The Chromium OS Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE-BSD-3-Clause file.
//
// SPDX-License-Identifier: Apache-2.0 AND BSD-3-Clause
#![deny(missing_docs)]

//! Implements virtio devices, queues, and transport mechanisms.

#[macro_use]
extern crate log;
extern crate vm_memory;
extern crate vmm_sys_util;

/// Provides abstractions for virtio block device.
pub mod block;
pub mod device;
mod queue;

#[cfg(feature = "backend-stdio")]
pub use self::block::stdio_executor::StdIoBackend;
pub use self::block::{request::Request as BlockRequest, request::RequestType as BlockRequestType};
pub use self::queue::*;
