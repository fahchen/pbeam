use super::*;

/// The debug text of a resource has its magic reference and the number of
/// words of its data, and no type and no address.
#[test]
fn writes_the_reference_and_the_words_of_a_resource_in_the_debug_text() {
    // The registry keeps the object with its C reference, so no drop of
    // the object runs and nothing closes the handle, which opened no
    // library.
    let loading = Loading {
        _library: Arc::new(Handle(0)),
        node: Weak::new(),
    };
    let kind = Arc::new(ResourceType::new(None, Arc::new(AtomTable::new()), loading));
    let data = allocate(kind, 17);
    let term = term(data);
    let Some(Term::Resource(object)) = &term else {
        panic!("the object is alive while the registry keeps it");
    };
    assert_eq!(
        format!("{object:?}"),
        format!(
            "Resource {{ reference: {:?}, words: 2, .. }}",
            object.reference()
        )
    );
}
