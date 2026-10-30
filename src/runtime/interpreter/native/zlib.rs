//! Native bindings of the `zlib` NIFs
//! (`erts/emulator/nifs/common/zlib_nif.c`).
//!
//! The pinned `zlib` module is `erts/preloaded/src/zlib.erl` in OTP 29.1.1.
//! Its Erlang layer enqueues the input and dequeues the output in chunks, and
//! the NIFs compress and decompress one chunk at a time. PBeam keeps each
//! stream as a reference resource in [`ZlibStreams`], as it keeps a buffer of
//! `prim_buffer`. The mapping from the reference to the stream is the
//! `gc_zlib` resource type of ERTS (`zlib_nif.c:395-400`).
//!
//! The deflate stream runs in the pure-Rust `zlib-rs` implementation. Pinned
//! ERTS vendors zlib 1.3.2 (`erts/emulator/zlib/vendor.info`), and `zlib-rs`
//! does not promise the same compressed bytes as that C library. The
//! compressed bytes of the two libraries can differ for the same arguments;
//! `docs/erts-differences.md` records the difference. Each other observable
//! result, the stream state, the chunking, the exceptions, and the compressed
//! format follow `zlib_nif.c`.
//!
//! The functions `deflateSetDictionary_nif/2`,
//! `deflateParams_nif/3`, `inflateSetDictionary_nif/2`, and
//! `inflateGetDictionary_nif/1` have no binding. A call of one stops the run
//! with the runtime error of the static library (`Library::unbound_fails`).

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use zlib_rs::{
    Deflate, DeflateConfig, DeflateError, DeflateFlush, Inflate, InflateError, InflateFlush,
    Method, Status, Strategy,
};

use super::{BeamError, BifResult, Binding, ExceptionClass, Job, Scheduler, Term};
use crate::runtime::resources::lock;
use crate::term::{Binary, ProcessId, Reference};

/// The bytes of one byte, for the bit range of a [`Binary`].
const BITS_PER_BYTE: usize = 8;

/// The most elements that `enqueue_nif/2` takes from its iovec, as the
/// `max_elements` argument of `enif_inspect_iovec` (`zlib_nif.c:980`).
const ENQUEUE_MAX_ELEMENTS: usize = 256;

/// The functions of the ERTS `zlib` NIF library, as `(name, arity)` in the
/// order of `nif_funcs` (`zlib_nif.c:147-179`). `erlang:load_nif/2` checks
/// each of them in this order.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(FUNCTIONS[0], ("close_nif", 1));
/// assert_eq!(FUNCTIONS[18], ("enqueue_nif", 2));
/// ```
pub(super) const FUNCTIONS: [(&str, u32); 19] = [
    ("close_nif", 1),
    ("open_nif", 0),
    ("set_controller_nif", 2),
    ("deflateInit_nif", 6),
    ("deflateSetDictionary_nif", 2),
    ("deflateReset_nif", 1),
    ("deflateEnd_nif", 1),
    ("deflateParams_nif", 3),
    ("deflate_nif", 4),
    ("inflateInit_nif", 3),
    ("inflateSetDictionary_nif", 2),
    ("inflateGetDictionary_nif", 1),
    ("inflateReset_nif", 1),
    ("inflateEnd_nif", 1),
    ("inflate_nif", 4),
    ("getStash_nif", 1),
    ("clearStash_nif", 1),
    ("setStash_nif", 2),
    ("enqueue_nif", 2),
];

/// The registered bindings of `zlib`, sorted by name, then by arity.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!((BINDINGS[0].name, BINDINGS[0].arity), ("clearStash_nif", 1));
/// ```
pub(super) const BINDINGS: &[Binding] = &[
    Binding {
        name: "clearStash_nif",
        arity: 1,
        job: Job::Normal,
        run: |scheduler, arguments| clear_stash_nif(scheduler, &arguments[0]),
    },
    Binding {
        name: "close_nif",
        arity: 1,
        job: Job::Normal,
        run: |scheduler, arguments| close_nif(scheduler, &arguments[0]),
    },
    Binding {
        name: "deflateEnd_nif",
        arity: 1,
        job: Job::Normal,
        run: |scheduler, arguments| deflate_end_nif(scheduler, &arguments[0]),
    },
    Binding {
        name: "deflateInit_nif",
        arity: 6,
        job: Job::Normal,
        run: |scheduler, arguments| {
            deflate_init_nif(
                scheduler,
                &arguments[0],
                &arguments[1],
                &arguments[2],
                &arguments[3],
                &arguments[4],
                &arguments[5],
            )
        },
    },
    Binding {
        name: "deflateReset_nif",
        arity: 1,
        job: Job::Normal,
        run: |scheduler, arguments| deflate_reset_nif(scheduler, &arguments[0]),
    },
    Binding {
        name: "deflate_nif",
        arity: 4,
        job: Job::Normal,
        run: |scheduler, arguments| {
            deflate_nif(
                scheduler,
                &arguments[0],
                &arguments[1],
                &arguments[2],
                &arguments[3],
            )
        },
    },
    Binding {
        name: "enqueue_nif",
        arity: 2,
        job: Job::Normal,
        run: |scheduler, arguments| enqueue_nif(scheduler, &arguments[0], &arguments[1]),
    },
    Binding {
        name: "getStash_nif",
        arity: 1,
        job: Job::Normal,
        run: |scheduler, arguments| get_stash_nif(scheduler, &arguments[0]),
    },
    Binding {
        name: "inflateEnd_nif",
        arity: 1,
        job: Job::Normal,
        run: |scheduler, arguments| inflate_end_nif(scheduler, &arguments[0]),
    },
    Binding {
        name: "inflateInit_nif",
        arity: 3,
        job: Job::Normal,
        run: |scheduler, arguments| {
            inflate_init_nif(scheduler, &arguments[0], &arguments[1], &arguments[2])
        },
    },
    Binding {
        name: "inflateReset_nif",
        arity: 1,
        job: Job::Normal,
        run: |scheduler, arguments| inflate_reset_nif(scheduler, &arguments[0]),
    },
    Binding {
        name: "inflate_nif",
        arity: 4,
        job: Job::Normal,
        run: |scheduler, arguments| {
            inflate_nif(
                scheduler,
                &arguments[0],
                &arguments[1],
                &arguments[2],
                &arguments[3],
            )
        },
    },
    Binding {
        name: "open_nif",
        arity: 0,
        job: Job::Normal,
        run: |scheduler, _| open_nif(scheduler),
    },
    Binding {
        name: "setStash_nif",
        arity: 2,
        job: Job::Normal,
        run: |scheduler, arguments| set_stash_nif(scheduler, &arguments[0], &arguments[1]),
    },
    Binding {
        name: "set_controller_nif",
        arity: 2,
        job: Job::Normal,
        run: |scheduler, arguments| set_controller_nif(scheduler, &arguments[0], &arguments[1]),
    },
];

/// The `zlib` streams of the node. ERTS frees a stream resource when no term
/// refers to it. PBeam has no garbage collection of resources, so it keeps
/// each stream until the end of the run.
#[derive(Default)]
pub(super) struct ZlibStreams {
    /// Each stream by its reference, as the `zlib_data_t` objects of
    /// `zlib_open` (`zlib_nif.c:524-564`).
    by_reference: HashMap<Reference, Arc<Mutex<ZStream>>>,
}

impl ZlibStreams {
    /// Keep the stream of a new stream resource under the reference
    /// `reference`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut streams = ZlibStreams::default();
    /// streams.insert(Reference(0), ZStream::new(ProcessId(0)));
    /// ```
    fn insert(&mut self, reference: Reference, stream: ZStream) {
        self.by_reference
            .insert(reference, Arc::new(Mutex::new(stream)));
    }

    /// Return the stream of the stream resource `reference`, as
    /// `enif_get_resource` with the `gc_zlib` resource type does: `None` for
    /// a reference of another resource type or of `make_ref/0`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// streams.insert(Reference(0), ZStream::new(ProcessId(0)));
    /// assert!(streams.get(Reference(0)).is_some());
    /// assert!(streams.get(Reference(1)).is_none());
    /// ```
    fn get(&self, reference: Reference) -> Option<Arc<Mutex<ZStream>>> {
        self.by_reference.get(&reference).cloned()
    }
}

/// The stream state of a `zlib_data_t`, as `zlib_state_t`
/// (`zlib_nif.c:71-76`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    /// `ST_NONE`: `open/0` made the stream and no init call has started it.
    None,
    /// `ST_DEFLATE`.
    Deflate,
    /// `ST_INFLATE`.
    Inflate,
    /// `ST_CLOSED`: `close/1` ended the stream.
    Closed,
}

/// One stream of `zlib:open/0`, as the `zlib_data_t` of
/// `zlib_nif.c:92-119`. The caller controls the fields:
///
/// - `deflate` and `inflate` hold the compression state of `zlib-rs`, and
///   the operation of the state (`deflateInit_nif/6` or
///   `inflateInit_nif/3`) creates it. The drop of the value ends the zlib
///   stream, as `deflateEnd` or `inflateEnd` does.
/// - `queue` holds the input binaries that `enqueue_nif/2` put there, from
///   the front. The codec takes the bytes that it consumes from the front.
/// - `stash` holds the term of `setStash_nif/2`, which the Erlang layer uses
///   to keep the progress of a call that raises `need_dictionary`.
/// - `controller` is the process that `open_nif/0` recorded. Each other NIF
///   raises `not_on_controlling_process` for another process.
struct ZStream {
    /// The stream state (`zlib_nif.c:94`).
    state: State,
    /// The deflate state, when `state` is [`State::Deflate`].
    deflate: Option<Deflate>,
    /// The inflate state, when `state` is [`State::Inflate`].
    inflate: Option<Inflate>,
    /// The `windowBits` that `inflateInit_nif/3` got. `inflateReset` keeps
    /// the window size and wrapper of the stream (`inflate.c:136-171`), and
    /// PBeam makes the state again with these bits.
    inflate_window_bits: i32,
    /// The window size that the first header of an automatic wrapper told,
    /// for `inflate_window_bits` 0, 16 to 23, or 32 to 39. The C state
    /// keeps this size in `wbits` until the next init, also across
    /// `inflateReset` (`inflate.c:136-171`, `539-545`).
    inflate_learned_bits: Option<i32>,
    /// The first bytes of the header of an automatic wrapper, until it
    /// tells the window size. The C state reads the header across calls
    /// (`inflate.c:505-545`), and a header can arrive in parts.
    inflate_header: Vec<u8>,
    /// True after a `deflate_nif/4` call that passed its flush check. The
    /// `deflateEnd` of zlib gives `Z_DATA_ERROR` for a stream in the busy
    /// state, which is each state after such a call until the stream end
    /// (`deflate.c:961-989`).
    deflate_started: bool,
    /// True after the deflate codec returned `stream_end`. `deflateEnd`
    /// accepts a stream in the finish state (`deflate.c:961-989`).
    deflate_finished: bool,
    /// The end-of-stream behavior atom of `inflateInit/3`, as the integer
    /// that `arg_eos_behavior/1` gives (`zlib.erl:898-901`). `0` raises a
    /// data error, `1` resets the stream, and `2` cuts the input.
    eos_behavior: i32,
    /// True after the codec returned `stream_end` (`zlib_nif.c:423-425`).
    eos_seen: bool,
    /// The input binaries that wait for the codec, as the `input_queue` of
    /// `zlib_nif.c:115`. The queue of ERTS holds no empty binary.
    queue: VecDeque<Binary>,
    /// The term of the last `setStash_nif/2`, as `stash_env` and
    /// `stash_term` (`zlib_nif.c:117-118`). `None` is the `empty` result.
    stash: Option<Term>,
    /// The controlling process (`zlib_nif.c:112`).
    controller: ProcessId,
}

impl ZStream {
    /// Return a stream of `open_nif/0`: no state, no queue, no stash, and
    /// the controlling process `controller` (`zlib_nif.c:524-564`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let stream = ZStream::new(ProcessId(0));
    /// assert_eq!(stream.state, State::None);
    /// ```
    fn new(controller: ProcessId) -> Self {
        Self {
            state: State::None,
            deflate: None,
            inflate: None,
            inflate_window_bits: 0,
            inflate_learned_bits: None,
            inflate_header: Vec::new(),
            deflate_started: false,
            deflate_finished: false,
            eos_behavior: 2,
            eos_seen: false,
            queue: VecDeque::new(),
            stash: None,
            controller,
        }
    }

    /// End the zlib state and drop the queue and the stash, as
    /// `zlib_internal_close` and `zlib_reset_input` do
    /// (`zlib_nif.c:280-337`). The drop of the compression state calls
    /// `deflateEnd` or `inflateEnd`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut stream = ZStream::new(ProcessId(0));
    /// stream.close();
    /// assert_eq!(stream.state, State::Closed);
    /// ```
    fn close(&mut self) {
        self.deflate = None;
        self.inflate = None;
        self.reset_input();
        self.state = State::Closed;
    }

    /// Remove the queued input and the stash, as `zlib_reset_input`
    /// (`zlib_nif.c:328-337`) does.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut stream = ZStream::new(ProcessId(0));
    /// stream.queue.push_back(Binary::from(b"ab".to_vec()));
    /// stream.reset_input();
    /// assert!(stream.queue.is_empty());
    /// ```
    fn reset_input(&mut self) {
        self.queue.clear();
        self.stash = None;
    }

    /// Return the bytes of the queued input, as `enif_ioq_size`
    /// (`erl_nif.c:4363-4366`) gives them.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut stream = ZStream::new(ProcessId(0));
    /// stream.queue.push_back(Binary::from(b"ab".to_vec()));
    /// assert_eq!(stream.queue_bytes(), 2);
    /// ```
    fn queue_bytes(&self) -> usize {
        self.queue.iter().map(|binary| queued(binary).len()).sum()
    }

    /// Run one codec call of `zlib_flush_queue` (`zlib_nif.c:339-406`) on
    /// one input block and return the step, the consumed bytes, and the
    /// produced bytes. `flush` is the integer of `arg_flush/1`
    /// (`zlib.erl:875-880`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut stream = ZStream::new(ProcessId(0));
    /// stream.deflate = Some(Deflate::new_with_config(deflate_config(6, 8, 15, 8, 0).unwrap()));
    /// stream.state = State::Deflate;
    /// let (step, consumed, produced) = stream.step(b"ab", &mut [0; 64], 0);
    /// assert_eq!(step, Step::Ok);
    /// assert_eq!(consumed, 2);
    /// ```
    fn step(&mut self, input: &[u8], output: &mut [u8], flush: i32) -> (Step, usize, usize) {
        match self.state {
            State::Deflate => {
                let Ok(flush) = DeflateFlush::try_from(flush) else {
                    // `deflate` raises `stream_error` for a flush above
                    // `Z_BLOCK`, before it changes the stream state
                    // (`deflate.c:1260-1264`).
                    return (Step::Raise("stream_error"), 0, 0);
                };
                self.deflate_started = true;
                let deflate = self.deflate.as_mut().expect("the deflate state");
                let before_in = deflate.total_in();
                let before_out = deflate.total_out();
                let status = deflate.compress(input, output, flush);
                let consumed = to_usize(deflate.total_in() - before_in);
                let produced = to_usize(deflate.total_out() - before_out);
                if status == Ok(Status::StreamEnd) {
                    self.deflate_finished = true;
                }
                (status_step(status), consumed, produced)
            }
            State::Inflate => {
                let flush = match flush {
                    4 => InflateFlush::Finish,
                    5 => InflateFlush::Block,
                    6 => InflateFlush::Trees,
                    // `inflate` ignores each other flush (`inflate.c:1310-1317`).
                    _ => InflateFlush::NoFlush,
                };
                if self.inflate_learned_bits.is_none()
                    && self.inflate_window_bits >= 0
                    && self.inflate_window_bits.trailing_zeros() >= 4
                {
                    // The header can arrive in parts, so keep its first two
                    // bytes until the window size is known.
                    let missing = 2 - self.inflate_header.len();
                    let taken = input.len().min(missing);
                    self.inflate_header.extend_from_slice(&input[..taken]);
                    if self.inflate_header.len() == 2 {
                        self.inflate_learned_bits =
                            learn_window_bits(self.inflate_window_bits, &self.inflate_header);
                        self.inflate_header.clear();
                    }
                }
                let inflate = self.inflate.as_mut().expect("the inflate state");
                let before_in = inflate.total_in();
                let before_out = inflate.total_out();
                let status = inflate.decompress(input, output, flush);
                let consumed = to_usize(inflate.total_in() - before_in);
                let produced = to_usize(inflate.total_out() - before_out);
                (status_step(status), consumed, produced)
            }
            // The callers check the state before the codec call.
            State::None | State::Closed => (Step::Raise("stream_error"), 0, 0),
        }
    }
}

/// The result of one codec call, as the return code of `deflate` or
/// `inflate` after `zlib_return` (`zlib_nif.c:246-278`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Step {
    /// `Z_OK`.
    Ok,
    /// `Z_STREAM_END`.
    StreamEnd,
    /// `Z_NEED_DICT` with the Adler-32 of the dictionary.
    NeedDict(u32),
    /// `Z_BUF_ERROR`, which the codec treats as no progress.
    BufError,
    /// A negative code that raises an exception: `stream_error` or
    /// `data_error`.
    Raise(&'static str),
    /// A negative code that gives an atom: `mem_error`.
    Atom(&'static str),
}

/// Return the [`Step`] of a codec result.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(status_step(Ok(Status::Ok)), Step::Ok);
/// assert_eq!(status_step(Err(DeflateError::StreamError)), Step::Raise("stream_error"));
/// ```
fn status_step<T>(status: Result<Status, T>) -> Step
where
    T: Into<Step>,
{
    match status {
        Ok(Status::Ok) => Step::Ok,
        Ok(Status::StreamEnd) => Step::StreamEnd,
        Ok(Status::BufError) => Step::BufError,
        Err(error) => error.into(),
    }
}

impl From<DeflateError> for Step {
    /// Map the errors of `zlib-rs` deflate to the codes of `zlib_return`
    /// (`zlib_nif.c:246-278`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Step::from(DeflateError::DataError), Step::Raise("data_error"));
    /// assert_eq!(Step::from(DeflateError::MemError), Step::Atom("mem_error"));
    /// ```
    fn from(error: DeflateError) -> Self {
        match error {
            DeflateError::StreamError => Step::Raise("stream_error"),
            DeflateError::DataError => Step::Raise("data_error"),
            DeflateError::MemError => Step::Atom("mem_error"),
        }
    }
}

impl From<InflateError> for Step {
    /// Map the errors of `zlib-rs` inflate to the codes of `zlib_return`
    /// (`zlib_nif.c:246-278`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Step::from(InflateError::DataError), Step::Raise("data_error"));
    /// assert_eq!(
    ///     Step::from(InflateError::NeedDict { dict_id: 7 }),
    ///     Step::NeedDict(7)
    /// );
    /// ```
    fn from(error: InflateError) -> Self {
        match error {
            InflateError::NeedDict { dict_id } => Step::NeedDict(dict_id),
            InflateError::StreamError => Step::Raise("stream_error"),
            InflateError::DataError => Step::Raise("data_error"),
            InflateError::MemError => Step::Atom("mem_error"),
        }
    }
}

/// The result of one NIF after its checks, before the atoms of the result
/// exist in the atom table of the run.
#[derive(Debug, PartialEq)]
enum Native {
    /// An atom result, such as `ok` or `mem_error`.
    Atom(&'static str),
    /// The exception `error:Reason`.
    Raise(&'static str),
    /// `{finished, Output}`.
    Finished(Vec<u8>),
    /// `{continue, Output}`.
    Continue(Vec<u8>),
    /// `{need_dictionary, Adler, Output}`.
    NeedDictionary(u32, Vec<u8>),
    /// `{continue, Rest}` of `enqueue_nif/2`.
    ContinueInput(Term),
    /// `{ok, Term}` of `getStash_nif/1`.
    Stash(Term),
}

/// Build the result of a NIF from its [`Native`] value, as the NIF makes it
/// with `enif_make_atom`, `enif_make_tuple2`, and `enif_make_tuple3`.
///
/// # Examples
///
/// ```rust,ignore
/// let result = result(&mut scheduler, Native::Atom("ok"))?;
/// assert!(matches!(result, BifResult::Value(Term::Atom(_))));
/// ```
///
/// # Errors
///
/// A new result atom in a full atom table gives the error of
/// `AtomTable::put`.
fn result(scheduler: &mut Scheduler, native: Native) -> Result<BifResult, BeamError> {
    fn atom(scheduler: &mut Scheduler, name: &str) -> Result<Term, BeamError> {
        Ok(Term::Atom(scheduler.atoms.put(name)?))
    }
    /// Return the output list of a codec call: `[]` for no bytes, else a
    /// list of one binary (`zlib_nif.c:444-455`).
    fn output(bytes: Vec<u8>) -> Term {
        if bytes.is_empty() {
            Term::Nil
        } else {
            Term::list_from(vec![Term::binary(bytes)])
        }
    }
    Ok(match native {
        Native::Atom(name) => BifResult::Value(atom(scheduler, name)?),
        Native::Raise(name) => BifResult::Raise {
            class: ExceptionClass::Error,
            reason: atom(scheduler, name)?,
        },
        Native::Finished(bytes) => BifResult::Value(Term::tuple(vec![
            atom(scheduler, "finished")?,
            output(bytes),
        ])),
        Native::Continue(bytes) => BifResult::Value(Term::tuple(vec![
            atom(scheduler, "continue")?,
            output(bytes),
        ])),
        Native::NeedDictionary(adler, bytes) => BifResult::Value(Term::tuple(vec![
            atom(scheduler, "need_dictionary")?,
            Term::Integer(i64::from(adler)),
            output(bytes),
        ])),
        Native::ContinueInput(tail) => {
            BifResult::Value(Term::tuple(vec![atom(scheduler, "continue")?, tail]))
        }
        Native::Stash(term) => BifResult::Value(Term::tuple(vec![atom(scheduler, "ok")?, term])),
    })
}

/// Return the stream of a `Z` argument, as `get_zlib_data` does
/// (`zlib_nif.c:307-309`): `None` for a term that is not a zlib resource.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(stream_argument(&mut scheduler, &Term::nil()).is_none());
/// ```
fn stream_argument(scheduler: &Scheduler, term: &Term) -> Option<Arc<Mutex<ZStream>>> {
    let Term::Reference(reference) = term else {
        return None;
    };
    lock(&scheduler.node().resources.kind::<ZlibStreams>()).get(*reference)
}

/// Return the C `int` of an argument, as `enif_get_int` accepts it, and
/// `None` for each other term.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(integer(&Term::Integer(2)), Some(2));
/// assert_eq!(integer(&Term::Integer(i64::from(i32::MAX) + 1)), None);
/// ```
fn integer(term: &Term) -> Option<i32> {
    match term {
        Term::Integer(value) => i32::try_from(*value).ok(),
        _ => None,
    }
}

/// Return the `windowBits` of `inflateInit2` when it is valid, and `None`
/// when the C function returns `Z_STREAM_ERROR` (`inflate.c:136-171`).
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(inflate_window_bits(15), Some(15));
/// assert_eq!(inflate_window_bits(-15), Some(-15));
/// assert_eq!(inflate_window_bits(100), None);
/// ```
fn inflate_window_bits(window_bits: i32) -> Option<i32> {
    if window_bits < 0 {
        return (-15..=-8).contains(&window_bits).then_some(window_bits);
    }
    let window = if window_bits < 48 {
        window_bits & 15
    } else {
        return None;
    };
    (window == 0 || (8..=15).contains(&window)).then_some(window_bits)
}

/// Return the `DeflateConfig` of `deflateInit2` when the arguments are
/// valid, and `None` when the C function returns `Z_STREAM_ERROR`
/// (`deflate.c:387-440`). The validation has the order and the limits of the
/// C function: `-1` is the default level, a positive `WindowBits` above 15
/// adds the gzip wrapper, `WindowBits` 8 needs a zlib wrapper, and a method
/// other than `deflated` is invalid.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(deflate_config(6, 8, 15, 8, 0).is_some());
/// assert!(deflate_config(6, 8, -8, 8, 0).is_none());
/// assert!(deflate_config(10, 8, 15, 8, 0).is_none());
/// ```
fn deflate_config(
    level: i32,
    method: i32,
    window_bits: i32,
    mem_level: i32,
    strategy: i32,
) -> Option<DeflateConfig> {
    let level = if level == -1 { 6 } else { level };
    let (window, wrap) = if window_bits < 0 {
        if window_bits < -15 {
            return None;
        }
        (-window_bits, 0)
    } else if window_bits > 15 {
        (window_bits - 16, 2)
    } else {
        (window_bits, 1)
    };
    if !(1..=9).contains(&mem_level)
        || method != 8
        || !(8..=15).contains(&window)
        || !(0..=9).contains(&level)
        || !(0..=4).contains(&strategy)
        || (window == 8 && wrap != 1)
    {
        return None;
    }
    Some(DeflateConfig {
        level,
        method: Method::Deflated,
        window_bits,
        mem_level,
        strategy: Strategy::try_from(strategy).ok()?,
    })
}

/// Run `zlib:open/0`, as the ERTS NIF `open_nif`
/// (`zlib_nif.c:524-564`) does: a new stream with no state, an empty input
/// queue, no stash, and the caller as the controlling process.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(open_nif(&mut scheduler)?, BifResult::Value(Term::Reference(_))));
/// ```
///
/// # Errors
///
/// A full reference counter gives the error of `Scheduler::make_ref`.
fn open_nif(scheduler: &mut Scheduler) -> Result<BifResult, BeamError> {
    let reference = scheduler.make_magic_ref()?;
    let controller = scheduler.current_process().id;
    lock(&scheduler.node().resources.kind::<ZlibStreams>())
        .insert(reference, ZStream::new(controller));
    Ok(BifResult::Value(Term::Reference(reference)))
}

/// Run `zlib:close/1`, as the ERTS NIF `close_nif` (`zlib_nif.c:566-581`)
/// does: end the stream and give `ok`. A closed stream raises
/// `not_initialized`, a term that is not a zlib resource raises `badarg`,
/// and another process that controls the stream raises
/// `not_on_controlling_process`.
///
/// # Examples
///
/// ```rust,ignore
/// let stream = open_nif(&mut scheduler)?;
/// // ok
/// assert!(matches!(close_nif(&mut scheduler, &stream)?, BifResult::Value(_)));
/// assert!(matches!(close_nif(&mut scheduler, &Term::nil())?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// A new result atom in a full atom table gives the error of
/// `AtomTable::put`.
fn close_nif(scheduler: &mut Scheduler, stream: &Term) -> Result<BifResult, BeamError> {
    let current = scheduler.current_process().id;
    let Some(stream) = stream_argument(scheduler, stream) else {
        return Ok(BifResult::badarg());
    };
    let mut stream = lock(&stream);
    let native = if stream.controller != current {
        Native::Raise("not_on_controlling_process")
    } else if stream.state == State::Closed {
        Native::Raise("not_initialized")
    } else {
        stream.close();
        Native::Atom("ok")
    };
    result(scheduler, native)
}

/// Run `zlib:set_controlling_process/2`, as the ERTS NIF
/// `zlib_set_controller` (`zlib_nif.c:583-602`) does: make the local process
/// `pid` the controller of the stream and give `ok`. A term that is not a
/// zlib resource or not a local pid raises `badarg`, and another process
/// that controls the stream raises `not_on_controlling_process`. A closed
/// stream changes its controller too.
///
/// # Examples
///
/// ```rust,ignore
/// let stream = open_nif(&mut scheduler)?;
/// assert!(matches!(set_controller_nif(&mut scheduler, &stream, &Term::Pid(ProcessId(1)))?, BifResult::Value(_)));
/// ```
///
/// # Errors
///
/// A new result atom in a full atom table gives the error of
/// `AtomTable::put`.
fn set_controller_nif(
    scheduler: &mut Scheduler,
    stream: &Term,
    pid: &Term,
) -> Result<BifResult, BeamError> {
    let current = scheduler.current_process().id;
    let (Some(stream), Term::Pid(pid)) = (stream_argument(scheduler, stream), pid) else {
        return Ok(BifResult::badarg());
    };
    let mut stream = lock(&stream);
    let native = if stream.controller == current {
        stream.controller = *pid;
        Native::Atom("ok")
    } else {
        Native::Raise("not_on_controlling_process")
    };
    result(scheduler, native)
}

/// Run `zlib:deflateInit/6`, as the ERTS NIF `deflateInit_nif`
/// (`zlib_nif.c:606-639`) does. An argument that is not a C `int` raises
/// `badarg`, a started stream raises `already_initialized`, and invalid
/// compression parameters raise `stream_error`.
///
/// # Examples
///
/// ```rust,ignore
/// let stream = open_nif(&mut scheduler)?;
/// let ok = deflate_init_nif(&mut scheduler, &stream, &Term::Integer(6), &Term::Integer(8), &Term::Integer(15), &Term::Integer(8), &Term::Integer(0))?;
/// assert!(matches!(ok, BifResult::Value(_)));
/// ```
///
/// # Errors
///
/// A new result atom in a full atom table gives the error of
/// `AtomTable::put`.
fn deflate_init_nif(
    scheduler: &mut Scheduler,
    stream: &Term,
    level: &Term,
    method: &Term,
    window_bits: &Term,
    mem_level: &Term,
    strategy: &Term,
) -> Result<BifResult, BeamError> {
    let (Some(level), Some(method), Some(window_bits), Some(mem_level), Some(strategy)) = (
        integer(level),
        integer(method),
        integer(window_bits),
        integer(mem_level),
        integer(strategy),
    ) else {
        return Ok(BifResult::badarg());
    };
    let current = scheduler.current_process().id;
    let Some(stream) = stream_argument(scheduler, stream) else {
        return Ok(BifResult::badarg());
    };
    let mut stream = lock(&stream);
    let native = if stream.controller != current {
        Native::Raise("not_on_controlling_process")
    } else if stream.state != State::None {
        Native::Raise("already_initialized")
    } else {
        match deflate_config(level, method, window_bits, mem_level, strategy) {
            Some(config) => {
                stream.deflate = Some(Deflate::new_with_config(config));
                stream.state = State::Deflate;
                stream.deflate_started = false;
                stream.deflate_finished = false;
                stream.eos_seen = false;
                Native::Atom("ok")
            }
            None => Native::Raise("stream_error"),
        }
    };
    result(scheduler, native)
}

/// Run `zlib:deflateReset/1`, as the ERTS NIF `deflateReset_nif`
/// (`zlib_nif.c:670-690`) does: reset the stream with the same parameters,
/// drop the queued input and the stash, and give `ok`. A stream that is not
/// in deflate mode raises `not_initialized`.
///
/// # Examples
///
/// ```rust,ignore
/// let stream = open_nif(&mut scheduler)?;
/// assert!(matches!(deflate_reset_nif(&mut scheduler, &stream)?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// A new result atom in a full atom table gives the error of
/// `AtomTable::put`.
fn deflate_reset_nif(scheduler: &mut Scheduler, stream: &Term) -> Result<BifResult, BeamError> {
    let current = scheduler.current_process().id;
    let Some(stream) = stream_argument(scheduler, stream) else {
        return Ok(BifResult::badarg());
    };
    let mut stream = lock(&stream);
    let native = if stream.controller != current {
        Native::Raise("not_on_controlling_process")
    } else if stream.state != State::Deflate {
        Native::Raise("not_initialized")
    } else {
        stream.deflate.as_mut().expect("the deflate state").reset();
        stream.deflate_started = false;
        stream.deflate_finished = false;
        stream.eos_seen = false;
        stream.reset_input();
        Native::Atom("ok")
    };
    result(scheduler, native)
}

/// Run `zlib:deflateEnd/1`, as the ERTS NIF `deflateEnd_nif`
/// (`zlib_nif.c:692-714`) does: end the deflate state and give `ok`, also
/// when the stream had no input. Input that waits in the queue gives
/// `data_error`. A stream that is not in deflate mode raises
/// `not_initialized`.
///
/// # Examples
///
/// ```rust,ignore
/// let stream = open_nif(&mut scheduler)?;
/// assert!(matches!(deflate_end_nif(&mut scheduler, &stream)?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// A new result atom in a full atom table gives the error of
/// `AtomTable::put`.
fn deflate_end_nif(scheduler: &mut Scheduler, stream: &Term) -> Result<BifResult, BeamError> {
    let current = scheduler.current_process().id;
    let Some(stream) = stream_argument(scheduler, stream) else {
        return Ok(BifResult::badarg());
    };
    let mut stream = lock(&stream);
    let native = if stream.controller != current {
        Native::Raise("not_on_controlling_process")
    } else if stream.state != State::Deflate {
        Native::Raise("not_initialized")
    } else if stream.queue_bytes() > 0 || (stream.deflate_started && !stream.deflate_finished) {
        stream.deflate = None;
        stream.reset_input();
        stream.state = State::None;
        Native::Raise("data_error")
    } else {
        stream.deflate = None;
        stream.reset_input();
        stream.state = State::None;
        Native::Atom("ok")
    };
    result(scheduler, native)
}

/// Run `zlib:deflate/3`, as the ERTS NIF `deflate_nif`
/// (`zlib_nif.c:756-768`) does: compress the queued input and give
/// `{finished, Output}` or `{continue, Output}`. A stream that is not in
/// deflate mode raises `not_initialized`.
///
/// # Examples
///
/// ```rust,ignore
/// let stream = open_nif(&mut scheduler)?;
/// assert!(matches!(deflate_nif(&mut scheduler, &stream, &Term::Integer(1024), &Term::Integer(1024), &Term::Integer(0))?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// A new result atom in a full atom table gives the error of
/// `AtomTable::put`.
fn deflate_nif(
    scheduler: &mut Scheduler,
    stream: &Term,
    input_chunk_size: &Term,
    output_chunk_size: &Term,
    flush: &Term,
) -> Result<BifResult, BeamError> {
    codec_nif(
        scheduler,
        stream,
        input_chunk_size,
        output_chunk_size,
        flush,
        State::Deflate,
    )
}

/// Run `zlib:inflate/3` and `zlib:safeInflate/2`, as the ERTS NIF
/// `inflate_nif` (`zlib_nif.c:931-969`) does: decompress the queued input,
/// with the end-of-stream behavior of `inflateInit/3`. A stream that is not
/// in inflate mode raises `not_initialized`.
///
/// # Examples
///
/// ```rust,ignore
/// let stream = open_nif(&mut scheduler)?;
/// assert!(matches!(inflate_nif(&mut scheduler, &stream, &Term::Integer(1024), &Term::Integer(1024), &Term::Integer(0))?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// A new result atom in a full atom table gives the error of
/// `AtomTable::put`.
fn inflate_nif(
    scheduler: &mut Scheduler,
    stream: &Term,
    input_chunk_size: &Term,
    output_chunk_size: &Term,
    flush: &Term,
) -> Result<BifResult, BeamError> {
    let current = scheduler.current_process().id;
    let (Some(input_chunk_size), Some(output_chunk_size), Some(flush)) = (
        integer(input_chunk_size),
        integer(output_chunk_size),
        integer(flush),
    ) else {
        return Ok(BifResult::badarg());
    };
    let Some(stream) = stream_argument(scheduler, stream) else {
        return Ok(BifResult::badarg());
    };
    let mut stream = lock(&stream);
    let native = if stream.controller != current {
        Native::Raise("not_on_controlling_process")
    } else if stream.state != State::Inflate {
        Native::Raise("not_initialized")
    } else {
        let mut reset = false;
        let mut cut = false;
        let mut raise = false;
        if stream.eos_seen && stream.queue_bytes() > 0 {
            match stream.eos_behavior {
                0 => raise = true,
                1 => reset = true,
                2 => cut = true,
                // The C `switch` has no default arm (`zlib_nif.c:937-958`).
                _ => {}
            }
        }
        if raise {
            Native::Raise("data_error")
        } else {
            if reset {
                stream.inflate = Some(inflate_state(
                    stream.inflate_window_bits,
                    stream.inflate_learned_bits,
                ));
                stream.inflate_header.clear();
                stream.eos_seen = false;
            }
            if cut {
                stream.reset_input();
            }
            codec(&mut stream, input_chunk_size, output_chunk_size, flush)
        }
    };
    result(scheduler, native)
}

/// Run the codec of `deflate_nif/4` or `inflate_nif/4`: the argument checks
/// first, then the controlling process, then the stream state
/// (`zlib_nif.c:756-768`, `931-969`).
///
/// # Examples
///
/// ```rust,ignore
/// let stream = open_nif(&mut scheduler)?;
/// // `not_initialized`
/// let result = codec_nif(&mut scheduler, &stream, &Term::Integer(1), &Term::Integer(1), &Term::Integer(0), State::Deflate)?;
/// ```
///
/// # Errors
///
/// A new result atom in a full atom table gives the error of
/// `AtomTable::put`.
fn codec_nif(
    scheduler: &mut Scheduler,
    stream: &Term,
    input_chunk_size: &Term,
    output_chunk_size: &Term,
    flush: &Term,
    state: State,
) -> Result<BifResult, BeamError> {
    let current = scheduler.current_process().id;
    let (Some(input_chunk_size), Some(output_chunk_size), Some(flush)) = (
        integer(input_chunk_size),
        integer(output_chunk_size),
        integer(flush),
    ) else {
        return Ok(BifResult::badarg());
    };
    let Some(stream) = stream_argument(scheduler, stream) else {
        return Ok(BifResult::badarg());
    };
    let mut stream = lock(&stream);
    let native = if stream.controller != current {
        Native::Raise("not_on_controlling_process")
    } else if stream.state != state {
        Native::Raise("not_initialized")
    } else {
        codec(&mut stream, input_chunk_size, output_chunk_size, flush)
    };
    result(scheduler, native)
}

/// Convert the byte count of one call, which the length of a slice bounds,
/// to `usize`.
fn to_usize(count: u64) -> usize {
    usize::try_from(count).expect("the bytes of one call fit in usize")
}

/// Run one chunking call, as `zlib_codec` and `zlib_flush_queue`
/// (`zlib_nif.c:339-471`) do: compress or decompress the queued input up to
/// `input_chunk_size` bytes, write at most `output_chunk_size` bytes,
/// remove the consumed input, and give the result tuple.
///
/// # Examples
///
/// ```rust,ignore
/// let mut stream = ZStream::new(ProcessId(0));
/// stream.deflate = Some(Deflate::new_with_config(deflate_config(6, 8, 15, 8, 0).unwrap()));
/// stream.state = State::Deflate;
/// stream.queue.push_back(Binary::from(b"ab".to_vec()));
/// let native = codec(&mut stream, 1024, 1024, 4);
/// assert!(matches!(native, Native::Finished(_)));
/// ```
fn codec(
    stream: &mut ZStream,
    input_chunk_size: i32,
    output_chunk_size: i32,
    flush: i32,
) -> Native {
    // `enif_alloc_binary` gives `Z_MEM_ERROR` when it cannot allocate the
    // buffer (`zlib_nif.c:408-412`). A negative size is the large `size_t`
    // of the C conversion, so the allocation fails. PBeam has no allocation
    // failure path for a valid size, so it never gives `mem_error` for one.
    if output_chunk_size < 0 {
        return Native::Atom("mem_error");
    }
    let size = usize::try_from(output_chunk_size).expect("a negative size returned above");
    let mut output = vec![0u8; size];
    // A negative `InputChunkSize` becomes the large `size_t` of the C
    // conversion (`zlib_nif.c:339-345`).
    let input_limit = isize::try_from(input_chunk_size)
        .expect("an isize holds an i32 on every supported target")
        .cast_unsigned();
    let mut queue = std::mem::take(&mut stream.queue);
    let mut consumed = 0;
    let mut produced = 0;
    let mut step = Step::Ok;
    let mut index = 0;
    while step == Step::Ok && index < queue.len() && consumed < input_limit {
        let block = queued(&queue[index]);
        let block_size = block.len().min(input_limit);
        let (next, block_consumed, block_produced) =
            stream.step(&block[..block_size], &mut output[produced..], 0);
        consumed += block_consumed;
        produced += block_produced;
        step = next;
        index += 1;
    }
    deq(&mut queue, consumed);
    let remaining = queue_bytes(&queue);
    if step == Step::Ok && remaining == 0 && produced < size {
        // The input is gone and the output has room: flush the zlib state,
        // also with `none` as the flush (`zlib_nif.c:389-398`).
        let (next, _, flushed) = stream.step(&[], &mut output[produced..], flush);
        produced += flushed;
        step = next;
    }
    if let Some(native) = codec_failure(step) {
        stream.queue = queue;
        return native;
    }
    if step == Step::StreamEnd {
        stream.eos_seen = true;
    }
    stream.queue = queue;
    if consumed == 0 && produced == 0 && remaining != 0 {
        // No progress on input that remains (`zlib_nif.c:429-435`).
        return Native::Raise("data_error");
    }
    let bytes = output[..produced].to_vec();
    if remaining == 0 && produced < size {
        Native::Finished(bytes)
    } else if let Step::NeedDict(adler) = step {
        Native::NeedDictionary(adler, bytes)
    } else {
        Native::Continue(bytes)
    }
}

/// Return the result of a codec step that ends the call before the tuple,
/// as `zlib_return` does for a negative code (`zlib_nif.c:246-278`), and
/// `None` for a step that gives a tuple.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(codec_failure(Step::Raise("data_error")), Some(Native::Raise("data_error"))));
/// assert!(matches!(codec_failure(Step::Atom("mem_error")), Some(Native::Atom("mem_error"))));
/// assert!(codec_failure(Step::Ok).is_none());
/// ```
fn codec_failure(step: Step) -> Option<Native> {
    match step {
        Step::Raise(name) => Some(Native::Raise(name)),
        Step::Atom(name) => Some(Native::Atom(name)),
        Step::Ok | Step::StreamEnd | Step::NeedDict(_) | Step::BufError => None,
    }
}

/// Return the bytes that wait in `queue`.
///
/// # Examples
///
/// ```rust,ignore
/// let mut queue = VecDeque::new();
/// queue.push_back(Binary::from(b"ab".to_vec()));
/// assert_eq!(queue_bytes(&queue), 2);
/// ```
fn queue_bytes(queue: &VecDeque<Binary>) -> usize {
    queue.iter().map(|binary| queued(binary).len()).sum()
}

/// Return the bytes of a binary of the input queue. The enqueue takes only
/// binaries of whole bytes, as `enif_inspect_iovec` does.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(queued(&Binary::from(b"ab".to_vec())), b"ab");
/// ```
fn queued(binary: &Binary) -> &[u8] {
    binary
        .bytes()
        .expect("the input queue holds only binaries of whole bytes")
}

/// Remove the first `consumed` bytes of `queue`, as `enif_ioq_deq` does
/// (`erl_nif.c`, `erts_ioq_deq`). A part of the front binary keeps the bytes
/// that remain, as the moved `iov_base` of the head entry of ERTS gives
/// them.
///
/// # Examples
///
/// ```rust,ignore
/// let mut queue = VecDeque::from([Binary::from(b"abc".to_vec())]);
/// deq(&mut queue, 2);
/// assert_eq!(queue[0].bytes(), Some(&b"c"[..]));
/// ```
fn deq(queue: &mut VecDeque<Binary>, mut consumed: usize) {
    debug_assert!(consumed <= queue_bytes(queue));
    while consumed > 0 {
        let head = queue.front().expect("the queue holds the taken bytes");
        if queued(head).len() <= consumed {
            consumed -= queued(head).len();
            queue.pop_front();
        } else {
            let front = head.bits(consumed * BITS_PER_BYTE, head.bit_size());
            *queue.front_mut().expect("the queue holds the taken bytes") = front;
            consumed = 0;
        }
    }
}

/// Run `zlib:inflateInit/3`, as the ERTS NIF `inflateInit_nif`
/// (`zlib_nif.c:777-810`) does. An argument that is not a C `int` raises
/// `badarg`, a started stream raises `already_initialized`, and invalid
/// window bits raise `stream_error`.
///
/// # Examples
///
/// ```rust,ignore
/// let stream = open_nif(&mut scheduler)?;
/// let ok = inflate_init_nif(&mut scheduler, &stream, &Term::Integer(15), &Term::Integer(2))?;
/// assert!(matches!(ok, BifResult::Value(_)));
/// ```
///
/// # Errors
///
/// A new result atom in a full atom table gives the error of
/// `AtomTable::put`.
fn inflate_init_nif(
    scheduler: &mut Scheduler,
    stream: &Term,
    window_bits: &Term,
    eos_behavior: &Term,
) -> Result<BifResult, BeamError> {
    let (Some(window_bits), Some(eos_behavior)) = (integer(window_bits), integer(eos_behavior))
    else {
        return Ok(BifResult::badarg());
    };
    let current = scheduler.current_process().id;
    let Some(stream) = stream_argument(scheduler, stream) else {
        return Ok(BifResult::badarg());
    };
    let mut stream = lock(&stream);
    let native = if stream.controller != current {
        Native::Raise("not_on_controlling_process")
    } else if stream.state != State::None {
        Native::Raise("already_initialized")
    } else if inflate_window_bits(window_bits).is_none() {
        Native::Raise("stream_error")
    } else {
        stream.inflate = Some(inflate_state(window_bits, None));
        stream.inflate_window_bits = window_bits;
        stream.inflate_learned_bits = None;
        stream.inflate_header.clear();
        stream.state = State::Inflate;
        stream.eos_behavior = eos_behavior;
        stream.eos_seen = false;
        Native::Atom("ok")
    };
    result(scheduler, native)
}

/// Return the `zlib-rs` inflate state of `inflateInit2` with `window_bits`.
/// The negative bits select the raw stream, and the bits from 16 select the
/// gzip header, as `inflate.c:144-157` does. When the bits have no window
/// size, `learned` is the size that the first header told, and a reset
/// keeps it as the C `wbits` does (`inflate.c:136-171`, `539-545`).
///
/// # Examples
///
/// ```rust,ignore
/// let state = inflate_state(15, None);
/// let state = inflate_state(-15, None);
/// let state = inflate_state(32, Some(9));
/// ```
fn inflate_state(window_bits: i32, learned: Option<i32>) -> Inflate {
    if window_bits < 0 {
        return Inflate::new(false, (-window_bits).to_le_bytes()[0]);
    }
    let window = window_bits & 15;
    if window != 0 {
        return Inflate::new(true, window_bits.to_le_bytes()[0]);
    }
    let Some(bits) = learned else {
        return Inflate::new(true, window_bits.to_le_bytes()[0]);
    };
    let configured = if window_bits < 16 {
        bits
    } else if window_bits < 32 {
        16 + bits
    } else {
        32 + bits
    };
    Inflate::new(true, configured.to_le_bytes()[0])
}

/// Return the window size of the header of `input`, when `window_bits`
/// takes the size from the header (`inflate.c:505-545`), and `None` for a
/// fixed size or a header that does not name one. The C state keeps this
/// size in `wbits`, also across `inflateReset` (`inflate.c:136-171`).
///
/// # Examples
///
/// ```rust,ignore
/// // A zlib header with the window size 9: CINFO is 1.
/// assert_eq!(learn_window_bits(32, &[0x18, 0x95]), Some(9));
/// assert_eq!(learn_window_bits(32, &[0x1f, 0x8b]), Some(15));
/// assert_eq!(learn_window_bits(15, &[0x78, 0x9c]), None);
/// ```
fn learn_window_bits(window_bits: i32, input: &[u8]) -> Option<i32> {
    if window_bits < 0 || window_bits & 15 != 0 {
        return None;
    }
    if input.starts_with(&[0x1f, 0x8b]) {
        // A gzip header has no window size, and zlib uses 15
        // (`inflate.c:513-515`).
        return (window_bits >= 16).then_some(15);
    }
    if (16..32).contains(&window_bits) {
        // The gzip only wrapper rejects a zlib header.
        return None;
    }
    if input.len() < 2 || input[0] & 15 != 8 {
        return None;
    }
    let header = (u16::from(input[0]) << 8) | u16::from(input[1]);
    if header % 31 != 0 {
        return None;
    }
    // The high four bits of the first byte are the window size minus 8
    // (`inflate.c:539-545`). A value above 15 is an invalid header, and
    // zlib stores it before it raises `data_error`. Clamp it, so that a
    // reset after that error does not get an invalid zlib-rs window.
    Some((i32::from(input[0] >> 4) + 8).min(15))
}

/// Run `zlib:inflateReset/1`, as the ERTS NIF `inflateReset_nif`
/// (`zlib_nif.c:885-905`) does: reset the stream with the same window bits,
/// drop the queued input and the stash, and give `ok`. A stream that is not
/// in inflate mode raises `not_initialized`.
///
/// # Examples
///
/// ```rust,ignore
/// let stream = open_nif(&mut scheduler)?;
/// assert!(matches!(inflate_reset_nif(&mut scheduler, &stream)?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// A new result atom in a full atom table gives the error of
/// `AtomTable::put`.
fn inflate_reset_nif(scheduler: &mut Scheduler, stream: &Term) -> Result<BifResult, BeamError> {
    let current = scheduler.current_process().id;
    let Some(stream) = stream_argument(scheduler, stream) else {
        return Ok(BifResult::badarg());
    };
    let mut stream = lock(&stream);
    let native = if stream.controller != current {
        Native::Raise("not_on_controlling_process")
    } else if stream.state != State::Inflate {
        Native::Raise("not_initialized")
    } else {
        stream.inflate = Some(inflate_state(
            stream.inflate_window_bits,
            stream.inflate_learned_bits,
        ));
        stream.inflate_header.clear();
        stream.eos_seen = false;
        stream.reset_input();
        Native::Atom("ok")
    };
    result(scheduler, native)
}

/// Run `zlib:inflateEnd/1`, as the ERTS NIF `inflateEnd_nif`
/// (`zlib_nif.c:907-929`) does: end the inflate state and give `ok` when
/// the stream reached its end and the queue is empty, else `data_error`. A
/// stream that is not in inflate mode raises `not_initialized`.
///
/// # Examples
///
/// ```rust,ignore
/// let stream = open_nif(&mut scheduler)?;
/// assert!(matches!(inflate_end_nif(&mut scheduler, &stream)?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// A new result atom in a full atom table gives the error of
/// `AtomTable::put`.
fn inflate_end_nif(scheduler: &mut Scheduler, stream: &Term) -> Result<BifResult, BeamError> {
    let current = scheduler.current_process().id;
    let Some(stream) = stream_argument(scheduler, stream) else {
        return Ok(BifResult::badarg());
    };
    let mut stream = lock(&stream);
    let native = if stream.controller != current {
        Native::Raise("not_on_controlling_process")
    } else if stream.state != State::Inflate {
        Native::Raise("not_initialized")
    } else if !stream.eos_seen || stream.queue_bytes() > 0 {
        stream.inflate = None;
        stream.reset_input();
        stream.state = State::None;
        Native::Raise("data_error")
    } else {
        stream.inflate = None;
        stream.reset_input();
        stream.state = State::None;
        Native::Atom("ok")
    };
    result(scheduler, native)
}

/// Run `zlib:getStash` through its NIF, as `getStash_nif`
/// (`zlib_nif.c:473-487`) does: `empty` for no stash, else `{ok, Term}`.
/// The NIF does not check the stream state.
///
/// # Examples
///
/// ```rust,ignore
/// let stream = open_nif(&mut scheduler)?;
/// // empty
/// assert!(matches!(get_stash_nif(&mut scheduler, &stream)?, BifResult::Value(_)));
/// ```
///
/// # Errors
///
/// A new result atom in a full atom table gives the error of
/// `AtomTable::put`.
fn get_stash_nif(scheduler: &mut Scheduler, stream: &Term) -> Result<BifResult, BeamError> {
    let current = scheduler.current_process().id;
    let Some(stream) = stream_argument(scheduler, stream) else {
        return Ok(BifResult::badarg());
    };
    let stream = lock(&stream);
    let native = if stream.controller == current {
        match &stream.stash {
            Some(term) => Native::Stash(term.clone()),
            None => Native::Atom("empty"),
        }
    } else {
        Native::Raise("not_on_controlling_process")
    };
    result(scheduler, native)
}

/// Run `zlib:clearStash` through its NIF, as `clearStash_nif`
/// (`zlib_nif.c:489-505`) does: `ok` after it drops the stash, and the
/// exception `error` for no stash. The NIF does not check the stream state.
///
/// # Examples
///
/// ```rust,ignore
/// let stream = open_nif(&mut scheduler)?;
/// // `error`
/// assert!(matches!(clear_stash_nif(&mut scheduler, &stream)?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// A new result atom in a full atom table gives the error of
/// `AtomTable::put`.
fn clear_stash_nif(scheduler: &mut Scheduler, stream: &Term) -> Result<BifResult, BeamError> {
    let current = scheduler.current_process().id;
    let Some(stream) = stream_argument(scheduler, stream) else {
        return Ok(BifResult::badarg());
    };
    let mut stream = lock(&stream);
    let native = if stream.controller != current {
        Native::Raise("not_on_controlling_process")
    } else if stream.stash.is_none() {
        Native::Raise("error")
    } else {
        stream.stash = None;
        Native::Atom("ok")
    };
    result(scheduler, native)
}

/// Run `zlib:setStash` through its NIF, as `setStash_nif`
/// (`zlib_nif.c:507-522`) does: keep the term and give `ok`, and the
/// exception `error` when a stash exists. The NIF does not check the stream
/// state.
///
/// # Examples
///
/// ```rust,ignore
/// let stream = open_nif(&mut scheduler)?;
/// // ok
/// assert!(matches!(set_stash_nif(&mut scheduler, &stream, &Term::Integer(1))?, BifResult::Value(_)));
/// ```
///
/// # Errors
///
/// A new result atom in a full atom table gives the error of
/// `AtomTable::put`.
fn set_stash_nif(
    scheduler: &mut Scheduler,
    stream: &Term,
    term: &Term,
) -> Result<BifResult, BeamError> {
    let current = scheduler.current_process().id;
    let Some(stream) = stream_argument(scheduler, stream) else {
        return Ok(BifResult::badarg());
    };
    let mut stream = lock(&stream);
    let native = if stream.controller != current {
        Native::Raise("not_on_controlling_process")
    } else if stream.stash.is_some() {
        Native::Raise("error")
    } else {
        stream.stash = Some(term.clone());
        Native::Atom("ok")
    };
    result(scheduler, native)
}

/// Run `zlib:enqueue_input/2` through its NIF, as `enqueue_nif`
/// (`zlib_nif.c:971-996`) does: put the first
/// [`ENQUEUE_MAX_ELEMENTS`] binaries of the iovec into the input queue, and
/// give `ok`, or `{continue, Rest}` when the list is longer. A stream that
/// is not started raises `not_initialized`, and a term that is not a list of
/// binaries raises `badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// let stream = open_nif(&mut scheduler)?;
/// let iovec = Term::list_from(vec![Term::binary(b"ab".to_vec())]);
/// assert!(matches!(enqueue_nif(&mut scheduler, &stream, &iovec)?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// A new result atom in a full atom table gives the error of
/// `AtomTable::put`.
fn enqueue_nif(
    scheduler: &mut Scheduler,
    stream: &Term,
    iovec: &Term,
) -> Result<BifResult, BeamError> {
    let current = scheduler.current_process().id;
    let Some(stream) = stream_argument(scheduler, stream) else {
        return Ok(BifResult::badarg());
    };
    let mut stream = lock(&stream);
    let native = if stream.controller != current {
        Native::Raise("not_on_controlling_process")
    } else if stream.state != State::Deflate && stream.state != State::Inflate {
        Native::Raise("not_initialized")
    } else {
        // `enif_inspect_iovec` takes the first 256 elements and gives the
        // rest, and a list that is short or improper gives `badarg`
        // (`erl_nif.c:4107-4166`, `4342-4356`).
        let mut rest = iovec;
        let mut elements = 0;
        let mut binaries = Vec::new();
        let valid = loop {
            if elements >= ENQUEUE_MAX_ELEMENTS {
                break rest.is_list();
            }
            let Some((element, tail)) = rest.cons_cell() else {
                break rest.is_nil();
            };
            // `enif_inspect_iovec` rejects a bitstring (`erl_nif.c:4120-4121`).
            let (Term::Binary(binary), Some(bytes)) = (element, element.binary_bytes()) else {
                break false;
            };
            // `enqueue_input_1` gives no empty iovec entry
            // (`zlib.erl:920-926`), and `enif_ioq_enqv` adds none.
            if !bytes.is_empty() {
                binaries.push(binary.clone());
            }
            rest = tail;
            elements += 1;
        };
        if valid {
            stream.queue.extend(binaries);
            if rest.is_nil() {
                Native::Atom("ok")
            } else {
                Native::ContinueInput(rest.clone())
            }
        } else {
            Native::Raise("badarg")
        }
    };
    result(scheduler, native)
}

#[cfg(test)]
#[path = "../../../../tests/runtime/interpreter/native/zlib.rs"]
mod tests;
