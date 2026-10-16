use super::{DELETED_WORK, ProcessTable};
use crate::runtime::scheduler::PTAB_SIZE;
use crate::{
    runtime::process::{ProcessState, Start, tests::frame},
    term::ProcessId,
};

fn process(id: u64) -> ProcessState {
    ProcessState::new(ProcessId(id), Start::Entry(frame()), vec![])
}

fn ids(table: &ProcessTable) -> Vec<ProcessId> {
    let mut ids: Vec<ProcessId> = table.values().map(|process| process.id).collect();
    ids.sort_by_key(|id| id.0);
    ids
}

#[test]
fn finds_each_process_by_its_pid() {
    let mut table = ProcessTable::default();
    assert!(table.is_empty());
    table.insert(process(0));
    table.insert(process(1));
    assert!(!table.is_empty());
    assert!(table.contains_key(&ProcessId(1)));
    assert!(!table.contains_key(&ProcessId(2)));
    assert_eq!(
        table.get(&ProcessId(0)).map(|process| process.id),
        Some(ProcessId(0))
    );
    assert!(table.get(&ProcessId(2)).is_none());
    table.get_mut(&ProcessId(1)).unwrap().waiting = true;
    assert!(table[&ProcessId(1)].waiting);
    assert!(!table[&ProcessId(0)].waiting);
    assert!(table.get_mut(&ProcessId(2)).is_none());
    assert_eq!(ids(&table), [ProcessId(0), ProcessId(1)]);
}

#[test]
fn uses_a_free_slot_again_for_a_new_process() {
    let mut table = ProcessTable::default();
    table.insert(process(0));
    table.insert(process(1));
    assert_eq!(
        table.remove(&ProcessId(0)).map(|process| process.id),
        Some(ProcessId(0))
    );
    assert!(table.remove(&ProcessId(0)).is_none());
    assert!(!table.contains_key(&ProcessId(0)));
    table.insert(process(2));
    assert_eq!(table.slots.len(), 2);
    assert_eq!(
        table.slots[0].as_ref().map(|process| process.id),
        Some(ProcessId(2))
    );
    assert_eq!(table[&ProcessId(2)].id, ProcessId(2));
    assert_eq!(ids(&table), [ProcessId(1), ProcessId(2)]);
}

#[test]
fn reads_the_current_process_from_its_slot() {
    let mut table = ProcessTable::default();
    assert!(table.current().is_none());
    assert!(table.current_mut().is_none());
    assert!(table.take_current().is_none());
    assert_eq!(table.current_id(), None);
    table.insert(process(0));
    table.insert(process(1));
    table.set_current(ProcessId(1)).fcalls = 7;
    assert_eq!(table.current_id(), Some(ProcessId(1)));
    assert_eq!(table.current().map(|process| process.fcalls), Some(7));
    table.current_mut().unwrap().fcalls -= 1;
    assert_eq!(table[&ProcessId(1)].fcalls, 6);
    assert_eq!(
        table.take_current().map(|process| process.id),
        Some(ProcessId(1))
    );
    assert_eq!(table.current_id(), None);
    assert!(table.current().is_none());
}

#[test]
fn keeps_no_current_process_after_its_slot_has_a_new_process() {
    let mut table = ProcessTable::default();
    table.insert(process(0));
    table.set_current(ProcessId(0));
    table.take_current();
    table.remove(&ProcessId(0));
    table.insert(process(1));
    assert_eq!(table.slots.len(), 1);
    assert_eq!(table.current_id(), None);
}

#[test]
#[should_panic(expected = "the current process switches out before it leaves the table")]
fn does_not_remove_the_current_process() {
    let mut table = ProcessTable::default();
    table.insert(process(0));
    table.set_current(ProcessId(0));
    table.remove(&ProcessId(0));
}

#[test]
#[should_panic(expected = "each process has a new pid")]
fn does_not_insert_a_pid_twice() {
    let mut table = ProcessTable::default();
    table.insert(process(0));
    table.insert(process(0));
}

#[test]
#[should_panic(expected = "the process is alive")]
fn does_not_index_a_pid_with_no_process() {
    let table = ProcessTable::default();
    let _ = &table[&ProcessId(0)];
}

/// Return the work that `step` does on the deleted list of `table`.
fn work<T>(table: &mut ProcessTable, step: impl FnOnce(&mut ProcessTable) -> T) -> (T, usize) {
    DELETED_WORK.set(0);
    let value = step(table);
    (value, DELETED_WORK.get())
}

/// Assert that each operation on the deleted list does a fixed quantity of
/// work when 3 `erlang:processes/0` reads overlap and 1000 processes exit.
/// An exit compares no read, and a step of a read compares intervals and
/// reads the first position of the reads, as ERTS does
/// (`erl_ptab.c:643-672`, `1046-1170`). Each read lists the processes that
/// left the table after its start and before it read their chunk. Only the
/// oldest read frees an entry, so the frees are the exits.
#[test]
fn bounds_the_work_of_each_step_of_overlapping_reads() {
    let mut table = ProcessTable::default();
    let readers = [ProcessId(0), ProcessId(1), ProcessId(2)];
    for id in (0..3).chain(1000..2000) {
        table.insert(process(id));
    }
    let mut max = 0;
    let mut victims = 1000..2000;
    for (reader, exits) in readers.into_iter().zip([334, 333, 333]) {
        table.start_list(reader);
        assert_eq!(table.list_chunk(reader, 1000).count(), 3);
        for id in victims.by_ref().take(exits) {
            let (process, cost) = work(&mut table, |table| table.remove(&ProcessId(id)));
            assert!(process.is_some());
            max = max.max(cost);
        }
    }
    for reader in readers {
        assert_eq!(table.list_chunk(reader, PTAB_SIZE).count(), 0);
    }
    let (mut listed, mut frees) = ([0; 3], 0);
    let mut active = readers.to_vec();
    while !active.is_empty() {
        active.retain(|reader| {
            let (step, cost) = work(&mut table, |table| table.next_deleted(*reader));
            max = max.max(cost);
            match step {
                Some((entry, free)) => {
                    listed[reader.0 as usize] += usize::from(entry.is_some());
                    frees += usize::from(free);
                    true
                }
                None => {
                    assert!(table.deleted_done(*reader));
                    let ((), cost) = work(&mut table, |table| table.end_list(*reader));
                    max = max.max(cost);
                    false
                }
            }
        });
    }
    assert!(max <= 5, "a step did {max} units of work");
    assert_eq!(listed, [1000, 666, 333]);
    assert_eq!(frees, 1000);
    assert!(table.deleted.is_empty());
    assert!(table.positions.is_empty());
}

/// Assert that the exit of the oldest read leaves the entries that no read
/// can see, and that each later operation drops at most 2 of them. The
/// exit of a read with its own entry and 2 drops is 4 units of work, and
/// an exit with an entry and 2 drops is 3.
#[test]
fn drops_the_entries_of_an_exited_read_in_bounded_steps() {
    let mut table = ProcessTable::default();
    for id in (0..2).chain(1000..1100) {
        table.insert(process(id));
    }
    table.start_list(ProcessId(0));
    assert_eq!(table.list_chunk(ProcessId(0), PTAB_SIZE).count(), 102);
    for id in 1000..1050 {
        table.remove(&ProcessId(id));
    }
    table.start_list(ProcessId(1));
    assert_eq!(table.list_chunk(ProcessId(1), PTAB_SIZE).count(), 52);
    let ((), cost) = work(&mut table, |table| {
        table.remove(&ProcessId(0));
    });
    assert!(cost <= 4);
    assert_eq!(table.deleted.len(), 49);
    for id in 1050..1100 {
        let (_, cost) = work(&mut table, |table| table.remove(&ProcessId(id)));
        assert!(cost <= 3);
    }
    assert_eq!(table.deleted_base, 50);
    assert_eq!(table.deleted.len(), 51);
    assert_eq!(table.next_deleted(ProcessId(1)), Some((None, true)));
}
