use super::{NodeResources, Resources, lock};

#[derive(Debug, Default, PartialEq)]
struct MyStruct {
    value: i32,
}

#[derive(Debug, PartialEq)]
struct AnotherStruct {
    value: &'static str,
}

type Alias1 = String;
type Alias2 = String;

#[test]
fn put_and_borrow_mut_keep_one_value_of_each_type() {
    let mut resources = Resources::default();
    resources.put(MyStruct { value: 100 });
    resources.put(AnotherStruct { value: "a string" });
    resources.borrow_mut::<MyStruct>().value += 10;
    assert_eq!(resources.borrow_mut::<MyStruct>(), &MyStruct { value: 110 });
    assert_eq!(
        resources.borrow_mut::<AnotherStruct>(),
        &AnotherStruct { value: "a string" }
    );
}

#[test]
fn put_replaces_the_value_of_the_same_type() {
    let mut resources = Resources::default();
    resources.put::<Alias1>("alias1".to_string());
    resources.put::<Alias2>("alias2".to_string());
    assert_eq!(resources.borrow_mut::<Alias1>(), "alias2");
}

#[test]
#[should_panic(
    expected = "required type pbeam_runtime::runtime::resources::tests::MyStruct is not present in Resources"
)]
fn borrow_mut_panics_without_a_value_of_the_type() {
    let mut resources = Resources::default();
    resources.borrow_mut::<MyStruct>();
}

#[test]
fn debug_writes_no_values() {
    let mut resources = Resources::default();
    resources.put(MyStruct { value: 1 });
    assert_eq!(format!("{resources:?}"), "Resources { .. }");
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
