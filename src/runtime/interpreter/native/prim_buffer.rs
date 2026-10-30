//! Native bindings of the `prim_buffer` NIFs
//! (`erts/emulator/nifs/common/prim_buffer_nif.c`).
//!
//! A buffer resource of `new/0` is a reference to a `Buffer` in the
//! `Buffers` value of the node resources, as a file resource of `prim_file`
//! is. The pinned `prim_file` keeps the read buffer of a file with a read
//! mode in such a resource (`prim_file.erl:516-517`).
//!
//! ERTS runs `write/2` and `find_byte_index/2` of a long iovec or queue in
//! several calls of the same NIF (`enif_schedule_nif`,
//! `prim_buffer_nif.c:360`, `451`) and consumes timeslice
//! (`prim_buffer_nif.c:456`). PBeam runs one call. The observable result,
//! the size, the order, and the bytes of the buffer are the same.
//! `new/0` raises `out_of_memory` when ERTS cannot allocate the queue
//! (`prim_buffer_nif.c:270`). PBeam has no allocation failure path, so it
//! never raises that atom.

use std::sync::{Arc, Mutex};

use super::{BeamError, BifResult, Binding, ExceptionClass, Job, Scheduler, Term};
use crate::runtime::buffers::{Buffer, Buffers};
use crate::runtime::resources::lock;
use num_traits::ToPrimitive;

/// The functions of the ERTS `prim_buffer` NIF library, as `(name, arity)`
/// in the order of `nif_funcs` (`prim_buffer_nif.c:86-96`).
/// `erlang:load_nif/2` checks each of them in this order.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(FUNCTIONS[0], ("new", 0));
/// assert_eq!(FUNCTIONS[8], ("unlock", 1));
/// ```
pub(super) const FUNCTIONS: [(&str, u32); 9] = [
    ("new", 0),
    ("size", 1),
    ("peek_head", 1),
    ("copying_read", 2),
    ("write", 2),
    ("skip", 2),
    ("find_byte_index", 2),
    ("try_lock", 1),
    ("unlock", 1),
];

/// The registered bindings of `prim_buffer`, sorted by name, then by arity.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!((BINDINGS[0].name, BINDINGS[0].arity), ("copying_read", 2));
/// ```
pub(super) const BINDINGS: &[Binding] = &[
    Binding {
        name: "copying_read",
        arity: 2,
        job: Job::Normal,
        run: |scheduler, arguments| copying_read_nif(scheduler, &arguments[0], &arguments[1]),
    },
    Binding {
        name: "find_byte_index",
        arity: 2,
        job: Job::Normal,
        run: |scheduler, arguments| find_byte_index_nif(scheduler, &arguments[0], &arguments[1]),
    },
    Binding {
        name: "new",
        arity: 0,
        job: Job::Normal,
        run: |scheduler, _| new_nif(scheduler),
    },
    Binding {
        name: "peek_head",
        arity: 1,
        job: Job::Normal,
        run: |scheduler, arguments| peek_head_nif(scheduler, &arguments[0]),
    },
    Binding {
        name: "size",
        arity: 1,
        job: Job::Normal,
        run: |scheduler, arguments| size_nif(scheduler, &arguments[0]),
    },
    Binding {
        name: "skip",
        arity: 2,
        job: Job::Normal,
        run: |scheduler, arguments| skip_nif(scheduler, &arguments[0], &arguments[1]),
    },
    Binding {
        name: "try_lock",
        arity: 1,
        job: Job::Normal,
        run: |scheduler, arguments| try_lock_nif(scheduler, &arguments[0]),
    },
    Binding {
        name: "unlock",
        arity: 1,
        job: Job::Normal,
        run: |scheduler, arguments| unlock_nif(scheduler, &arguments[0]),
    },
    Binding {
        name: "write",
        arity: 2,
        job: Job::Normal,
        run: |scheduler, arguments| write_nif(scheduler, &arguments[0], &arguments[1]),
    },
];

/// The most elements of the iovec of one `write_nif/2` call, as the
/// `max_elements` argument of `enif_inspect_iovec` (`prim_buffer_nif.c:343`),
/// which is also the limit of `prim_file:write_nif/2`.
const WRITE_MAX_ELEMENTS: usize = 64;

/// Run `prim_buffer:new()`, as the ERTS NIF `new_nif`
/// (`prim_buffer_nif.c:252-276`) does: a new buffer resource with an empty
/// queue and no accumulator.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(new_nif(&mut scheduler)?, BifResult::Value(Term::Reference(_))));
/// ```
///
/// # Errors
///
/// A full reference counter gives the error of `Scheduler::make_ref`.
fn new_nif(scheduler: &mut Scheduler) -> Result<BifResult, BeamError> {
    let reference = scheduler.make_magic_ref()?;
    lock(&scheduler.node().resources.kind::<Buffers>()).insert(reference, Buffer::default());
    Ok(BifResult::Value(Term::Reference(reference)))
}

/// Run `prim_buffer:size(Buffer)`, as the ERTS NIF `size_nif`
/// (`prim_buffer_nif.c:278-300`) does: the bytes of the queue and of the
/// accumulator. A term that is not a buffer resource raises `error:badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// // 2
/// let size = size_nif(&mut scheduler, &buffer)?;
/// assert!(matches!(size_nif(&mut scheduler, &Term::nil())?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// A new result atom in a full atom table gives the error of
/// `AtomTable::put`.
#[expect(
    clippy::unnecessary_wraps,
    reason = "entry of the native function table, whose `run` returns `Result<BifResult, BeamError>`"
)]
fn size_nif(scheduler: &mut Scheduler, buffer: &Term) -> Result<BifResult, BeamError> {
    let Some(buffer) = buffer_argument(scheduler, buffer) else {
        return Ok(BifResult::badarg());
    };
    let buffer = lock(&buffer);
    Ok(BifResult::Value(Term::Integer(buffer.size().cast_signed())))
}

/// Run `prim_buffer:peek_head(Buffer)`, as the ERTS NIF `peek_head_nif`
/// (`prim_buffer_nif.c:366-388`) does: the binary at the front of the
/// queue, with no change to the buffer. ERTS gives a sub-binary of the head
/// entry above `ERL_ONHEAP_BINARY_LIMIT` of 64 bytes
/// (`erl_nif.c:4394-4415`) and copies the bytes of a head entry at or below
/// that size into an on-heap binary (`erl_nif.c:4417-4423`). PBeam gives a
/// view of the queue element, which copies no byte, and the bytes and the
/// order are the same. A term that is not a buffer resource, or a buffer
/// with an empty queue (`enif_ioq_peek_head`, `erl_nif.c:4377-4383`),
/// raises `error:badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// // <<"ab">>
/// let head = peek_head_nif(&mut scheduler, &buffer)?;
/// assert!(matches!(peek_head_nif(&mut scheduler, &Term::nil())?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// A new result atom in a full atom table gives the error of
/// `AtomTable::put`.
#[expect(
    clippy::unnecessary_wraps,
    reason = "entry of the native function table, whose `run` returns `Result<BifResult, BeamError>`"
)]
fn peek_head_nif(scheduler: &mut Scheduler, buffer: &Term) -> Result<BifResult, BeamError> {
    let Some(buffer) = buffer_argument(scheduler, buffer) else {
        return Ok(BifResult::badarg());
    };
    let mut buffer = lock(&buffer);
    match buffer.peek_head() {
        Some(head) => Ok(BifResult::Value(Term::Binary(head))),
        None => Ok(BifResult::badarg()),
    }
}

/// Run `prim_buffer:copying_read(Buffer, Size)`, as the ERTS NIF
/// `copying_read_nif` (`prim_buffer_nif.c:302-334`) does: a copy of the
/// first `Size` bytes of the buffer, which removes them. A term that is
/// not a buffer resource, a `Size` that is not a `u64` (`enif_get_uint64`),
/// and a `Size` above the bytes of the buffer raise `error:badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// // <<"ab">>
/// let read = copying_read_nif(&mut scheduler, &buffer, &Term::Integer(2))?;
/// assert!(matches!(copying_read_nif(&mut scheduler, &buffer, &Term::Integer(3))?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// A new result atom in a full atom table gives the error of
/// `AtomTable::put`.
#[expect(
    clippy::unnecessary_wraps,
    reason = "entry of the native function table, whose `run` returns `Result<BifResult, BeamError>`"
)]
fn copying_read_nif(
    scheduler: &mut Scheduler,
    buffer: &Term,
    size: &Term,
) -> Result<BifResult, BeamError> {
    let Some(size) = unsigned(size) else {
        return Ok(BifResult::badarg());
    };
    let Some(buffer) = buffer_argument(scheduler, buffer) else {
        return Ok(BifResult::badarg());
    };
    let mut buffer = lock(&buffer);
    match buffer.copying_read(size) {
        Some(bytes) => Ok(BifResult::Value(Term::binary(bytes))),
        None => Ok(BifResult::badarg()),
    }
}

/// Run `prim_buffer:write(Buffer, IOVec)`, as the ERTS NIF `write_nif`
/// (`prim_buffer_nif.c:336-364`) does: the small writes go into the
/// accumulator, and each other iovec entry is one queue element. A term
/// that is not a buffer resource raises `error:badarg`, and so does an
/// `IOVec` that is not a list of binaries, as `enif_inspect_iovec`
/// (`erl_nif.c:4321-4337`, `4107-4166`) checks: the first
/// [`WRITE_MAX_ELEMENTS`] elements of one call are binaries and are
/// followed by a list (see [`super::prim_file::write_nif`]). ERTS writes
/// the elements of a failed later call and raises for that call, and PBeam
/// does the same. The result is `ok`.
///
/// # Examples
///
/// ```rust,ignore
/// let iovec = Term::list_from(vec![Term::binary(b"ab".to_vec())]);
/// assert!(matches!(write_nif(&mut scheduler, &buffer, &iovec)?, BifResult::Value(Term::Atom(_))));
/// assert!(matches!(write_nif(&mut scheduler, &buffer, &Term::binary(vec![]))?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// A new result atom in a full atom table gives the error of
/// `AtomTable::put`.
fn write_nif(
    scheduler: &mut Scheduler,
    buffer: &Term,
    iovec: &Term,
) -> Result<BifResult, BeamError> {
    let Some(buffer) = buffer_argument(scheduler, buffer) else {
        return Ok(BifResult::badarg());
    };
    let mut buffer = lock(&buffer);
    let mut rest = iovec;
    loop {
        let mut elements = Vec::new();
        let mut count = 0;
        while let Some((element, tail)) = rest.cons_cell() {
            // `examine_iovec_term` rejects a bitstring (erl_nif.c:4120-4121).
            let (Term::Binary(binary), Some(bytes)) = (element, element.binary_bytes()) else {
                return Ok(BifResult::badarg());
            };
            // `examine_iovec_term` gives no iovec entry for an empty binary
            // (erl_nif.c:4123-4155).
            if !bytes.is_empty() {
                elements.push(binary.clone());
            }
            rest = tail;
            count += 1;
            if count >= WRITE_MAX_ELEMENTS {
                break;
            }
        }
        if !rest.is_list() {
            return Ok(BifResult::badarg());
        }
        buffer.write(&elements);
        if rest.is_nil() {
            return Ok(BifResult::Value(Term::static_atom("ok")));
        }
    }
}

/// Run `prim_buffer:skip(Buffer, Size)`, as the ERTS NIF `skip_nif`
/// (`prim_buffer_nif.c:390-413`) does: it removes the first `Size` bytes
/// of the buffer and gives `ok`. A term that is not a buffer resource, a
/// `Size` that is not a `u64` (`enif_get_uint64`), and a `Size` above the
/// bytes of the buffer raise `error:badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// let ok = skip_nif(&mut scheduler, &buffer, &Term::Integer(2))?;
/// assert!(matches!(skip_nif(&mut scheduler, &buffer, &Term::Integer(3))?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// A new result atom in a full atom table gives the error of
/// `AtomTable::put`.
#[expect(
    clippy::unnecessary_wraps,
    reason = "entry of the native function table, whose `run` returns `Result<BifResult, BeamError>`"
)]
fn skip_nif(scheduler: &mut Scheduler, buffer: &Term, size: &Term) -> Result<BifResult, BeamError> {
    let Some(size) = unsigned(size) else {
        return Ok(BifResult::badarg());
    };
    let Some(buffer) = buffer_argument(scheduler, buffer) else {
        return Ok(BifResult::badarg());
    };
    let mut buffer = lock(&buffer);
    if buffer.skip(size) {
        Ok(BifResult::Value(Term::static_atom("ok")))
    } else {
        Ok(BifResult::badarg())
    }
}

/// Run `prim_buffer:find_byte_index(Buffer, Needle)`, as the ERTS NIF
/// `find_nif` (`prim_buffer_nif.c:415-485`) does: `{ok, Index}` for the
/// index of the first `Needle` byte of the buffer, and `not_found` when
/// the buffer holds no such byte. A term that is not a buffer resource, a
/// `Needle` that is not a C `int` (`enif_get_int`), and a `Needle` outside
/// 0 to 255 raise `error:badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// // {ok, 2}
/// let found = find_byte_index_nif(&mut scheduler, &buffer, &Term::Integer(10))?;
/// // not_found
/// let missing = find_byte_index_nif(&mut scheduler, &buffer, &Term::Integer(120))?;
/// assert!(matches!(find_byte_index_nif(&mut scheduler, &buffer, &Term::Integer(256))?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// A new result atom in a full atom table gives the error of
/// `AtomTable::put`.
fn find_byte_index_nif(
    scheduler: &mut Scheduler,
    buffer: &Term,
    needle: &Term,
) -> Result<BifResult, BeamError> {
    let Some(needle) = needle
        .small_integer()
        .and_then(|value| i32::try_from(value).ok())
    else {
        return Ok(BifResult::badarg());
    };
    let Some(buffer) = buffer_argument(scheduler, buffer) else {
        return Ok(BifResult::badarg());
    };
    let mut buffer = lock(&buffer);
    // `find_nif` flushes the accumulator before it checks the range of the
    // needle (prim_buffer_nif.c:432-436).
    let Some(needle) = u8::try_from(needle).ok() else {
        buffer.flush();
        return Ok(BifResult::badarg());
    };
    let found = buffer.find_byte_index(needle);
    match found {
        Some(index) => Ok(BifResult::Value(Term::tuple(vec![
            Term::static_atom("ok"),
            Term::Integer(index.cast_signed()),
        ]))),
        None => Ok(BifResult::Value(Term::Atom(
            scheduler.atoms.put("not_found")?,
        ))),
    }
}

/// Run `prim_buffer:try_lock(Buffer)`, as the ERTS NIF `trylock_nif`
/// (`prim_buffer_nif.c:489-501`) does: `acquired` for the call that takes
/// the external lock, and `busy` for each other call. A term that is not a
/// buffer resource raises `error:badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// // acquired
/// let first = try_lock_nif(&mut scheduler, &buffer)?;
/// // busy
/// let second = try_lock_nif(&mut scheduler, &buffer)?;
/// ```
///
/// # Errors
///
/// A new result atom in a full atom table gives the error of
/// `AtomTable::put`.
fn try_lock_nif(scheduler: &mut Scheduler, buffer: &Term) -> Result<BifResult, BeamError> {
    let Some(buffer) = buffer_argument(scheduler, buffer) else {
        return Ok(BifResult::badarg());
    };
    let mut buffer = lock(&buffer);
    let name = if buffer.try_lock() {
        "acquired"
    } else {
        "busy"
    };
    Ok(BifResult::Value(Term::Atom(scheduler.atoms.put(name)?)))
}

/// Run `prim_buffer:unlock(Buffer)`, as the ERTS NIF `unlock_nif`
/// (`prim_buffer_nif.c:503-515`) does: `ok` when the buffer holds the
/// external lock, and else the exception `error:lock_order_violation`. A
/// term that is not a buffer resource raises `error:badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// try_lock_nif(&mut scheduler, &buffer)?;
/// // ok
/// let released = unlock_nif(&mut scheduler, &buffer)?;
/// assert!(matches!(unlock_nif(&mut scheduler, &buffer)?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// A new result atom in a full atom table gives the error of
/// `AtomTable::put`.
fn unlock_nif(scheduler: &mut Scheduler, buffer: &Term) -> Result<BifResult, BeamError> {
    let Some(buffer) = buffer_argument(scheduler, buffer) else {
        return Ok(BifResult::badarg());
    };
    let mut buffer = lock(&buffer);
    if buffer.unlock() {
        return Ok(BifResult::Value(Term::static_atom("ok")));
    }
    let reason = scheduler.atoms.put("lock_order_violation")?;
    Ok(BifResult::Raise {
        class: ExceptionClass::Error,
        reason: Term::Atom(reason),
    })
}

/// Return the buffer of a `Buffer` argument, as `get_buffer_data` with the
/// `gc_buffer` resource type does (`prim_buffer_nif.c:156-158`): `None`
/// for a term that is not a buffer resource.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(buffer_argument(&mut scheduler, &buffer).is_some());
/// assert!(buffer_argument(&mut scheduler, &Term::nil()).is_none());
/// ```
fn buffer_argument(scheduler: &Scheduler, buffer: &Term) -> Option<Arc<Mutex<Buffer>>> {
    let Term::Reference(reference) = buffer else {
        return None;
    };
    lock(&scheduler.node().resources.kind::<Buffers>()).get(*reference)
}

/// Return the `u64` of a `Size` argument, as `enif_get_uint64` accepts it:
/// an integer from 0 to `u64::MAX`, a bignum in that range included, and
/// `None` for each other term, which raises `error:badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(unsigned(&Term::Integer(2)), Some(2));
/// assert_eq!(unsigned(&Term::Integer(-1)), None);
/// assert_eq!(unsigned(&Term::static_atom("ok")), None);
/// ```
fn unsigned(size: &Term) -> Option<u64> {
    match size {
        Term::Integer(value) => u64::try_from(*value).ok(),
        Term::BigInt(_) => size.bigint()?.to_u64(),
        _ => None,
    }
}

#[cfg(test)]
#[path = "../../../../tests/runtime/interpreter/native/prim_buffer.rs"]
mod tests;
