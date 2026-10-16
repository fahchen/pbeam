use super::*;

#[test]
fn closes_an_open_file_resource_when_its_owner_exits() {
    let mut files = Files::default();
    files.insert(Reference(0), File::open("/dev/null").unwrap());
    assert!(matches!(files.get_mut(Reference(0)), Some(Some(_))));
    files.monitor_down(Reference(0));
    assert!(matches!(files.get_mut(Reference(0)), Some(None)));
    // A resource that `close_nif/1` closed stays closed.
    files.monitor_down(Reference(0));
    assert!(matches!(files.get_mut(Reference(0)), Some(None)));
    assert!(files.get_mut(Reference(1)).is_none());
}
