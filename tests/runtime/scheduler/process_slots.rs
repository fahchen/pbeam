use super::ProcessSlots;
use crate::{
    runtime::process::{ProcessState, Start, tests::frame},
    term::ProcessId,
};

fn process(id: u64) -> ProcessState {
    ProcessState::new(ProcessId(id), Start::Entry(frame()), vec![])
}

fn ids(table: &ProcessSlots) -> Vec<ProcessId> {
    let mut ids: Vec<ProcessId> = table.values().map(|process| process.id).collect();
    ids.sort_by_key(|id| id.0);
    ids
}

#[test]
fn finds_each_process_by_its_pid() {
    let mut table = ProcessSlots::default();
    assert!(table.is_empty());
    table.insert(process(0));
    table.insert(process(1));
    assert!(!table.is_empty());
    assert!(table.contains_key(ProcessId(1)));
    assert!(!table.contains_key(ProcessId(2)));
    assert_eq!(
        table.get(ProcessId(0)).map(|process| process.id),
        Some(ProcessId(0))
    );
    assert!(table.get(ProcessId(2)).is_none());
    table.get_mut(ProcessId(1)).unwrap().waiting = true;
    assert!(table[&ProcessId(1)].waiting);
    assert!(!table[&ProcessId(0)].waiting);
    assert!(table.get_mut(ProcessId(2)).is_none());
    assert_eq!(ids(&table), [ProcessId(0), ProcessId(1)]);
}

#[test]
fn uses_a_free_slot_again_for_a_new_process() {
    let mut table = ProcessSlots::default();
    table.insert(process(0));
    table.insert(process(1));
    assert_eq!(
        table.remove(ProcessId(0)).map(|process| process.id),
        Some(ProcessId(0))
    );
    assert!(table.remove(ProcessId(0)).is_none());
    assert!(!table.contains_key(ProcessId(0)));
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
    let mut table = ProcessSlots::default();
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
    let mut table = ProcessSlots::default();
    table.insert(process(0));
    table.set_current(ProcessId(0));
    table.take_current();
    table.remove(ProcessId(0));
    table.insert(process(1));
    assert_eq!(table.slots.len(), 1);
    assert_eq!(table.current_id(), None);
}

#[test]
#[should_panic(expected = "the current process switches out before it leaves the table")]
fn does_not_remove_the_current_process() {
    let mut table = ProcessSlots::default();
    table.insert(process(0));
    table.set_current(ProcessId(0));
    table.remove(ProcessId(0));
}

#[test]
#[should_panic(expected = "each process has a new pid")]
fn does_not_insert_a_pid_twice() {
    let mut table = ProcessSlots::default();
    table.insert(process(0));
    table.insert(process(0));
}

#[test]
#[should_panic(expected = "the process is alive")]
fn does_not_index_a_pid_with_no_process() {
    let table = ProcessSlots::default();
    let _ = &table[&ProcessId(0)];
}
