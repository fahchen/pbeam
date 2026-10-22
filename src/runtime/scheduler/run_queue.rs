use std::collections::{HashMap, VecDeque};

use crate::{runtime::process::Priority, term::ProcessId};

/// The number of times that a `low` process must be at the head of the
/// `normal` queue before it runs (`RESCHEDULE_LOW`, `erl_process.h:373`).
const RESCHEDULE_LOW: u8 = 8;

/// The process run queue of the scheduler, as the process part of the ERTS
/// `ErtsRunQueue` (`erl_process.h`): one queue for `max`, one for `high`,
/// and one that `normal` and `low` processes share. The `schedule_count`
/// of each `low` process in the shared queue is in `low`; a `normal`
/// process has the count 1, so it needs no entry.
#[derive(Debug, Default)]
pub(in crate::runtime) struct RunQueue {
    max: VecDeque<ProcessId>,
    high: VecDeque<ProcessId>,
    normal: VecDeque<ProcessId>,
    low: HashMap<ProcessId, u8>,
}

impl RunQueue {
    /// Put the process `id` at the end of the queue of `priority`, as
    /// `enqueue_process` (`erl_process.c:3953-3971`) does: a `low` process
    /// goes into the `normal` queue with the count `RESCHEDULE_LOW`, and
    /// each other process gets the count 1.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// queue.push(ProcessId(1), Priority::Low);
    /// ```
    pub(in crate::runtime) fn push(&mut self, id: ProcessId, priority: Priority) {
        match priority {
            Priority::Max => self.max.push_back(id),
            Priority::High => self.high.push_back(id),
            Priority::Normal => self.normal.push_back(id),
            Priority::Low => {
                self.normal.push_back(id);
                self.low.insert(id, RESCHEDULE_LOW);
            }
        }
    }

    /// Take the next process to run, as `pick_next_process` of
    /// `erts_schedule` (`erl_process.c:9993-10023`) does: the head of the
    /// `max` queue, else of the `high` queue, else of the shared queue.
    /// The head of the shared queue first decrements its count, and goes to
    /// the end of the queue when the count is then more than 0 and it is not
    /// the last entry (`check_requeue_process`, `erl_process.c:4039-4054`).
    /// Return `None` when the queues are empty.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(queue.pop(), Some(ProcessId(1)));
    /// ```
    pub(in crate::runtime) fn pop(&mut self) -> Option<ProcessId> {
        if let Some(id) = self.max.pop_front() {
            return Some(id);
        }
        if let Some(id) = self.high.pop_front() {
            return Some(id);
        }
        // A queue with no `low` process takes its head, as the count 1 of
        // each `normal` process gives.
        if self.low.is_empty() {
            return self.normal.pop_front();
        }
        loop {
            let id = *self.normal.front()?;
            let last = self.normal.len() == 1;
            if let Some(count) = self.low.get_mut(&id) {
                *count -= 1;
                if *count > 0 && !last {
                    self.normal.rotate_left(1);
                    continue;
                }
                self.low.remove(&id);
            }
            return self.normal.pop_front();
        }
    }

    /// Return the pids of the queues in run order, for the scheduler unit
    /// tests.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(queue.ids(), [ProcessId(1)]);
    /// ```
    #[cfg(test)]
    pub(super) fn ids(&self) -> Vec<ProcessId> {
        let queues = self.max.iter().chain(&self.high).chain(&self.normal);
        queues.copied().collect()
    }
}
