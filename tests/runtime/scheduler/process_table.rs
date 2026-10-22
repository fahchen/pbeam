use super::{BLOCK, DELETED_WORK, ProcessTable, SHARDS};
use crate::runtime::scheduler::PTAB_SIZE;
use crate::term::ProcessId;

#[test]
fn finds_the_scheduler_and_the_exiting_state_of_each_process() {
    let table = ProcessTable::default();
    table.insert(ProcessId(0), 0);
    table.insert(ProcessId(1), 1);
    assert_eq!(table.owner(ProcessId(1)), Some(1));
    assert_eq!(table.live_owner(ProcessId(1)), Some(1));
    assert_eq!(table.owner(ProcessId(2)), None);
    table.set_exiting(ProcessId(1));
    assert_eq!(table.owner(ProcessId(1)), Some(1));
    assert_eq!(table.live_owner(ProcessId(1)), None);
    assert_eq!(table.live_owner(ProcessId(0)), Some(0));
    assert!(table.hold(ProcessId(0)).is_some());
    assert!(table.hold(ProcessId(2)).is_none());
    table.remove(ProcessId(1), || {});
    assert_eq!(table.owner(ProcessId(1)), None);
}

#[test]
#[should_panic(expected = "each process has a new pid")]
fn does_not_insert_a_pid_twice() {
    let table = ProcessTable::default();
    table.insert(ProcessId(0), 0);
    table.insert(ProcessId(0), 1);
}

#[test]
#[should_panic(expected = "a process leaves the process table once")]
fn does_not_remove_a_process_twice() {
    let table = ProcessTable::default();
    table.insert(ProcessId(0), 0);
    table.remove(ProcessId(0), || {});
    table.remove(ProcessId(0), || {});
}

#[test]
#[should_panic(expected = "an exiting process is in the process table")]
fn does_not_mark_a_pid_with_no_process() {
    ProcessTable::default().set_exiting(ProcessId(0));
}

/// The table gives the processes of more than one block and more than one
/// shard in the index order of the ERTS table, also a pid whose index wraps
/// at the table size, and stops at the given number of kept processes.
#[test]
fn gives_the_processes_of_each_block_in_index_order() {
    let table = ProcessTable::default();
    let wrapped = PTAB_SIZE + 5;
    let far = BLOCK * SHARDS as u64 + 3;
    for id in [far, BLOCK + 1, wrapped, 5, BLOCK - 1, 0] {
        table.insert(ProcessId(id), 0);
    }
    let all = table.indexed(0, PTAB_SIZE, usize::MAX, |_| true);
    assert_eq!(
        all,
        [
            (0, ProcessId(0)),
            (5, ProcessId(5)),
            (5, ProcessId(wrapped)),
            (BLOCK - 1, ProcessId(BLOCK - 1)),
            (BLOCK + 1, ProcessId(BLOCK + 1)),
            (far, ProcessId(far)),
        ]
    );
    let kept = table.indexed(1, PTAB_SIZE, 2, |id| id != ProcessId(5));
    assert_eq!(
        kept,
        [(5, ProcessId(wrapped)), (BLOCK - 1, ProcessId(BLOCK - 1))]
    );
    assert_eq!(table.indexed(6, BLOCK - 1, 10, |_| true), []);
    table.start_list(ProcessId(0));
    assert_eq!(
        table.list_chunk(ProcessId(0), BLOCK + 2),
        [
            ProcessId(0),
            ProcessId(5),
            ProcessId(wrapped),
            ProcessId(BLOCK - 1),
            ProcessId(BLOCK + 1)
        ]
    );
    assert_eq!(table.list_chunk(ProcessId(0), PTAB_SIZE), [ProcessId(far)]);
}

/// The scheduler threads of a node insert, look up, and remove processes
/// at the same time, and each removal leaves the table once.
#[test]
fn shares_the_table_between_threads() {
    let table = ProcessTable::default();
    std::thread::scope(|scope| {
        for owner in 0..4u64 {
            let table = &table;
            scope.spawn(move || {
                for n in 0..2000 {
                    let id = ProcessId(n * 4 + owner);
                    table.insert(id, owner as usize);
                    assert_eq!(table.owner(id), Some(owner as usize));
                    if n % 2 == 0 {
                        table.remove(id, || {});
                    }
                }
            });
        }
    });
    assert_eq!(
        table.indexed(0, PTAB_SIZE, usize::MAX, |_| true).len(),
        4000
    );
}

/// Return the work that `step` does on the deleted list of `table`.
fn work<T>(table: &ProcessTable, step: impl FnOnce(&ProcessTable) -> T) -> (T, usize) {
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
    let table = ProcessTable::default();
    let readers = [ProcessId(0), ProcessId(1), ProcessId(2)];
    for id in (0..3).chain(1000..2000) {
        table.insert(ProcessId(id), 0);
    }
    let mut max = 0;
    let mut victims = 1000..2000;
    for (reader, exits) in readers.into_iter().zip([334, 333, 333]) {
        table.start_list(reader);
        assert_eq!(table.list_chunk(reader, 1000).len(), 3);
        for id in victims.by_ref().take(exits) {
            let ((), cost) = work(&table, |table| table.remove(ProcessId(id), || {}));
            max = max.max(cost);
        }
    }
    for reader in readers {
        assert_eq!(table.list_chunk(reader, PTAB_SIZE).len(), 0);
    }
    let (mut listed, mut frees) = ([0; 3], 0);
    let mut active = readers.to_vec();
    while !active.is_empty() {
        active.retain(|reader| {
            let (step, cost) = work(&table, |table| table.next_deleted(*reader));
            max = max.max(cost);
            match step {
                Some((entry, free)) => {
                    listed[reader.0 as usize] += usize::from(entry.is_some());
                    frees += usize::from(free);
                    true
                }
                None => {
                    assert!(table.deleted_done(*reader));
                    let ((), cost) = work(&table, |table| table.end_list(*reader));
                    max = max.max(cost);
                    false
                }
            }
        });
    }
    assert!(max <= 5, "a step did {max} units of work");
    assert_eq!(listed, [1000, 666, 333]);
    assert_eq!(frees, 1000);
    let listing = table.listing.read().unwrap();
    assert!(listing.deleted.is_empty());
    assert!(listing.positions.is_empty());
}

/// Assert that the exit of the oldest read leaves the entries that no read
/// can see, and that each later operation drops at most 2 of them. The
/// exit of a read with its own entry and 2 drops is 4 units of work, and
/// an exit with an entry and 2 drops is 3. An exit after the last read
/// ended drops the entries that remain.
#[test]
fn drops_the_entries_of_an_exited_read_in_bounded_steps() {
    let table = ProcessTable::default();
    for id in (0..2).chain(1000..1100).chain([5000]) {
        table.insert(ProcessId(id), 0);
    }
    table.start_list(ProcessId(0));
    assert_eq!(table.list_chunk(ProcessId(0), PTAB_SIZE).len(), 103);
    for id in 1000..1050 {
        table.remove(ProcessId(id), || {});
    }
    table.start_list(ProcessId(1));
    assert_eq!(table.list_chunk(ProcessId(1), PTAB_SIZE).len(), 53);
    let ((), cost) = work(&table, |table| table.remove(ProcessId(0), || {}));
    assert!(cost <= 4);
    assert_eq!(table.listing.read().unwrap().deleted.len(), 49);
    for id in 1050..1100 {
        let ((), cost) = work(&table, |table| table.remove(ProcessId(id), || {}));
        assert!(cost <= 3);
    }
    {
        let listing = table.listing.read().unwrap();
        assert_eq!(listing.deleted_base, 50);
        assert_eq!(listing.deleted.len(), 51);
    }
    assert_eq!(table.next_deleted(ProcessId(1)), Some((None, true)));
    table.end_list(ProcessId(1));
    let left = table.listing.read().unwrap().deleted.len();
    table.remove(ProcessId(5000), || {});
    assert_eq!(table.listing.read().unwrap().deleted.len(), left - 2);
}
