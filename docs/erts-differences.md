# ERTS Differences

PBeam follows the ERTS implementation model and observable behavior. This file lists each intentional difference from ERTS and OTP. Add a row in the same PR that adds a difference. Remove the row in the same PR that removes the difference.

| Area | PBeam behavior | ERTS behavior | Reason |
| --- | --- | --- | --- |
| Scheduling | One scheduler thread. A process runs until it waits or exits. There are no reductions and no preemption. | Many scheduler threads. Each process gets a reduction budget and yields when the budget is used. | The process runtime (#49) needs only cooperative processes for OTP IO. |
| Exit signals | An abnormal exit that reaches a linked process without `trap_exit` returns a runtime error. | The exit signal kills the linked process, and the signal propagates to its links. | Exit signal propagation is not necessary for the current fixtures. |
| Receive timeout | `receive ... after` is not supported. The `wait_timeout` instruction is not decoded. | `wait_timeout` starts a timer. The process continues at the `timeout` label when the timer expires. | The current fixtures do not use a receive timeout, and PBeam has no timers. |
| Heap limit | There is no `max_heap_size` process flag and no heap accounting. | `max_heap_size` can stop a process when its heap gets too large. Messages count toward the heap only when `message_queue_data` is `on_heap`. | PBeam uses Rust memory management, not a per-process heap. |
| Process and reference counters | Process identifiers and references come from `u64` counters. Counter overflow returns a runtime error. | The number of live processes is limited by `+P` (default 1048576). Process table slots are used again. | A `u64` counter does not overflow in practice. |
| Zero-arity BIF exit | An exit from a BIF called through `bif0` returns a runtime error. | The BIF raises the exception in the calling process. | No zero-arity BIF exits today. |
