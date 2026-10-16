//! The buffer resources of `prim_buffer`, as the `buffer_data_t` objects of
//! `prim_buffer_nif.c` and their magic references (`erl_binary.h`). The
//! scheduler keeps them as the `Buffers` value of its `Resources`, and all
//! processes of a run share them. The pinned `prim_file` keeps the read
//! buffer of a file with a read mode (`prim_file.erl:516-517`) and reads,
//! writes, and searches it through the `prim_buffer` NIFs.

use std::{collections::HashMap, collections::VecDeque};

use crate::term::{Binary, Reference};

/// The most bytes that the accumulator holds before `write` puts it into
/// the queue, as `ACCUMULATOR_SIZE` (`prim_buffer_nif.c:34`).
const ACCUMULATOR_SIZE: usize = 2 << 10;

/// The bits of one byte, as the size of a [`Binary`] is a number of bits.
const BITS_PER_BYTE: usize = 8;

/// The buffer resources of the node. ERTS frees a buffer resource when no
/// term refers to it. PBeam has no garbage collection of resources, so it
/// keeps each buffer until the end of the run.
#[derive(Debug, Default)]
pub(super) struct Buffers {
    /// Each buffer by its reference, as the `buffer_data_t` objects of
    /// `new_nif` (`prim_buffer_nif.c:252-276`).
    by_reference: HashMap<Reference, Buffer>,
}

impl Buffers {
    /// Keep the buffer of a new buffer resource under the reference
    /// `reference`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut buffers = Buffers::default();
    /// buffers.insert(Reference(0), Buffer::default());
    /// ```
    pub(super) fn insert(&mut self, reference: Reference, buffer: Buffer) {
        self.by_reference.insert(reference, buffer);
    }

    /// Return the buffer of the buffer resource `reference`, as
    /// `enif_get_resource` with the `gc_buffer` resource type does: `None`
    /// for a reference of another resource type or of `make_ref/0`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// buffers.insert(Reference(0), Buffer::default());
    /// assert!(buffers.get_mut(Reference(0)).is_some());
    /// assert!(buffers.get_mut(Reference(1)).is_none());
    /// ```
    pub(super) fn get_mut(&mut self, reference: Reference) -> Option<&mut Buffer> {
        self.by_reference.get_mut(&reference)
    }
}

/// One buffer of `prim_buffer:new/0`, as the `buffer_data_t` of
/// `prim_buffer_nif.c:58-70`. ERTS holds a queued binary by reference count
/// and PBeam holds one shared [`Binary`], so neither copies one byte of it
/// and both release it with its last holder. The observable size, order, and
/// bytes are the same.
#[derive(Debug, Default)]
pub(super) struct Buffer {
    /// The queued binaries, from the front: each element is one binary that
    /// `enqueue_accumulator` or `write` put at the back of the queue
    /// (`erts_ioq_enqv`, `erl_io_queue.c:233-287`). `erts_ioq_enqv` gives
    /// one queue element for each iovec entry that holds bytes, so an empty
    /// binary is no element, and the accumulator is one element. An element
    /// of a read or a skip that takes a part of it holds the bytes that
    /// remain, as the head entry of ERTS does after it moves its `iov_base`
    /// (`erl_io_queue.c:361-369`).
    queue: VecDeque<Binary>,
    /// The bytes of `queue`, as the `size` of the IO queue of ERTS
    /// (`erts_ioq_size`, `erl_io_queue.c:161-169`): `write`, `deq`, and
    /// `enqueue_accumulator` update it, so `size/1` and the check of a read
    /// or a skip read no element.
    bytes: usize,
    /// The accumulator binary of `combine_small_writes`
    /// (`prim_buffer_nif.c:222-248`): the bytes of the small writes that
    /// are not in the queue yet, with the queue element that ERTS makes for
    /// them. `None` is no accumulator, as `accumulator_present == 0`; a
    /// binary with no bytes is the accumulator that ERTS keeps after a write
    /// of no bytes.
    accumulator: Option<Binary>,
    /// The external lock of `try_lock` and `unlock`
    /// (`prim_buffer_nif.c:489-515`), as `external_lock`. One lock holds
    /// each process of the run, as the one resource of the buffer does.
    locked: bool,
}

impl Buffer {
    /// Return the `size/1` of the buffer: the bytes of the queue and of the
    /// accumulator (`prim_buffer_nif.c:278-300`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut buffer = Buffer::default();
    /// assert_eq!(buffer.size(), 0);
    /// ```
    pub(super) fn size(&self) -> u64 {
        (self.queue_bytes() + self.accumulated_bytes()) as u64
    }

    /// Write the iovec `iovec` at the back of the buffer, as `write_nif`
    /// (`prim_buffer_nif.c:336-364`) does: the small writes of
    /// [`combine_small_writes`](Self::combine_small_writes) go into the
    /// accumulator, and each other iovec entry is one queue element
    /// (`erl_io_queue.c:233-287`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut buffer = Buffer::default();
    /// buffer.write(&[Binary::from(b"ab".to_vec())]);
    /// assert_eq!(buffer.size(), 2);
    /// ```
    pub(super) fn write(&mut self, iovec: &[Binary]) {
        if !self.combine_small_writes(iovec) {
            self.enqueue_accumulator();
            let elements = iovec.iter().filter(|binary| !binary.is_empty());
            self.bytes += elements.clone().map(|binary| binary.len()).sum::<usize>();
            self.queue.extend(elements.cloned());
        }
    }

    /// Return the first `size` bytes of the buffer and remove them, as
    /// `copying_read_nif` (`prim_buffer_nif.c:302-334`) does: `None` when
    /// the buffer holds fewer bytes, which raises `error:badarg`. ERTS
    /// copies the bytes into a new binary (`enif_make_new_binary`,
    /// prim_buffer_nif.c:324), and PBeam copies them into a `Vec`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut buffer = Buffer::default();
    /// buffer.write(&[Binary::from(b"abc".to_vec())]);
    /// // The read of ERTS gives a copy of the first two bytes.
    /// assert_eq!(buffer.copying_read(2), Some(b"ab".to_vec()));
    /// assert_eq!(buffer.size(), 1);
    /// ```
    pub(super) fn copying_read(&mut self, size: u64) -> Option<Vec<u8>> {
        self.enqueue_accumulator();
        if self.size() < size {
            return None;
        }
        let size = size as usize;
        let mut bytes = Vec::with_capacity(size);
        while bytes.len() < size {
            let head = self.queue.front().expect("the queue holds the bytes");
            let needed = size - bytes.len();
            bytes.extend_from_slice(&head[..needed.min(head.len())]);
            self.deq(needed.min(head.len()));
        }
        Some(bytes)
    }

    /// Return the bytes of the front of the queue that remain with no change
    /// to the buffer, as `peek_head_nif` (`prim_buffer_nif.c:366-388`) does:
    /// `None` for a queue of no bytes, which raises `error:badarg`
    /// (`enif_ioq_peek_head`, `erl_nif.c:4377-4383`). ERTS gives a
    /// sub-binary of the head entry, which shares its bytes
    /// (`erl_nif.c:4393-4407`), and PBeam gives a view of the queue element,
    /// so neither copies a byte.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut buffer = Buffer::default();
    /// buffer.write(&[Binary::from(b"ab".to_vec())]);
    /// assert_eq!(buffer.peek_head().as_deref(), Some(&b"ab"[..]));
    /// assert_eq!(buffer.size(), 2);
    /// ```
    pub(super) fn peek_head(&mut self) -> Option<Binary> {
        self.enqueue_accumulator();
        self.queue.front().cloned()
    }

    /// Remove the first `size` bytes of the buffer, as `skip_nif`
    /// (`prim_buffer_nif.c:390-413`) does. Return `false` when the buffer
    /// holds fewer bytes, which raises `error:badarg`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut buffer = Buffer::default();
    /// buffer.write(&[Binary::from(b"abc".to_vec())]);
    /// assert!(buffer.skip(2));
    /// assert_eq!(buffer.size(), 1);
    /// assert!(!buffer.skip(2));
    /// ```
    pub(super) fn skip(&mut self, size: u64) -> bool {
        self.enqueue_accumulator();
        if self.size() < size {
            return false;
        }
        self.deq(size as usize);
        true
    }

    /// Return the index of the first `needle` byte of the buffer, as
    /// `find_nif` (`prim_buffer_nif.c:415-485`) does: `None` when the
    /// buffer holds no such byte, which gives `not_found`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut buffer = Buffer::default();
    /// buffer.write(&[Binary::from(b"ab\n".to_vec())]);
    /// assert_eq!(buffer.find_byte_index(b'\n'), Some(2));
    /// assert_eq!(buffer.find_byte_index(b'x'), None);
    /// ```
    pub(super) fn find_byte_index(&mut self, needle: u8) -> Option<u64> {
        self.enqueue_accumulator();
        let mut offset = 0;
        for binary in &self.queue {
            if let Some(index) = binary.iter().position(|&byte| byte == needle) {
                return Some((offset + index) as u64);
            }
            offset += binary.len();
        }
        None
    }

    /// Take the external lock of the buffer, as `trylock_nif`
    /// (`prim_buffer_nif.c:489-501`) does: `false` when another holder has
    /// the lock, which gives `busy`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut buffer = Buffer::default();
    /// assert!(buffer.try_lock());
    /// assert!(!buffer.try_lock());
    /// ```
    pub(super) fn try_lock(&mut self) -> bool {
        let acquired = !self.locked;
        self.locked = true;
        acquired
    }

    /// Release the external lock of the buffer, as `unlock_nif`
    /// (`prim_buffer_nif.c:503-515`) does. Return `false` when the buffer
    /// is not locked, which raises `error:lock_order_violation`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut buffer = Buffer::default();
    /// buffer.try_lock();
    /// assert!(buffer.unlock());
    /// assert!(!buffer.unlock());
    /// ```
    pub(super) fn unlock(&mut self) -> bool {
        let released = self.locked;
        self.locked = false;
        released
    }

    /// Put the accumulator at the back of the queue with no other change,
    /// as the `enqueue_write_accumulator` call of `copying_read_nif`,
    /// `peek_head_nif`, `skip_nif`, and `find_nif`
    /// (`prim_buffer_nif.c:316`, `377`, `402`, `432`) does before the check
    /// of its own argument.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut buffer = Buffer::default();
    /// buffer.write(&[Binary::from(b"ab".to_vec())]);
    /// buffer.flush();
    /// assert_eq!(buffer.peek_head().as_deref(), Some(&b"ab"[..]));
    /// ```
    pub(super) fn flush(&mut self) {
        self.enqueue_accumulator();
    }

    /// Put the accumulator at the back of the queue, as
    /// `enqueue_write_accumulator` (`prim_buffer_nif.c:204-220`) does: a
    /// new queue element from the accumulator bytes, and no accumulator
    /// after the enqueue. An accumulator with no bytes adds no element.
    fn enqueue_accumulator(&mut self) {
        if let Some(accumulator) = self.accumulator.take()
            && !accumulator.is_empty()
        {
            self.bytes += accumulator.len();
            self.queue.push_back(accumulator);
        }
    }

    /// Accumulate the iovec `iovec` at the back of the accumulator, as
    /// `combine_small_writes` (`prim_buffer_nif.c:222-248`) does. Return
    /// `false` when the iovec goes into the queue as its own elements: its
    /// bytes are at least half of [`ACCUMULATOR_SIZE`], so the accumulator
    /// does not hold it.
    fn combine_small_writes(&mut self, iovec: &[Binary]) -> bool {
        let bytes: usize = iovec.iter().map(|binary| binary.len()).sum();
        if self.accumulated_bytes() + bytes >= ACCUMULATOR_SIZE {
            if bytes >= ACCUMULATOR_SIZE / 2 {
                return false;
            }
            self.enqueue_accumulator();
        }
        let mut written = Vec::with_capacity(bytes);
        for binary in iovec {
            written.extend_from_slice(binary);
        }
        self.accumulator = Some(match self.accumulator.take() {
            Some(accumulator) => Binary::concat(&accumulator, &written),
            None => Binary::from(written),
        });
        true
    }

    /// Remove the first `size` bytes at the front of the queue, as
    /// `erts_ioq_deq` (`erl_io_queue.c:347-380`) does: it removes the front
    /// element when the size takes its last byte, and the element of a
    /// smaller size keeps the bytes that remain, as the moved `iov_base` and
    /// the smaller `iov_len` of the head entry of ERTS give them
    /// (`erl_io_queue.c:361-369`).
    fn deq(&mut self, size: usize) {
        let mut remaining = size;
        while remaining > 0 {
            let head = self.queue.front().expect("the queue holds the bytes");
            if head.len() <= remaining {
                remaining -= head.len();
                self.bytes -= head.len();
                self.queue.pop_front();
            } else {
                let front = head.bits(remaining * BITS_PER_BYTE, head.bit_size());
                *self.queue.front_mut().expect("the queue holds the bytes") = front;
                self.bytes -= remaining;
                remaining = 0;
            }
        }
    }

    /// Return the bytes of the queue, as `enif_ioq_size`
    /// (`erl_nif.c:4363-4366`) gives them, from the count that
    /// [`write`](Self::write), [`deq`](Self::deq), and
    /// [`enqueue_accumulator`](Self::enqueue_accumulator) keep
    /// (`erts_ioq_size`, `erl_io_queue.c:161-169`).
    fn queue_bytes(&self) -> usize {
        self.bytes
    }

    /// Return the bytes of the accumulator, as `accumulated_bytes`
    /// (`prim_buffer_nif.c:64`).
    fn accumulated_bytes(&self) -> usize {
        self.accumulator.as_ref().map_or(0, |binary| binary.len())
    }
}

#[cfg(test)]
#[path = "../../tests/runtime/buffers.rs"]
mod tests;
