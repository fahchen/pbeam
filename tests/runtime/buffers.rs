use super::*;

/// Return the binary of the bytes `bytes`, one element of an iovec of
/// `write/2`.
fn binary(bytes: &[u8]) -> Binary {
    Binary::from(bytes.to_vec())
}

/// Return the bytes of the front of the queue of `buffer`, or `None` for a
/// queue of no bytes.
fn head(buffer: &mut Buffer) -> Option<Vec<u8>> {
    buffer.peek_head().map(|binary| binary.to_bytes())
}

#[test]
fn keeps_the_bytes_and_the_order_of_the_queue() {
    let mut buffer = Buffer::default();
    assert_eq!(buffer.size(), 0);
    // Two small writes accumulate into one queue element, as the ERTS
    // accumulator does.
    buffer.write(&[binary(b"ab")]);
    buffer.write(&[binary(b"cd")]);
    assert_eq!(buffer.size(), 4);
    assert_eq!(head(&mut buffer), Some(b"abcd".to_vec()));
    assert_eq!(buffer.size(), 4);
    // An empty binary is no queue element.
    buffer.write(&[binary(b""), binary(b"ef")]);
    assert_eq!(buffer.size(), 6);
    // The copy of the first bytes and the skip remove them from the front.
    assert_eq!(buffer.copying_read(2), Some(b"ab".to_vec()));
    assert_eq!(buffer.size(), 4);
    assert!(buffer.skip(2));
    assert_eq!(buffer.size(), 2);
    // The reads of 0 bytes and of no bytes change nothing.
    assert_eq!(buffer.copying_read(0), Some(Vec::new()));
    buffer.write(&[]);
    assert_eq!(buffer.size(), 2);
    assert_eq!(buffer.copying_read(2), Some(b"ef".to_vec()));
    assert_eq!(buffer.copying_read(1), None);
    assert!(!buffer.skip(1));
    assert_eq!(head(&mut buffer), None);
}

#[test]
fn accumulates_the_writes_of_the_buffer() {
    let mut buffer = Buffer::default();
    // A small write accumulates. The writes of 2000 and of 100 bytes cross
    // ACCUMULATOR_SIZE, so the first becomes one queue element and the
    // second starts a new accumulator.
    let two_thousand = vec![b'a'; 2000];
    let hundred = vec![b'b'; 100];
    buffer.write(&[binary(&two_thousand)]);
    buffer.write(&[binary(&hundred)]);
    assert_eq!(buffer.size(), 2100);
    assert_eq!(head(&mut buffer), Some(two_thousand));
    assert_eq!(buffer.size(), 2100);
    // A write of 3000 bytes is at least half of ACCUMULATOR_SIZE, so it
    // goes into the queue as its own element with no accumulation.
    let three_thousand = vec![b'c'; 3000];
    buffer.write(&[binary(b""), binary(&three_thousand), binary(b"de")]);
    assert_eq!(buffer.size(), 5102);
    assert!(buffer.skip(2000));
    assert_eq!(head(&mut buffer), Some(hundred));
    assert!(buffer.skip(100));
    assert_eq!(head(&mut buffer), Some(three_thousand));
    assert!(buffer.skip(3000));
    assert_eq!(head(&mut buffer), Some(b"de".to_vec()));
}

#[test]
fn reads_and_removes_bytes_across_the_queue_elements() {
    let mut buffer = Buffer::default();
    let first = vec![b'a'; 3000];
    let second = vec![b'b'; 3000];
    buffer.write(&[binary(&first)]);
    buffer.write(&[binary(&second)]);
    assert_eq!(buffer.size(), 6000);
    assert_eq!(
        buffer.copying_read(4000),
        Some([first, vec![b'b'; 1000]].concat())
    );
    assert_eq!(buffer.size(), 2000);
    assert!(buffer.skip(1000));
    assert_eq!(buffer.size(), 1000);
    assert!(!buffer.skip(1001));
    assert_eq!(buffer.size(), 1000);
    assert_eq!(buffer.copying_read(1000), Some(vec![b'b'; 1000]));
    assert_eq!(buffer.size(), 0);
}

#[test]
fn finds_a_byte_in_the_queue_of_the_buffer() {
    let mut buffer = Buffer::default();
    buffer.flush();
    assert_eq!(buffer.find_byte_index(b'\n'), None);
    buffer.write(&[binary(b"ab\n"), binary(b"c\nd")]);
    assert_eq!(buffer.find_byte_index(b'\n'), Some(2));
    assert_eq!(buffer.find_byte_index(b'd'), Some(5));
    assert_eq!(buffer.find_byte_index(b'x'), None);
}

#[test]
fn finds_a_byte_after_a_partial_read_and_across_the_queue() {
    let mut buffer = Buffer::default();
    buffer.write(&[binary(b"ab\n")]);
    // The search starts at the bytes that remain: the front element holds
    // "b\n" after the read of one byte, so the search finds the newline at 1.
    assert_eq!(buffer.copying_read(1), Some(b"a".to_vec()));
    assert_eq!(head(&mut buffer), Some(b"b\n".to_vec()));
    assert_eq!(buffer.find_byte_index(b'\n'), Some(1));
    assert!(buffer.skip(2));
    // The search of an element other than the front one counts the bytes of
    // each element before it (prim_buffer_nif.c:460-480).
    let first = vec![b'a'; 2000];
    let second = vec![b'x'; 2000];
    buffer.write(&[binary(&first)]);
    buffer.write(&[binary(&second)]);
    assert_eq!(buffer.size(), 4000);
    assert_eq!(buffer.find_byte_index(b'a'), Some(0));
    assert_eq!(buffer.find_byte_index(b'x'), Some(2000));
}

#[test]
fn flushes_the_accumulator_into_the_queue() {
    let mut buffer = Buffer::default();
    buffer.write(&[binary(b"ab")]);
    buffer.flush();
    // The flushed accumulator is one queue element, also when a later
    // write adds bytes to a new accumulator.
    buffer.write(&[binary(b"cd")]);
    buffer.flush();
    assert_eq!(buffer.size(), 4);
    assert_eq!(head(&mut buffer), Some(b"ab".to_vec()));
    assert!(buffer.skip(2));
    assert_eq!(head(&mut buffer), Some(b"cd".to_vec()));
}

#[test]
fn takes_and_releases_the_external_lock() {
    let mut buffer = Buffer::default();
    assert!(!buffer.unlock());
    assert!(buffer.try_lock());
    assert!(!buffer.try_lock());
    assert!(buffer.unlock());
    assert!(!buffer.unlock());
}

#[test]
fn keeps_each_buffer_of_the_node_by_its_reference() {
    let mut buffers = Buffers::default();
    assert!(buffers.get(Reference(0)).is_none());
    buffers.insert(Reference(0), Buffer::default());
    buffers.insert(Reference(1), Buffer::default());
    assert!(buffers.get(Reference(0)).is_some());
    assert!(buffers.get(Reference(1)).is_some());
    assert!(buffers.get(Reference(2)).is_none());
    buffers
        .get(Reference(0))
        .unwrap()
        .lock()
        .unwrap()
        .write(&[binary(b"ab")]);
    assert_eq!(buffers.get(Reference(0)).unwrap().lock().unwrap().size(), 2);
    assert_eq!(buffers.get(Reference(1)).unwrap().lock().unwrap().size(), 0);
}
