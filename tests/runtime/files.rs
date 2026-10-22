use super::*;

#[test]
fn closes_an_open_file_resource_when_its_owner_exits() {
    let mut files = Files::default();
    files.insert(Reference(0), File::open("/dev/null").unwrap(), ProcessId(1));
    let data = files.get(Reference(0)).unwrap();
    files.monitor_down(Reference(0));
    assert!(data.acquire().is_none());
    // A resource that `close_nif/1` closed stays closed.
    files.monitor_down(Reference(0));
    assert!(data.acquire().is_none());
    assert!(matches!(files.close(Reference(0)), Some(None)));
    assert!(files.get(Reference(1)).is_none());
    assert!(files.close(Reference(1)).is_none());
    files.monitor_down(Reference(1));
}

#[test]
fn closes_a_busy_file_resource_at_the_end_of_its_operation() {
    let mut files = Files::default();
    files.insert(Reference(0), File::open("/dev/null").unwrap(), ProcessId(1));
    let data = files.get(Reference(0)).unwrap();
    // A busy resource gives no file to a second operation or to a close,
    // as `file_handle_wrapper` and `close_nif` give `{error, einval}`.
    let file = data.acquire().unwrap();
    assert!(data.acquire().is_none());
    assert!(matches!(files.close(Reference(0)), Some(None)));
    data.release(file);
    let file = data.acquire().unwrap();
    // The exit of the owner during the operation closes the resource at
    // the end of the operation.
    files.monitor_down(Reference(0));
    files.monitor_down(Reference(0));
    assert!(data.acquire().is_none());
    data.release(file);
    assert!(data.acquire().is_none());
    assert!(matches!(files.close(Reference(0)), Some(None)));
    // A close of an idle resource gives its file and its owner.
    files.insert(Reference(1), File::open("/dev/null").unwrap(), ProcessId(2));
    assert!(matches!(
        files.close(Reference(1)),
        Some(Some((_, ProcessId(2))))
    ));
}
