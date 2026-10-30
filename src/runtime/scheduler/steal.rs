//! Work stealing between the schedulers of a node.
//!
//! An ERTS scheduler with an empty run queue takes runnable processes from
//! the run queue of another scheduler (`try_steal_task`,
//! `erl_process.c:4640-4720`). A steal changes only the run queue of a
//! process: the process table of the node stays as it is. In PBeam the
//! state of a process lives in the process slots of its scheduler, and only
//! the thread of that scheduler reads it, so the thread of the victim makes
//! the handoff. A scheduler with no runnable process marks itself hungry at
//! the node ([`super::node::Node::set_hungry`]). At its next scheduling
//! step, a scheduler with more than one runnable process gives half of the
//! queue of its highest priority to the first hungry scheduler after it
//! ([`super::node::Node::give_work`], [`super::run_queue::RunQueue::steal`]).
//! The process state and the owner in the process table move together
//! ([`super::node::Node::transfer`]). The mark and each steal use the lock
//! of the idle count, so a scheduler that leaves its wait has each process
//! that it got in its inbox, and takes them before it runs a process.
//!
//! Only a runnable process moves: a process in the run queue that does not
//! wait, does not exit, is not in a dirty call, and holds nothing that stays
//! with its scheduler. A process that waits for a receive timer stays.

use std::sync::Arc;

use super::Scheduler;
use crate::runtime::process::{ProcessState, ReceiveTimer};

impl Scheduler {
    /// Give runnable processes to a hungry scheduler, at a scheduling step
    /// with no current process. A scheduler with one runnable process keeps
    /// it.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// node.set_hungry(1, true);
    /// scheduler.serve_steal();
    /// assert_eq!(node.owner(stolen), Some(1));
    /// ```
    pub(super) fn serve_steal(&mut self) {
        debug_assert!(self.processes.current_id().is_none());
        if self.runnable.len() < 2 || self.policy.is_some() || !self.node.has_hungry(self.index) {
            return;
        }
        let node = Arc::clone(&self.node);
        node.give_work(self.index, || self.take_stealable());
    }

    /// Take the processes of a steal out of the run queue and the slots
    /// ([`super::run_queue::RunQueue::steal`]).
    /// The first process runs next on the receiver, so it gives no count.
    fn take_stealable(&mut self) -> Vec<(ProcessState, Option<u8>)> {
        let processes = &self.processes;
        let system = &self.system;
        let stolen = self.runnable.steal(|id| {
            processes
                .get(id)
                .is_some_and(|process| !system.contains(&id) && stealable(process))
        });
        stolen
            .into_iter()
            .enumerate()
            .map(|(order, (id, low))| {
                let process = self
                    .processes
                    .remove(id)
                    .expect("a stolen process is in the slots");
                (process, low.filter(|_| order > 0))
            })
            .collect()
    }

    /// Put the process `process` that another scheduler gave this scheduler
    /// into its slots and its run queue, with the count `low` that a `low`
    /// process has left.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.adopt(process, None);
    /// assert!(scheduler.processes.contains_key(&id));
    /// ```
    pub(super) fn adopt(&mut self, process: ProcessState, low: Option<u8>) {
        debug_assert!(process.in_run_queue);
        let (id, priority) = (process.id, process.priority);
        self.processes.insert(process);
        self.runnable.push_stolen(id, priority, low);
    }
}

/// Return true when the process `process` of the run queue can move to
/// another scheduler.
fn stealable(process: &ProcessState) -> bool {
    !process.waiting
        && process.exiting.is_none()
        && process.dirty_call.is_none()
        && !matches!(process.receive_timer, ReceiveTimer::Set { .. })
}

#[cfg(test)]
#[path = "../../../tests/runtime/scheduler/steal.rs"]
mod tests;
