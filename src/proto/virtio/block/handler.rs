use super::super::queue::Queue;
use super::request::{Request, RequestType};

struct BlockQueueHandler {
    queue: Queue,
}

impl BlockQueueHandler {
    pub(crate) fn process_queue(&mut self) -> bool {
        let mem = match self.device_state {
            DeviceState::Activated(ref mem) => mem,
            // This should never happen, it's been already validated in the event handler.
            DeviceState::Inactive => unreachable!(),
        };
        let queue = &mut self.queue;
        let mut used_any = false;
        while let Some(head) = queue.pop(mem) {
            let len;
            match Request::parse(&head, mem) {
                Ok(request) => {
                    // // If limiter.consume() fails it means there is no more TokenType::Ops
                    // // budget and rate limiting is in effect.
                    // if !self.rate_limiter.consume(1, TokenType::Ops) {
                    //     // Stop processing the queue and return this descriptor chain to the
                    //     // avail ring, for later processing.
                    //     queue.undo_pop();
                    //     METRICS.block.rate_limiter_throttled_events.inc();
                    //     break;
                    // }

                    // // Exercise the rate limiter only if this request is of data transfer type.
                    // if request.request_type == RequestType::In
                    //     || request.request_type == RequestType::Out
                    // {
                    //     // If limiter.consume() fails it means there is no more TokenType::Bytes
                    //     // budget and rate limiting is in effect.
                    //     if !self
                    //         .rate_limiter
                    //         .consume(u64::from(request.data_len), TokenType::Bytes)
                    //     {
                    //         // Revert the OPS consume().
                    //         self.rate_limiter.manual_replenish(1, TokenType::Ops);
                    //         // Stop processing the queue and return this descriptor chain to the
                    //         // avail ring, for later processing.
                    //         queue.undo_pop();
                    //         METRICS.block.rate_limiter_throttled_events.inc();
                    //         break;
                    //     }
                    // }

                    let status = match request.execute(&mut self.disk, mem) {
                        Ok(l) => {
                            len = l;
                            VIRTIO_BLK_S_OK
                        }
                        Err(e) => {
                            error!("Failed to execute request: {:?}", e);
                            METRICS.block.invalid_reqs_count.inc();
                            len = 1; // We need at least 1 byte for the status.
                            e.status()
                        }
                    };
                    // We use unwrap because the request parsing process already checked that the
                    // status_addr was valid.
                    mem.write_obj(status, request.status_addr).unwrap();
                }
                Err(e) => {
                    error!("Failed to parse available descriptor chain: {:?}", e);
                    METRICS.block.execute_fails.inc();
                    len = 0;
                }
            }

            queue.add_used(mem, head.index, len).unwrap_or_else(|e| {
                error!(
                    "Failed to add available descriptor head {}: {}",
                    head.index, e
                )
            });
            used_any = true;
        }

        // if !used_any {
        //     METRICS.block.no_avail_buffer.inc();
        // }

        used_any
    }
}