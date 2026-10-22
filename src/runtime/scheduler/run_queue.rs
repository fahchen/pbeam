use std::collections::{HashMap, VecDeque};

use crate::{runtime::process::Priority, term::ProcessId};

/// The number of times that a `low` process must be at the head of the
/// `normal` queue before it runs (`RESCHEDULE_LOW`, `erl_process.h:373`).
const RESCHEDULE_LOW: u8 = 8;

/// The most processes that one steal takes, as ERTS limits the critical
/// section of a steal (`erl_process.c:4533-4534`).
const STEAL_LIMIT: usize = 100;

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
    /// Return the number of processes in the queue.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// queue.push(a, Priority::Low);
    /// assert_eq!(queue.len(), 1);
    /// ```
    pub(in crate::runtime) fn len(&self) -> usize {
        self.max.len() + self.high.len() + self.normal.len()
    }

    /// Take the processes that an idle scheduler steals. The queues go in
    /// priority order, `max`, `high`, then `normal`, and the first queue
    /// that gives a process ends the search; a queue that gives none lets
    /// the search go on to the next one, as in ERTS
    /// (`try_steal_task_from_victim`, `erl_process.c:4489-4592`). A queue
    /// gives at most half of its processes and at most [`STEAL_LIMIT`], from
    /// its head, and skips each process for which `stealable` is false, as
    /// ERTS skips a process bound to its run queue
    /// (`erts_try_change_runq_proc`, `erl_process.h:2722-2723`). The
    /// `normal` queue holds the `low` processes too, as the ERTS normal
    /// queue does (`erl_process.c:4510-4517`).
    ///
    /// ERTS takes `(len + 1) / 2` of a queue (`erl_process.c:4528-4534`),
    /// and its victim runs a process that is not in a queue. The scheduler
    /// of this queue keeps the process that it runs next, at the head of
    /// the first queue that is not empty, so that queue gives `len / 2`, and
    /// each later queue gives `(len + 1) / 2`. Each stolen `low` process
    /// gives the count that it has left.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// queue.push(a, Priority::Normal);
    /// queue.push(b, Priority::Normal);
    /// assert_eq!(queue.steal(|_| true), vec![(a, None)]);
    /// assert_eq!(queue.pop(), Some(b));
    /// ```
    pub(in crate::runtime) fn steal(
        &mut self,
        mut stealable: impl FnMut(ProcessId) -> bool,
    ) -> Vec<(ProcessId, Option<u8>)> {
        let mut next = true;
        let mut stolen = Vec::new();
        for queue in [&mut self.max, &mut self.high, &mut self.normal] {
            if queue.is_empty() {
                continue;
            }
            let half = if std::mem::take(&mut next) {
                queue.len() / 2
            } else {
                queue.len().div_ceil(2)
            };
            stolen = take_head(queue, half.min(STEAL_LIMIT), &mut stealable);
            if !stolen.is_empty() {
                break;
            }
        }
        stolen
            .into_iter()
            .map(|id| (id, self.low.remove(&id)))
            .collect()
    }

    /// Put the stolen process `id` at the end of the queue of its priority.
    /// A `low` process keeps the count `low` that it had in the queue of the
    /// other scheduler, as ERTS links the stolen processes into its queue
    /// with their `schedule_count` (`erl_process.c:4574-4590`); with no
    /// count, it is the next of the shared queue to run, as ERTS runs the
    /// first stolen process at once (`erl_process.c:4567-4572`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// queue.push_stolen(id, Priority::Low, Some(3));
    /// assert_eq!(queue.len(), 1);
    /// ```
    pub(in crate::runtime) fn push_stolen(
        &mut self,
        id: ProcessId,
        priority: Priority,
        low: Option<u8>,
    ) {
        if priority != Priority::Low {
            return self.push(id, priority);
        }
        self.normal.push_back(id);
        if let Some(count) = low {
            self.low.insert(id, count);
        }
    }

    #[cfg(test)]
    pub(super) fn ids(&self) -> Vec<ProcessId> {
        let queues = self.max.iter().chain(&self.high).chain(&self.normal);
        queues.copied().collect()
    }
}

/// Take at most `count` processes for which `stealable` is true from the
/// queue `queue`, from its head, and keep the others in their order.
fn take_head(
    queue: &mut VecDeque<ProcessId>,
    mut count: usize,
    stealable: &mut impl FnMut(ProcessId) -> bool,
) -> Vec<ProcessId> {
    let mut stolen = Vec::new();
    if count == 0 {
        return stolen;
    }
    let mut kept = VecDeque::with_capacity(queue.len());
    for id in queue.drain(..) {
        if count > 0 && stealable(id) {
            count -= 1;
            stolen.push(id);
        } else {
            kept.push_back(id);
        }
    }
    *queue = kept;
    stolen
}
