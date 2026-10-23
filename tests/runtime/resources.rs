use super::{NodeResources, lock};

#[derive(Debug, Default, PartialEq)]
struct MyStruct {
    value: i32,
}

#[test]
fn node_resources_give_one_lock_for_each_kind() {
    let resources = NodeResources::default();
    lock(&resources.kind::<MyStruct>()).value += 1;
    lock(&resources.kind::<MyStruct>()).value += 1;
    assert_eq!(*lock(&resources.kind::<MyStruct>()), MyStruct { value: 2 });
    assert_eq!(*lock(&resources.kind::<u8>()), 0);
    assert_eq!(format!("{resources:?}"), "NodeResources { .. }");
}

#[test]
fn node_resources_are_send_and_sync() {
    fn shared<T: Send + Sync>() {}
    shared::<NodeResources>();
}
