//! Parse Code chunk instructions into the runtime instruction model.

use super::{
    COMPACT_EXTENDED_FLAG, COMPACT_SHORT_VALUE_SHIFT, COMPACT_SIGN_FLAG, COMPACT_TAG_MASK,
    FIRST_LABEL_NUMBER, LIST_SUBTAG, LineTable, TAG_ATOM, TAG_INTEGER, TAG_LABEL, TAG_LITERAL,
    TAG_REGISTER, TAG_UNSIGNED, TAG_Y_REGISTER, WORD_SIZE, atom_at, read_integer, read_operand,
    read_u32, read_value,
};
use crate::error::{BeamError, error};
use crate::runtime::{BifId, resolve_bif};
use crate::term::{Atom, AtomTable, Lambda, RecordDef, Term, flatmap_order};
use std::num::NonZeroU64;
use std::sync::Arc;

/// Read a compact operand with the expected tag as a 32-bit field of an
/// instruction: a label, a register, a count, or an arity.
#[expect(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    reason = "the loader keeps the low 32 bits of an operand that fills a 32-bit field of an instruction"
)]
fn read_operand_u32(data: &[u8], cursor: &mut usize, expected_tag: u8) -> Result<u32, BeamError> {
    Ok(read_operand(data, cursor, expected_tag)? as u32)
}

/// Read a compact operand with the expected tag as an index or a count.
fn read_operand_usize(
    data: &[u8],
    cursor: &mut usize,
    expected_tag: u8,
) -> Result<usize, BeamError> {
    Ok(usize::try_from(read_operand(data, cursor, expected_tag)?)
        .expect("a u64 fits in usize on a 64-bit host"))
}

const CODE_HEADER_SIZE: usize = WORD_SIZE * 5;
const CODE_HEADER_SUB_SIZE: u32 = 16;
const INSTRUCTION_SET_ERLANG: u32 = 0;
const CODE_LABEL_COUNT_OFFSET: usize = WORD_SIZE * 3;
const CODE_FUNCTION_COUNT_OFFSET: usize = WORD_SIZE * 4;
const OPCODE_LABEL: u8 = 1;
const OPCODE_FUNCTION_INFO: u8 = 2;
const OPCODE_INT_CODE_END: u8 = 3;
const OPCODE_CALL: u8 = 4;
const OPCODE_CALL_ONLY: u8 = 6;
const OPCODE_CALL_EXT: u8 = 7;
const OPCODE_CALL_EXT_LAST: u8 = 8;
const OPCODE_CALL_EXT_ONLY: u8 = 78;
const OPCODE_ALLOCATE: u8 = 12;
const OPCODE_ALLOCATE_HEAP: u8 = 13;
const OPCODE_TRY: u8 = 104;
const OPCODE_TRY_END: u8 = 105;
const OPCODE_TRY_CASE: u8 = 106;
const OPCODE_TRY_CASE_END: u8 = 107;
const OPCODE_RAISE: u8 = 108;
const OPCODE_IF_END: u8 = 73;
const OPCODE_BUILD_STACKTRACE: u8 = 160;
const OPCODE_RAW_RAISE: u8 = 161;
const OPCODE_TEST_IS_FUNCTION: u8 = 77;
const OPCODE_APPLY: u8 = 112;
const OPCODE_APPLY_LAST: u8 = 113;
const OPCODE_TEST_IS_FUNCTION2: u8 = 115;
const OPCODE_CALL_FUN2: u8 = 178;
const OPCODE_NIF_START: u8 = 179;
const OPCODE_BADRECORD: u8 = 180;
const OPCODE_RETURN: u8 = 19;
const OPCODE_DEALLOCATE: u8 = 18;
const OPCODE_MOVE: u8 = 64;
const OPCODE_SELECT_VAL: u8 = 59;
const OPCODE_SELECT_TUPLE_ARITY: u8 = 60;
const OPCODE_JUMP: u8 = 61;
const OPCODE_CATCH: u8 = 62;
const OPCODE_CATCH_END: u8 = 63;
/// `bs_get_float2 Fail Ctx Live Size Unit Flags Dst` (genop.tab:448).
const OPCODE_BS_GET_FLOAT2: u8 = 118;
/// The `little` flag of a segment (`BSF_LITTLE`, `erl_bits.h:462`).
const BSF_LITTLE: u64 = 2;
/// The `native` flag of a segment (`BSF_NATIVE`, `erl_bits.h:465`).
const BSF_NATIVE: u64 = 16;
/// The `signed` flag of a segment (`BSF_SIGNED`, `erl_bits.h:463`).
const BSF_SIGNED: u64 = 4;
/// `bs_get_binary2 Fail Ctx Live Size Unit Flags Dst` (genop.tab:449).
const OPCODE_BS_GET_BINARY2: u8 = 119;
const OPCODE_BS_SKIP_BITS2: u8 = 120;
/// `bs_get_integer2 Fail Ctx Live Size Unit Flags Dst` (genop.tab:447).
const OPCODE_BS_GET_INTEGER2: u8 = 117;
/// `gc_bif3 Fail Live Bif Arg1 Arg2 Arg3 Dst` (genop.tab:533).
const OPCODE_GC_BIF3: u8 = 152;
/// `bif3 Lbl Bif Arg1 Arg2 Arg3 Reg` (genop.tab:716-720).
const OPCODE_BIF3: u8 = 185;
/// `is_any_native_record Fail Src` (genop.tab:724).
const OPCODE_IS_ANY_NATIVE_RECORD: u8 = 186;
/// `fmove Src Dst` (genop.tab:410). ERTS loads it as `fstore` or `fload`
/// (emu ops.tab:1337-1338; JIT jit/arm/ops.tab:1124-1125).
const OPCODE_FMOVE: u8 = 96;
/// `fconv Src Dst` (genop.tab:411). ERTS loads it as `fconv` (emu
/// ops.tab:1335, 1343; JIT jit/arm/ops.tab:1130).
const OPCODE_FCONV: u8 = 97;
/// `fadd Fail A B Dst` (genop.tab:412). ERTS loads it as `i_fadd` (emu
/// ops.tab:1329; JIT jit/arm/ops.tab:1118).
const OPCODE_FADD: u8 = 98;
/// `fsub Fail A B Dst` (genop.tab:413). ERTS loads it as `i_fsub` (emu
/// ops.tab:1330; JIT jit/arm/ops.tab:1119).
const OPCODE_FSUB: u8 = 99;
/// `fmul Fail A B Dst` (genop.tab:414). ERTS loads it as `i_fmul` (emu
/// ops.tab:1331; JIT jit/arm/ops.tab:1120).
const OPCODE_FMUL: u8 = 100;
/// `fdiv Fail A B Dst` (genop.tab:415). ERTS loads it as `i_fdiv` (emu
/// ops.tab:1332; JIT jit/arm/ops.tab:1121).
const OPCODE_FDIV: u8 = 101;
/// `fnegate Fail A Dst` (genop.tab:416). ERTS loads it as `i_fnegate` (emu
/// ops.tab:1333; JIT jit/arm/ops.tab:1122).
const OPCODE_FNEGATE: u8 = 102;
/// The extended operand subtag of a float register, `{fr, N}`
/// (`beam_asm.erl`, `TAG_z` value 2).
const FLOAT_REGISTER_SUBTAG: i128 = 2;
const OPCODE_GC_BIF1: u8 = 124;
const OPCODE_GC_BIF2: u8 = 125;
/// `bs_get_tail Ctx Dst Live` (genop.tab:591-593).
const OPCODE_BS_GET_TAIL: u8 = 165;
/// `bs_start_match3 Fail Bin Live Dst` (genop.tab:597).
const OPCODE_BS_START_MATCH3: u8 = 166;
/// `bs_get_position Ctx Dst Live` (genop.tab:599-601).
const OPCODE_BS_GET_POSITION: u8 = 167;
/// `bs_set_position Ctx Pos` (genop.tab:603-605).
const OPCODE_BS_SET_POSITION: u8 = 168;
/// `bs_start_match4 Fail Live Src Dst` (genop.tab:613-617).
const OPCODE_BS_START_MATCH4: u8 = 170;
/// `bs_get_utf8 Fail Ctx Live Flags Dst` (genop.tab:501).
const OPCODE_BS_GET_UTF8: u8 = 138;
const OPCODE_BS_SKIP_UTF8: u8 = 139;
/// `bs_get_utf16 Fail Ctx Live Flags Dst` (genop.tab:504).
const OPCODE_BS_GET_UTF16: u8 = 140;
/// `bs_skip_utf16 Fail Ctx Live Flags` (genop.tab:505).
const OPCODE_BS_SKIP_UTF16: u8 = 141;
/// `bs_get_utf32 Fail Ctx Live Flags Dst` (genop.tab:507).
const OPCODE_BS_GET_UTF32: u8 = 142;
/// `bs_skip_utf32 Fail Ctx Live Flags` (genop.tab:508).
const OPCODE_BS_SKIP_UTF32: u8 = 143;
/// `on_load` (`genop.tab:520`), which has no operands. It marks the
/// function of the last label as the `-on_load` function of the module.
const OPCODE_ON_LOAD: u8 = 149;
const OPCODE_LINE: u8 = 153;
const OPCODE_CALL_LAST: u8 = 5;
const OPCODE_BIF0: u8 = 9;
const OPCODE_BIF1: u8 = 10;
const OPCODE_BIF2: u8 = 11;
const OPCODE_TEST_HEAP: u8 = 16;
const OPCODE_PUT_MAP_EXACT: u8 = 155;
const OPCODE_GET_MAP_ELEMENTS: u8 = 158;
const OPCODE_GET_HD: u8 = 162;
const OPCODE_PUT_TUPLE2: u8 = 164;
const OPCODE_SWAP: u8 = 169;
const OPCODE_INIT_YREGS: u8 = 172;
/// `bs_create_bin Fail Alloc Live Unit Dst OpList` (genop.tab:649-651).
const OPCODE_BS_CREATE_BIN: u8 = 177;
/// The number of list items of one `bs_create_bin` segment: `Type`,
/// `Segment`, `Unit`, `Flags`, `Src`, and `Size` (JIT
/// jit/arm/generators.tab:456-465; emu generators.tab:889-937).
const BINARY_SEGMENT_ITEMS: usize = 6;
const OPCODE_BS_INIT_WRITABLE: u8 = 133;
const OPCODE_SEND: u8 = 20;
const OPCODE_REMOVE_MESSAGE: u8 = 21;
const OPCODE_TIMEOUT: u8 = 22;
const OPCODE_LOOP_REC: u8 = 23;
const OPCODE_LOOP_REC_END: u8 = 24;
const OPCODE_WAIT: u8 = 25;
const OPCODE_WAIT_TIMEOUT: u8 = 26;
const OPCODE_GET_LIST: u8 = 65;
const OPCODE_GET_TUPLE_ELEMENT: u8 = 66;
const OPCODE_PUT_LIST: u8 = 69;
const OPCODE_BADMATCH: u8 = 72;
const OPCODE_CASE_END: u8 = 74;
const OPCODE_CALL_FUN: u8 = 75;
const OPCODE_TRIM: u8 = 136;
const OPCODE_GET_TL: u8 = 163;
const OPCODE_TEST_IS_INTEGER: u8 = 45;
const OPCODE_TEST_IS_FLOAT: u8 = 46;
const OPCODE_TEST_IS_NUMBER: u8 = 47;
const OPCODE_TEST_IS_ATOM: u8 = 48;
const OPCODE_TEST_IS_PID: u8 = 49;
const OPCODE_TEST_IS_REFERENCE: u8 = 50;
const OPCODE_TEST_IS_PORT: u8 = 51;
const OPCODE_TEST_IS_NIL: u8 = 52;
const OPCODE_TEST_IS_BINARY: u8 = 53;
const OPCODE_TEST_IS_LIST: u8 = 55;
const OPCODE_TEST_IS_NONEMPTY_LIST: u8 = 56;
const OPCODE_TEST_IS_TUPLE: u8 = 57;
const OPCODE_TEST_ARITY: u8 = 58;
const OPCODE_TEST_IS_BOOLEAN: u8 = 114;
const OPCODE_TEST_IS_BITSTR: u8 = 129;
const OPCODE_TEST_TAGGED_TUPLE: u8 = 159;
const OPCODE_TEST_IS_MAP: u8 = 156;
const OPCODE_TEST_EQ: u8 = 41;
const OPCODE_TEST_LT: u8 = 39;
const OPCODE_TEST_NE: u8 = 42;
const OPCODE_TEST_GE: u8 = 40;
const OPCODE_TEST_EXACT_EQ: u8 = 43;
const OPCODE_TEST_EXACT_NE: u8 = 44;
const OPCODE_PUT_MAP_ASSOC: u8 = 154;
const OPCODE_HAS_MAP_FIELDS: u8 = 157;
const OPCODE_GET_RECORD_ELEMENTS: u8 = 188;
const OPCODE_PUT_RECORD: u8 = 189;
/// `is_native_record Lbl Rec Module Name` (genop.tab:726-729).
const OPCODE_IS_NATIVE_RECORD: u8 = 187;
/// `is_record_accessible Lbl Rec Scope` (genop.tab:742-751).
const OPCODE_IS_RECORD_ACCESSIBLE: u8 = 190;
/// `get_record_field Lbl Rec Id Field Dst` (genop.tab:753-757).
const OPCODE_GET_RECORD_FIELD: u8 = 191;
const OPCODE_MAKE_FUN3: u8 = 171;
/// `bs_match Fail Ctx {commands,Commands}` (genop.tab:685-696).
const OPCODE_BS_MATCH: u8 = 182;
/// `bs_match_string Fail Ctx Bits Val` (genop.tab:488).
const OPCODE_BS_MATCH_STRING: u8 = 132;
const OPCODE_UPDATE_RECORD: u8 = 181;
const OPCODE_RECV_MARKER_BIND: u8 = 173;
const OPCODE_RECV_MARKER_CLEAR: u8 = 174;
const OPCODE_RECV_MARKER_RESERVE: u8 = 175;
const OPCODE_RECV_MARKER_USE: u8 = 176;

/// The largest choice list that keeps the linear walk of
/// [`Instruction::SelectValue`].
///
/// The value is 32. It is a measured estimate between the two end-to-end
/// points of the task: the ordered form loses 13.276 ns for each iteration at
/// 16 choices, net of the 2-choice control, and wins 458.005 ns at 256 choices
/// (`tests/integration/select_val.rs`). The only micro-measurement point
/// between them is 32 choices, where the linear walk costs 152.053 ns in the
/// worst case and the ordered search costs 34.172 ns, against 78.440 ns and
/// 27.791 ns at 16. No end-to-end probe covers 17 to 32 choices, so the bound
/// is an estimate, and a later session can pin it with end-to-end probes at 32
/// and 64 choices.
///
/// The pinned ERTS uses 10 (`erts/emulator/beam/emu/generators.tab:240-254`,
/// OTP 29.1.1). Its ordered search compares one encoded value at a time, which
/// costs less than the `Term::compare` call of PBeam, so the crossover of ERTS
/// sits below the crossover of PBeam. ERTS reaches that generator only when a
/// jump table does not fit, and it special-cases two values with
/// `i_select_val2` (`erts/emulator/beam/emu/ops.tab:159-169`, OTP 29.1.1).
/// PBeam has no jump table, and it keeps the linear form for two values.
///
/// `docs/interpreter-dispatch-measurements.md` records the numbers.
pub(crate) const SELECT_VALUE_LINEAR_LIMIT: usize = 32;

/// The prepared choice list of [`Instruction::SelectValue`].
///
/// The decoder builds this form one time at load time. A selection with at
/// most [`SELECT_VALUE_LINEAR_LIMIT`] choices keeps the encoded order in
/// [`SelectValueChoices::Linear`], and the interpreter walks it. A larger
/// selection goes into [`SelectValueChoices::Ordered`], where the choices
/// are sorted by the ERTS term order, and the interpreter searches them
/// with a binary search.
///
/// The ERTS loader takes the same two forms in the generator that serves a
/// choice list that a jump table does not fit, and after its two-value special
/// case. It selects `i_select_val_lins` for 3 to 10 values and
/// `i_select_val_bins` above that, and it sorts the encoded values with
/// `beam_load_sort_select_vals`
/// (`erts/emulator/beam/emu/generators.tab:214-268` and
/// `erts/emulator/beam/beam_transform_helpers.c:135-140`, OTP 29.1.1). PBeam
/// sorts by [`Term::compare`] instead of the encoded value. It sorts and
/// searches with the same order, so the selected clause agrees.
///
/// # Invariant
///
/// [`SelectValueChoices::Ordered`] holds the choices in the order of
/// [`Term::compare`] and holds each distinct choice one time. A choice list
/// with two equal choices, with two choices that have no term order, or with
/// at most [`SELECT_VALUE_LINEAR_LIMIT`] choices stays
/// [`SelectValueChoices::Linear`], so the linear walk keeps the
/// first-match result that ERTS gives.
///
/// # Examples
///
/// ```rust,ignore
/// let atoms = AtomTable::new();
/// let choices = vec![(Term::Integer(1), 7), (Term::Integer(2), 8)];
/// assert!(matches!(
///     prepare_select_value_choices(choices, &atoms),
///     SelectValueChoices::Linear(_)
/// ));
/// ```
#[derive(Debug, Clone)]
pub(crate) enum SelectValueChoices {
    /// The choices of the stream in the encoded order. The list holds at
    /// most [`SELECT_VALUE_LINEAR_LIMIT`] entries.
    Linear(Vec<(Term, u32)>),
    /// The choices in the order of [`Term::compare`]. The list holds more
    /// than [`SELECT_VALUE_LINEAR_LIMIT`] entries, and each entry is
    /// distinct and comparable with each other entry.
    Ordered(Vec<(Term, u32)>),
}

/// Build the prepared form of a choice list for [`Instruction::SelectValue`].
///
/// Return [`SelectValueChoices::Linear`] with the choices in the encoded
/// order when the list holds at most [`SELECT_VALUE_LINEAR_LIMIT`] entries,
/// when two choices are equal, or when two choices have no term order.
/// Otherwise return [`SelectValueChoices::Ordered`] with the choices sorted
/// by [`Term::compare_exact`] and free of duplicates. ERTS matches each
/// choice exactly, so the order puts `1` and `1.0` apart, and `-0.0` and
/// `0.0` apart (`emu/select_instrs.tab:57-88`).
///
/// The Erlang compiler emits distinct choices, and this function still
/// detects a duplicate, because ERTS gives the first matching clause and a
/// sorted list would lose that order.
///
/// # Examples
///
/// ```rust,ignore
/// let atoms = AtomTable::new();
/// let choices: Vec<(Term, u32)> = (0..33)
///     .map(|value| (Term::Integer(value as i64), value as u32))
///     .collect();
/// let SelectValueChoices::Ordered(ordered) = prepare_select_value_choices(choices, &atoms) else {
///     panic!("33 distinct choices take the ordered form");
/// };
/// assert!(ordered[0].0.compare_exact(&ordered[1].0, &atoms)?.is_lt());
/// ```
pub(crate) fn prepare_select_value_choices(
    choices: Vec<(Term, u32)>,
    atoms: &AtomTable,
) -> SelectValueChoices {
    if choices.len() <= SELECT_VALUE_LINEAR_LIMIT {
        return SelectValueChoices::Linear(choices);
    }
    let mut ordered = choices.clone();
    ordered.sort_by(|(left, _), (right, _)| left.order_exact(right, atoms));
    let distinct = ordered
        .windows(2)
        .all(|pair| pair[0].0 != pair[1].0 && pair[0].0.order_exact(&pair[1].0, atoms).is_ne());
    if distinct {
        SelectValueChoices::Ordered(ordered)
    } else {
        SelectValueChoices::Linear(choices)
    }
}

#[derive(Debug, Clone)]
/// An instruction that the parser can read from a BEAM `Code` chunk.
///
/// Byte examples use hexadecimal values. Compact operands use the short form
/// in the Code chunk grammar. `End` represents the stream terminator; the
/// executor does not run it.
///
/// A branch operand and a local-call operand hold the label number that the
/// parser read from the stream. `ModuleTables::build` replaces each one with
/// the resolved code position before any process runs: a branch operand
/// becomes the index of the instruction after the label, and a call operand
/// becomes the index of the function in the function table of the module and
/// the index of the instruction at the label. A loaded instruction therefore
/// reads the field and runs no lookup.
pub(crate) enum Instruction {
    /// Starts a code label. The label must be in `1..=label_count`.
    ///
    /// Encoding: opcode, then a compact unsigned label. Example: `[0x01, 0x10]`
    /// encodes label 1.
    ///
    /// Erlang: every function and branch target has a label.
    /// `a() -> ok.` compiles to `{label,1}` and `{label,2}`.
    Label(u32),
    /// Declares the module, function, and arity for the following function.
    /// Atom indexes refer to the one-based `AtU8` table.
    ///
    /// Encoding: opcode, module atom, function atom, and arity. Example:
    /// `[0x02, 0x12, 0x22, 0x00]` refers to atom indexes 1 and 2 with arity 0.
    ///
    /// Erlang: `a() -> ok.` in module `ex` compiles to
    /// `{func_info,{atom,ex},{atom,a},0}`.
    FunctionInfo {
        module: String,
        name: String,
        arity: u32,
    },
    /// Moves a literal or register value into an X or Y register.
    ///
    /// Erlang: `a() -> ok.` compiles to `{move,{atom,ok},{x,0}}`.
    Move(Operand, Operand),
    /// Calls a local function and continues after it returns.
    ///
    /// `function` is the index of the function in the function table of the
    /// module, and `target` is the index of the instruction at the label that
    /// the call names. A call starts at `target`, which can be a label inside
    /// a function and not only its entry. The parser writes the label operand
    /// into both fields, and `ModuleTables::build` replaces both.
    ///
    /// Erlang: `b(X) -> c(X), ok.` compiles to `{call,1,{f,6}}`.
    Call {
        arity: u32,
        function: u32,
        target: u32,
    },
    /// Releases `deallocate` Y registers and calls a local function in place
    /// of the active one. `function` and `target` are as for [`Self::Call`].
    ///
    /// Encoding: opcode, arity, label, deallocate. Example:
    /// `[0x05, 0x30, 0x35, 0x20]` calls label 3 with arity 3 and releases two
    /// Y registers.
    ///
    /// Erlang: `io:request/3` compiles `request(group_leader(), Request, Error)`
    /// to `{call_last,3,{io,request,3},2}`.
    CallLast {
        arity: u32,
        function: u32,
        target: u32,
        deallocate: u32,
    },
    /// Tail-calls a local function. `function` and `target` are as for
    /// [`Self::Call`].
    ///
    /// Erlang: `c(X) -> d(X).` compiles to `{call_only,1,{f,8}}`.
    CallOnly {
        arity: u32,
        function: u32,
        target: u32,
    },
    /// Calls an imported function. `tail` marks `call_ext_last` and
    /// `call_ext_only`; `deallocate` is the stack size to release first.
    /// The loader puts `Deallocate { stack_size: deallocate }` after a
    /// `call_ext_last` and `Return` after a `call_ext_last` or a
    /// `call_ext_only`. A tail call of Erlang code replaces the frame and
    /// never reaches them. A tail call of a BIF runs as a body call and then
    /// runs them, as the ERTS loader turns it into `call_light_bif Bif |
    /// deallocate D | return` or `i_call_ext Bif | deallocate D | return`
    /// (`jit/arm/ops.tab:687-714`). Thus the `return` costs a reduction.
    ///
    /// Encoding: `[0x4E, 0x10, 0x00]` is `call_ext_only 1` of import 0.
    ///
    /// Erlang: `d(X) -> lists:reverse(X).` compiles to
    /// `{call_ext_only,1,{extfunc,lists,reverse,1}}`.
    CallExternal {
        /// The index of the import in [`BeamModule::imports`], which holds
        /// the MFA and the call that the loader decided for it. The arity
        /// of the call is the arity of the import.
        ///
        /// [`BeamModule::imports`]: crate::beam::BeamModule
        import: u32,
        tail: bool,
        deallocate: u32,
    },
    /// Runs `erlang:hibernate/0` as the ERTS `i_hibernate` instruction: it
    /// writes `ok` to `x0`, keeps the frame, and stops the run of the
    /// process. The loader turns `call_ext 0 erlang:hibernate/0` into
    /// `Hibernate`, `call_ext_last 0 erlang:hibernate/0 D` into `Hibernate`,
    /// `Deallocate { stack_size: D }`, `Return`, and
    /// `call_ext_only 0 erlang:hibernate/0` into `Hibernate`, `Return`
    /// (`jit/arm/ops.tab:669-673`; `emu/ops.tab:896-898`).
    ///
    /// Encoding: `[0x07, 0x00, 0x00]` is `call_ext 0` of import 0, where
    /// import 0 is `erlang:hibernate/0`.
    ///
    /// Erlang: `gen_server:loop_hibernate/4` compiles
    /// `erlang:hibernate()` to `{call_ext,0,{extfunc,erlang,hibernate,0}}`,
    /// and `erlang.beam` compiles `hibernate() -> erlang:hibernate().` to
    /// `{call_ext_only,0,{extfunc,erlang,hibernate,0}}`.
    Hibernate,
    /// Runs `erlang:yield/0` as the ERTS `i_yield` instruction: it writes
    /// `true` to `x0`, keeps the frame, and switches out the process with no
    /// reduction cost (`jit/arm/instr_common.cpp:3121-3124`;
    /// `jit/arm/instr_call.cpp:30-36`). The loader turns
    /// `call_ext 0 erlang:yield/0` into `Yield`,
    /// `call_ext_last 0 erlang:yield/0 D` into `Yield`,
    /// `Deallocate { stack_size: D }`, `Return`, and
    /// `call_ext_only 0 erlang:yield/0` into `Yield`, `Return`
    /// (`jit/arm/ops.tab:664-666`).
    ///
    /// Encoding: `[0x07, 0x00, 0x00]` is `call_ext 0` of import 0, where
    /// import 0 is `erlang:yield/0`.
    ///
    /// Erlang: `erlang.beam` compiles `yield() -> erlang:yield().` to
    /// `{call_ext_only,0,{extfunc,erlang,yield,0}}`.
    Yield,
    /// Calls the function whose module is in `x(arity)` and whose name is in
    /// `x(arity + 1)`, with the arguments in `x0` to `x(arity - 1)`. `apply`
    /// (opcode 112) continues after the call. `apply_last` (opcode 113) sets
    /// `tail` and releases `deallocate` Y registers first.
    ///
    /// Encoding: `apply` has opcode and arity; `apply_last` has opcode, arity,
    /// and deallocate. Example: the pinned `gen_statem.beam` has
    /// `[0x70, 0x00]` (`apply 0`) in `get_callback_mode/2` and
    /// `[0x71, 0x10, 0xB0]` (`apply_last 1 11`) in `terminate/7`.
    ///
    /// Erlang: `a3(M, F, A) -> {M:F(A, b, c)}.` compiles to `{apply,3}`, and
    /// `last(M, F) -> M:F(bye).` compiles to `{apply_last,1,0}`.
    Apply {
        arity: u32,
        tail: bool,
        deallocate: u32,
    },
    /// Calls a BIF through `bif0` (opcode 9), `bif1` (opcode 10), `bif2`
    /// (opcode 11), `bif3` (opcode 185), `gc_bif1` (opcode 124), `gc_bif2`
    /// (opcode 125), or `gc_bif3` (opcode 152) and writes its value to a register. `bif0` has no fail label and decodes with `fail_label` 0.
    /// With fail label 0, a BIF failure raises its exception; with another
    /// fail label, it jumps to the label.
    ///
    /// Encoding: `bif0` has opcode, import index, destination register;
    /// `bif1`, `bif2`, and `bif3` have opcode, fail label, import index, one,
    /// two, or three arguments, destination register. `gc_bif1`, `gc_bif2`, and `gc_bif3`
    /// have opcode, fail label, live register count, import index, one, two,
    /// or three arguments, destination register. The live count tells the ERTS garbage collector which X
    /// registers hold terms; PBeam has no process heap and does not keep it.
    /// An argument can be a typed register (extended subtag 5). Example:
    /// `io:io_request/2` in the pinned `io.beam` has
    /// `[0x0A, 0x05, 0xE0, 0x57, 0x03, 0x40, 0x23]`, which calls import 14
    /// (`erlang:node/1`) with `x0` typed as a pid and writes `x2`, and
    /// `[0x09, 0xF0, 0x53]`, which calls import 15 (`erlang:node/0`) and
    /// writes `x5`. The pinned `gen_statem.beam` has
    /// `[0x0B, 0x05, 0x08, 0x21, 0x21, 0x03, 0x03]`, which calls import 33
    /// (`erlang:element/2`) with the integer 2 and `x0` and writes `x0`.
    /// `arith_probe:up/1` in `fixtures/arith/arith_probe.beam` has
    /// `[0x7D, 0x0D, 0x3A, 0x10, 0x00, 0x57, 0x03, 0x10, 0x11, 0x03]`, which
    /// calls import 0 (`erlang:'+'/2`) with fail label 58, live count 1,
    /// `x0` typed, and the integer 1, and writes `x0`. `arith_probe:neg/1`
    /// has `[0x7C, 0x0D, 0x37, 0x10, 0x50, 0x03, 0x03]`, which calls import 5
    /// (`erlang:'-'/1`) with fail label 55, live count 1, and `x0`, and
    /// writes `x0`. `binary_part_probe:guard/3` in
    /// `fixtures/binary_part/binary_part_probe.beam` has
    /// `[0x98, 0xD5, 0x30, 0x00, 0x03, 0x13, 0x23, 0x03]`, which calls import 0
    /// (`erlang:binary_part/3`) with fail label 13, live count 3, `x0`, `x1`,
    /// and `x2`, and writes `x0`. `is_integer_probe:guard/3` in
    /// `fixtures/is_integer/is_integer_probe.beam` has
    /// `[0xB9, 0xC5, 0x10, 0x03, 0x13, 0x23, 0x03]`, which calls import 1
    /// (`erlang:is_integer/3`) with fail label 12, `x0`, `x1`, and `x2`, and
    /// writes `x0`.
    ///
    /// Erlang: `h() -> self().` compiles to `{bif,self,{f,0},[],{x,0}}`, and
    /// `node(Pid) =:= node()` in `io:io_request/2` compiles to
    /// `{bif,node,{f,0},[{tr,{x,0},pid}],{x,2}}` and
    /// `{bif,node,nofail,[],{x,5}}`. The `STACKTRACE()` macro of
    /// `gen_statem`, `element(2, erlang:process_info(self(),
    /// current_stacktrace))`, compiles to
    /// `{bif,element,{f,0},[{integer,2},{x,0}],{x,0}}`. The guard
    /// `when tuple_size(X) >= 2` in `fixtures/group/group_probe.erl`
    /// compiles to `{bif,tuple_size,{f,22},[{x,0}],{x,1}}`, which is
    /// `[0x0A, 0x0D, 0x16, 0x80, 0x03, 0x13]` with import 8. The guard
    /// `when X + 1 >= 1` of `arith_probe:up/1` compiles to
    /// `{gc_bif,'+',{f,58},1,[{tr,{x,0},...},{integer,1}],{x,0}}`, and
    /// `id(-9223372036854775808) - id(1)` in `minus_overflow/0` compiles to
    /// `{gc_bif,'-',{f,0},1,[{tr,{y,0},...},{tr,{x,0},...}],{x,0}}`. The
    /// guard `when -X > 0` of `arith_probe:neg/1` compiles to
    /// `{gc_bif,'-',{f,55},1,[{x,0}],{x,0}}`, and `length(Args)` in
    /// `proc_lib:trans_init/3` (`proc_lib.erl:944`) compiles to
    /// `{gc_bif,length,{f,0},3,[{tr,{x,2},...}],{x,2}}`. The guard
    /// `when binary_part(Binary, Start, Length) =:= <<"ell">>` of
    /// `binary_part_probe:guard/3` compiles to
    /// `{gc_bif,binary_part,{f,13},3,[{x,0},{x,1},{x,2}],{x,0}}`. The
    /// guard `when is_integer(Term, Low, High)` of
    /// `is_integer_probe:guard/3` compiles to
    /// `{bif,is_integer,{f,12},[{x,0},{x,1},{x,2}],{x,0}}`, as the compiler
    /// emits a BIF of arity 3 that does no garbage collection as `bif3`
    /// (`beam_asm.erl:650`; `beam_ssa_codegen.erl:2970`).
    Bif {
        /// Label to jump to when the BIF fails, or 0 to raise the exception.
        fail_label: u32,
        /// The BIF of the import that the instruction calls. The parser
        /// resolves the MFA of the import to its identity, as the ERTS
        /// loader binds the import of a BIF to the BIF (`emu_load.c`), so
        /// the instruction holds no name and the run searches no table.
        bif: BifId,
        /// Argument operands of the BIF, in argument order.
        arguments: Vec<Operand>,
        /// Register for the BIF value.
        destination: Operand,
    },
    /// Makes a fun of a lambda with the values of `free` as its free
    /// variables (`make_fun3`, opcode 171), and writes it to `destination`.
    /// The loader resolves the `FunT` entry of the lambda number.
    ///
    /// Encoding: opcode, unsigned lambda number, destination register,
    /// extended list, item count, then the free variable operands. Example:
    /// `fun_probe:make_pair/1` in `fixtures/funs/fun_probe.beam` has
    /// `[0xAB, 0x40, 0x03, 0x17, 0x10, 0x03]`, which makes a fun of `FunT`
    /// entry 4 with the free variable `x0` and writes it to `x0`.
    ///
    /// Erlang: `make_pair(T) -> fun(X) -> {T, X} end.` compiles to
    /// `{make_fun3,{f,112},0,0,{x,0},{list,[{x,0}]}}`.
    MakeFun {
        lambda: Arc<Lambda>,
        free: Vec<Operand>,
        destination: Operand,
    },
    /// Calls the fun in `function` with the arguments in `x0` to
    /// `x(arity - 1)` and continues after the call. `call_fun` (opcode 75)
    /// calls the fun in `x(arity)`. `call_fun2` (opcode 178) names the fun
    /// register and has a tag: `safe`, `unsafe`, or a lambda number. The tag
    /// tells the JIT which tests it can skip; the result is the same, so
    /// the tag is not kept.
    ///
    /// Encoding: `call_fun` has opcode and arity; `call_fun2` has opcode,
    /// tag, arity, and the fun register. Example: `[0x4B, 0x00]` is
    /// `call_fun 0`, and `fun_probe:tail_call/1` in
    /// `fixtures/funs/fun_probe.beam` has `[0xB2, 0x00, 0x10, 0x57, 0x13,
    /// 0x20]`, which calls the fun in the typed register `x1` with lambda
    /// number 0 and arity 1.
    ///
    /// Erlang: `(id(Const))()` compiles to `{call_fun,0}`, and
    /// `tail_call(F) -> F(tail).` compiles to `{call_fun2,{f,127},1,{tr,{x,1},...}}`.
    ///
    /// BEAM has no tail form of a fun call. The compiler emits the call, then
    /// `deallocate D` and `return`. The ERTS loader joins the three
    /// instructions into `i_call_fun_last` or `i_call_fun2_last`
    /// (`jit/arm/ops.tab:868-880`, `emu/ops.tab:1067-1073`), and so does
    /// `parse_code`: the call then sets `tail`, and `deallocate` is the stack
    /// size to release first.
    CallFun {
        arity: u32,
        function: Operand,
        tail: bool,
        deallocate: u32,
    },
    /// Writes a catch tag to a Y register. While the tag is live, an
    /// exception continues after `label`. `try` (opcode 104) and `catch`
    /// (opcode 62) both map here, because the ERTS loader rewrites
    /// `try Y F` to `catch Y F` (`ops.tab`).
    ///
    /// Encoding: opcode, Y register, label. Example: `[0x68, 0x04, 0x75]` is
    /// `try y0 f7`, `[0x68, 0x04, 0x0D, 0x11]` is `try y0 f17`, and
    /// `[0x3E, 0x04, 0xA5]` is `catch y0 f10`.
    ///
    /// Erlang: `catch_value() -> catch ?MODULE:id(ok).` in
    /// `fixtures/exception/exception_probe.erl` compiles to
    /// `{'catch',{y,0},{f,13}}`, and `try_value/1` compiles to
    /// `{'try',{y,0},{f,10}}`.
    Catch {
        /// Index of the Y register that holds the tag.
        register: u32,
        /// Label of the handler code.
        label: u32,
    },
    /// Clears the catch tag of a `try` when its body returns without an
    /// exception. The Y register becomes `[]`, as ERTS `make_blank` does.
    ///
    /// Encoding: opcode, Y register. Example: `[0x69, 0x04]` is `try_end y0`.
    ///
    /// Erlang: `try_value/1` in `fixtures/exception/exception_probe.erl`
    /// compiles to `{try_end,{y,0}}` after the `try` body.
    TryEnd {
        /// Index of the Y register that holds the tag.
        register: u32,
    },
    /// Starts the handler of a `try`: clears the catch tag, then copies the
    /// exception class from `x3` to `x0`. `x1` holds the reason and `x2` the
    /// raw stack trace.
    ///
    /// Encoding: opcode, Y register. Example: `[0x6A, 0x04]` is
    /// `try_case y0`.
    ///
    /// Erlang: `try_stub() -> try stub() catch error:R -> {error, R} end.`
    /// compiles to `{try_case,{y,0}}` at the handler label.
    TryCase {
        /// Index of the Y register that holds the tag.
        register: u32,
    },
    /// Ends a `catch` expression: clears the catch tag. After an exception,
    /// `x0` becomes `{'EXIT', {Reason, Stack}}` for class `error`,
    /// `{'EXIT', Reason}` for class `exit`, and `Reason` for class `throw`.
    ///
    /// Encoding: opcode, Y register. Example: `[0x3F, 0x04]` is
    /// `catch_end y0`.
    ///
    /// Erlang: `catch_exit() -> catch exit(bye).` compiles to
    /// `{catch_end,{y,0}}` at the handler label.
    CatchEnd {
        /// Index of the Y register that holds the tag.
        register: u32,
    },
    /// Raises an exception again. The class and the stack trace come from
    /// the raw stack trace `trace`, and the reason is `value`. A `try` uses
    /// it when no `catch` clause matches, and so does the cleanup of
    /// `try ... after`.
    ///
    /// Encoding: opcode, raw stack trace, reason. Example:
    /// `[0x6C, 0x23, 0x13]` is `raise x2 x1`.
    ///
    /// Erlang: `try erlang:raise(exit, R, []) catch error:_ -> error end`
    /// compiles to `{bif,raise,{f,0},[{x,2},{x,1}],{x,0}}` after the
    /// clauses; `beam_asm` writes only the two arguments.
    Raise {
        /// The raw stack trace that the handler of the `try` got in `x2`.
        trace: Operand,
        /// The reason that the handler of the `try` got in `x1`.
        value: Operand,
    },
    /// Raises class `error` with the reason `{Tag, Value}`, where `Tag` is
    /// `badmatch`, `case_clause`, `try_clause`, or `badrecord`. `badmatch`,
    /// `case_end`, `try_case_end`, and `badrecord` map here.
    ///
    /// Encoding: opcode, value. Examples: `[0x48, 0x04]` is `badmatch y0`,
    /// `[0x4A, 0x03]` is `case_end x0`, `[0x6B, 0x03]` is
    /// `try_case_end x0`, and `[0xB4, 0x03]` is `badrecord x0`.
    ///
    /// Erlang: `{ok, _} = X` compiles to `{badmatch,{x,0}}`, `case X of a ->
    /// ok end` to `{case_end,{x,0}}`, `try f() of a -> ok catch _ -> e end`
    /// to `{try_case_end,{x,0}}`, and `X#r.a` to `{badrecord,{x,0}}` when no
    /// clause matches.
    TaggedError {
        /// The atom of the reason tuple: `badmatch`, `case_clause`,
        /// `try_clause`, or `badrecord`.
        tag: &'static str,
        /// The term that did not match.
        value: Operand,
    },
    /// Raises class `error` with the reason `if_clause`.
    ///
    /// Encoding: `[0x49]`.
    ///
    /// Erlang: `if X =:= a -> ok end` compiles to `if_end` after the
    /// guard.
    IfEnd,
    /// Raises the reason in `x1` with the class in `x0` and the stack trace
    /// of the raw stack trace in `x2`. A class that is not `error`, `exit`,
    /// or `throw` writes `badarg` to `x0` and continues.
    ///
    /// Encoding: `[0xA1]`.
    ///
    /// Erlang: `try f() catch _:R:St -> erlang:raise(C, R, St) end` compiles
    /// to `{move,{y,0},{x,0}}` and `raw_raise` after `try_case`, when the
    /// handler uses `St` only in `erlang:raise/3`.
    RawRaise,
    /// Changes the raw stack trace in `x0` to its stack trace, a list. The
    /// compiler moves the raw stack trace from `x2` to `x0` first.
    ///
    /// Encoding: `[0xA0]`.
    ///
    /// Erlang: `try f() catch C:R:S -> S end` compiles to
    /// `{move,{x,2},{x,0}}` and `build_stacktrace` after `try_case`.
    BuildStacktrace,
    /// Allocates `stack_size` Y registers for the active function.
    /// `allocate_heap` (opcode 13) also maps here: PBeam terms do not use a
    /// process heap, so its heap need has no effect.
    ///
    /// Encoding: opcode, stack size, live registers; `allocate_heap` has a heap
    /// need before the live count. Example: `[0x0d, 0x30, 0x80, 0x20]`
    /// allocates three Y registers with heap need 8 and two live registers.
    ///
    /// Erlang: `b(X) -> c(X), ok.` compiles to `{allocate,0,1}`;
    /// `io:put_chars/2` starts with `{allocate_heap,3,8,2}`.
    Allocate { stack_size: u32 },
    /// Releases the Y registers of the active function.
    ///
    /// Erlang: `b(X) -> c(X), ok.` compiles to `{deallocate,0}`.
    Deallocate { stack_size: u32 },
    /// Drops the first `count` Y registers of the active function and keeps
    /// the next `remaining` registers, which become `y0` onwards.
    ///
    /// Encoding: opcode, count, remaining. Example: `[0x88, 0x10, 0x10]`
    /// drops `y0` and keeps one register.
    ///
    /// Erlang: `order/0` in `fixtures/exit_signal/exit_signal_probe.erl`
    /// compiles to `{trim,1,1}` before its last local call.
    Trim { count: u32, remaining: u32 },
    /// Continues execution after a label.
    ///
    /// Erlang: a receive clause jumps past the `after` body, for example
    /// `{jump,{f,8}}` in `fixtures/process/process_probe.erl`.
    Jump(u32),
    /// Jumps to `fail_label` when the value does not have the tested type.
    ///
    /// Erlang: `e(X) when is_atom(X) -> X; e(_) -> other.` compiles to
    /// `{test,is_atom,{f,11},[{x,0}]}`.
    Test {
        kind: TestKind,
        fail_label: u32,
        value: Operand,
    },
    /// Jumps to `fail_label` unless the value is a fun of `arity` arguments
    /// (`is_function2`, opcode 115). The arity is a literal or a register
    /// (`is_function2 f s s` in `jit/arm/ops.tab:605`). The test passes only
    /// when `erl_is_function` (`erl_bif_op.c:237-267`) returns `true`, as
    /// the JIT does for a register arity (`jit/arm/instr_common.cpp:1266-1283`),
    /// so an arity that is negative or not an integer fails the test.
    ///
    /// Encoding: opcode, fail label, value, arity. Example:
    /// `fun_probe:kind/1` in `fixtures/funs/fun_probe.beam` has
    /// `[0x73, 0x0D, 0x64, 0x57, 0x03, 0x40, 0x11]`, which fails to label 100
    /// unless the typed register `x0` holds a fun of one argument.
    ///
    /// Erlang: `kind(F) when is_function(F, 1) -> one;` compiles to
    /// `{test,is_function2,{f,100},[{tr,{x,0},...},{integer,1}]}`.
    TestFunctionArity {
        fail_label: u32,
        value: Operand,
        arity: Operand,
    },
    /// Jumps to `fail_label` unless the value is a tuple with `arity`
    /// elements.
    ///
    /// Encoding: opcode, fail label, value, arity. Example:
    /// `[0x3a, 0x95, 0x03, 0x50]` fails to label 9 unless `x0` is a 5-tuple.
    ///
    /// Erlang: the `receive {_, Ref, _, _, _}` in
    /// `erts_internal:flush_monitor_messages/3` compiles to
    /// `{test,test_arity,{f,109},[{x,0},5]}`.
    TestArity {
        fail_label: u32,
        value: Operand,
        arity: u32,
    },
    /// Jumps to the label for the arity of a tuple, or to `fail_label`.
    ///
    /// Encoding: opcode, value, fail label, extended list of
    /// `arity, label` pairs. Example: `[0x3c, 0x03, 0x45, 0x17, 0x20, 0x30,
    /// 0x55]` jumps to label 5 for a 3-tuple in `x0`, else to label 4.
    ///
    /// Erlang: the `receive` in `io:execute_request/3` compiles to
    /// `{select_tuple_arity,{x,0},{f,167},{list,[3,{f,165},5,{f,160}]}}`.
    SelectTupleArity {
        value: Operand,
        fail_label: u32,
        choices: Vec<(u32, u32)>,
    },
    /// Jumps to the label of the first matching choice, or to `fail_label`.
    ///
    /// `choices` holds the prepared form of the loader: the encoded order
    /// for a small list, and the ERTS term order for a large list. See
    /// [`SelectValueChoices`].
    ///
    /// Erlang: `f(1) -> one; f(2) -> two; f(_) -> many.` compiles to
    /// `{select_val,{x,0},{f,16},{list,[{integer,1},{f,15},{integer,2},{f,14}]}}`.
    SelectValue {
        value: Operand,
        fail_label: u32,
        choices: SelectValueChoices,
    },
    /// Copies the value of each key in a map to its destination register, or
    /// jumps to `fail_label` when a key is missing. `pairs` holds each key
    /// and its destination. The loader sorts two or more pairs in flatmap
    /// key order (see [`sort_map_key_pairs`]).
    ///
    /// Encoding: opcode, fail label, map operand, extended list, item count,
    /// then key and destination operands in turn. Example:
    /// `[0x9E, 0x35, 0x03, 0x17, 0x20, 0x12, 0x13]` copies the value of atom 1
    /// in the map in `x0` to `x1`, or fails to label 3.
    ///
    /// Erlang: `output(#{output := raw}) -> raw.` compiles to
    /// `{get_map_elements,{f,9},{x,0},{list,[{atom,output},{x,0}]}}`, and
    /// `get40(#{40 := V}) -> V;` compiles to
    /// `{get_map_elements,{f,17},{tr,{x,0},{t_map,any,any}},{list,[{integer,40},{x,0}]}}`.
    GetMapElements {
        fail_label: u32,
        map: Operand,
        pairs: Vec<(Operand, Operand)>,
    },
    /// Jumps to `fail_label` when a key is not in a map, and writes no
    /// register. Each key is a literal or a register. The ERTS loader changes
    /// this test to `get_map_elements` with a scratch destination for each
    /// key (`generators.tab:230-253`).
    ///
    /// Encoding: opcode, fail label, map operand, extended list, item count,
    /// then the key operands. Example: `[0x9D, 0x35, 0x03, 0x17, 0x10, 0x12]`
    /// tests that the map in `x0` has atom 1, or fails to label 3.
    ///
    /// Erlang: `pair(#{input := _, output := _}) -> both;` compiles to
    /// `{test,has_map_fields,{f,17},{x,0},{list,[{atom,input},{atom,output}]}}`,
    /// and the `#{K := _}` pattern of `key(K, M)` compiles to
    /// `{test,has_map_fields,{f,20},{x,1},{list,[{x,0}]}}`.
    HasMapFields {
        fail_label: u32,
        map: Operand,
        keys: Vec<Operand>,
    },
    /// Jumps to `fail_label` unless the native record in `value` has the
    /// definition of `name` in `module` (`erl_is_native_record`,
    /// `erl_record.c:214-221`). Erlang: the match `#r{} = R` compiles to
    /// `{test,is_native_record,{f,5},[{x,0},{atom,m},{atom,r}]}`.
    IsNativeRecord {
        fail_label: u32,
        value: Operand,
        module: Atom,
        name: Atom,
    },
    /// Jumps to `fail_label` unless the fields of the native record in
    /// `value` are accessible (`erl_record.c:452-468`): with `external`,
    /// only an exported record passes; otherwise (`auto_local`), an
    /// exported record or a record of the running module passes. Erlang:
    /// `#m:r{} = R` compiles to
    /// `{test,is_record_accessible,{f,5},[{x,0},{atom,external}]}`.
    IsRecordAccessible {
        fail_label: u32,
        value: Operand,
        external: bool,
    },
    /// Writes the value of each field of the native record in `value` to its
    /// register, in the sorted key order, and jumps to `fail_label` when a
    /// field is missing, after the writes of the fields before it
    /// (`erl_get_record_elements`, `erl_record.c:223-252`). Erlang:
    /// `#r{x = X} = R` compiles to
    /// `{get_record_elements,{f,5},{x,0},{list,[{atom,x},{x,1}]}}`.
    GetRecordElements {
        fail_label: u32,
        value: Operand,
        pairs: Vec<(Atom, Operand)>,
    },
    /// Writes a new native record to `destination` (`put_record`,
    /// `erl_record.c:254-450`). `updates` holds each field and its value in
    /// the sorted key order. Erlang: `#r{x = 1}` compiles to
    /// `{put_record,{f,0},{atom,r},nil,{x,0},1,{list,[{atom,x},{integer,1}]}}`.
    PutRecord {
        kind: PutRecord,
        destination: Operand,
        updates: Vec<(Atom, Operand)>,
    },
    /// Writes the value of `field` of the native record in `value` to
    /// `destination` (`erl_get_local_record_field` and
    /// `erl_get_record_field`, `erl_record.c:470-568`). It raises when
    /// `fail_label` is 0, and jumps to it otherwise. Erlang: `R#r.x`
    /// compiles to `{get_record_field,{f,0},{x,0},{atom,r},{atom,x},{x,0}}`.
    GetRecordField {
        fail_label: u32,
        value: Operand,
        id: RecordId,
        field: Atom,
        destination: Operand,
    },
    /// Writes a copy of a map with each key set to its value. `pairs` holds
    /// each key and value. The fail label and live-register count are not
    /// kept: the compiler emits fail label 0, and PBeam needs no live count.
    /// The pairs go into the copy one by one, as in ERTS, so the copy is a
    /// flatmap up to 32 keys and a hashmap above 32 keys.
    ///
    /// Encoding: opcode, fail label, map operand, destination register, live
    /// count, extended list, item count, then key and value operands in turn.
    /// Example: `[0x9A, 0x05, 0x03, 0x03, 0x10, 0x17, 0x20, 0x12, 0x22]` sets
    /// atom 1 to atom 2 in the map in `x0` and writes the result to `x0`.
    ///
    /// Erlang: `add(M) -> M#{output => raw}.` compiles to
    /// `{put_map_assoc,{f,0},{x,0},{x,0},1,{list,[{atom,output},{atom,raw}]}}`.
    PutMapAssoc {
        map: Operand,
        destination: Operand,
        pairs: Vec<(Operand, Operand)>,
    },
    /// Writes a copy of a map with the value of each existing key replaced.
    /// `pairs` holds each key and value. The live-register count is not
    /// kept, because PBeam needs no live count. When the source is not a
    /// map, or a key is not in the map, the instruction jumps to
    /// `fail_label`, or with fail label 0 raises `error:{badmap, Map}` or
    /// `error:{badkey, Key}`.
    ///
    /// Encoding: opcode, fail label, map operand, destination register, live
    /// count, extended list, item count, then key and value operands in turn.
    /// Example: `update_guard/1` in `fixtures/io_start/io_start_probe.beam`
    /// has `[0x9B, 0x0D, 0x26, 0x03, 0x03, 0x10, 0x17, 0x20, 0x0A, 0x12,
    /// 0x0A, 0x13]`, which sets atom 18 (`output`) to atom 19 (`cooked`) in
    /// the map in `x0`, writes the result to `x0`, and jumps to label 38
    /// when the key is missing.
    ///
    /// Erlang: `M#{output := cooked}` in a guard compiles to
    /// `{put_map_exact,{f,38},{x,0},{x,0},1,{list,[{atom,output},{atom,cooked}]}}`.
    PutMapExact {
        fail_label: u32,
        map: Operand,
        destination: Operand,
        pairs: Vec<(Operand, Operand)>,
    },
    /// Exchanges the values of two registers.
    ///
    /// Encoding: opcode, two registers. Example: `[0xA9, 0x03, 0x13]` swaps
    /// `x0` and `x1`.
    ///
    /// Erlang: `main/0` in `fixtures/map/map_probe.erl` compiles to
    /// `{swap,{y,3},{x,0}}` before its last external call, and
    /// `io:put_chars/2` compiles to `{swap,{x,0},{x,2}}`.
    Swap(Operand, Operand),
    /// Builds a list cell from a head and a tail.
    ///
    /// Erlang: `[X]` compiles to `{put_list,{x,0},nil,{x,0}}`.
    PutList {
        head: Operand,
        tail: Operand,
        destination: Operand,
    },
    /// Builds a tuple from the listed operands.
    ///
    /// Encoding: opcode, destination register, extended list, element count,
    /// elements. Example: `[0xA4, 0x03, 0x17, 0x20, 0x12, 0x22]` writes a
    /// tuple of atoms 1 and 2 to `x0`.
    ///
    /// Erlang: `g(X) -> {X, X}.` compiles to
    /// `{put_tuple2,{x,0},{list,[{x,0},{x,0}]}}`.
    PutTuple {
        elements: Vec<Operand>,
        destination: Operand,
    },
    /// Writes a copy of a tuple with each 1-based position set to its value.
    /// All values are read before the destination is written. The `copy`,
    /// `reuse`, and `inplace` hints and the tuple size are not kept: with
    /// immutable terms, each hint gives the same result.
    ///
    /// Encoding: opcode, hint atom, unsigned size, source, destination
    /// register, extended list, item count, then unsigned position and value
    /// operands in turn. Example: `gen_statem:enter/8` in the pinned
    /// `gen_statem.beam` has `[0xB5, 0x0A, 0x31, 0x70, 0x57, 0x94, 0x10, 0x33,
    /// 0x17, 0x20, 0x50, 0x57, 0x24, 0x30]`, which sets position 5 of the
    /// 7-tuple in `y9` to the value of `y2` and writes `x3`.
    ///
    /// Erlang: `rec(R, X) -> R#r{b = X, c = R#r.a}.` compiles to
    /// `{update_record,{atom,copy},4,{tr,{x,0},...},{x,0},{list,[3,{x,1},4,{x,2}]}}`.
    UpdateRecord {
        source: Operand,
        destination: Operand,
        /// Each 1-based position and its value operand.
        updates: Vec<(u32, Operand)>,
    },
    /// Copies the head and the tail of a nonempty list to two registers.
    ///
    /// Encoding: opcode, source, head register, tail register. Example:
    /// `gen_statem:replies/1` in the pinned `gen_statem.beam` has
    /// `[0x41, 0x03, 0x13, 0x23]`, which writes the head of `x0` to `x1` and
    /// its tail to `x2`.
    ///
    /// Erlang: `lst([H | T]) -> {H, T}.` compiles to
    /// `{get_list,{x,0},{x,1},{x,0}}`.
    GetList {
        source: Operand,
        head: Operand,
        tail: Operand,
    },
    /// Copies the head of a nonempty list to a register.
    ///
    /// Encoding: opcode, source, destination register. Example:
    /// `gen_statem:system_code_change/4` in the pinned `gen_statem.beam` has
    /// `[0xA2, 0x53, 0x73]`, which writes the head of `x5` to `x7`.
    ///
    /// Erlang: `hd1([H | _]) -> H.` compiles to `{get_hd,{x,0},{x,0}}`.
    GetHead {
        source: Operand,
        destination: Operand,
    },
    /// Copies the tail of a nonempty list to a register.
    ///
    /// Encoding: opcode, source, destination register. Example:
    /// `proc_lib:trans_init/3` in the pinned `proc_lib.beam` has
    /// `[0xA3, 0x43, 0x33]`, which writes the tail of `x4` to `x3`.
    ///
    /// Erlang: `rest([_ | T]) -> T.` compiles to `{get_tl,{x,0},{x,0}}`.
    GetTail {
        source: Operand,
        destination: Operand,
    },
    /// Copies the zero-based tuple element at `index` to a register.
    ///
    /// Encoding: opcode, tuple register, unsigned index, destination register.
    /// Example: `[0x42, 0x03, 0x10, 0x13]` copies element 1 of `x0` to `x1`.
    ///
    /// Erlang: `{Service, Monitor} = spawn_monitor(M, F, A)` reads `Monitor`
    /// with `{get_tuple_element,{x,0},1,{y,2}}`.
    GetTupleElement {
        tuple: Operand,
        index: u32,
        destination: Operand,
    },
    /// Jumps to `fail_label` when the operands are not exactly equal.
    ///
    /// Encoding: opcode, fail label, two operands. A typed register (extended
    /// subtag 5) reads as its register. Example: `[0x2B, 0x35, 0x13, 0x12]`
    /// compares `x1` with atom 1 and fails to label 3.
    ///
    /// Erlang: `receive {io_reply, Reference, ok} -> io_ok end` compiles to
    /// `{test,is_eq_exact,{f,14},[{x,0},{atom,ok}]}`.
    TestExactEqual {
        fail_label: u32,
        left: Operand,
        right: Operand,
    },
    /// Jumps to `fail_label` when the operands are not exactly equal, with
    /// the operands that the loader classified one time.
    ///
    /// The loader selects this form from `test is_eq_exact` when both
    /// operands are a register or an immediate term and at least one operand
    /// is a register, so this form reads one register and one immediate, or
    /// two registers. It is the operand form that the pinned ERTS selects at
    /// load time: `is_eq_exact Lbl R=xy C=ia => i_is_eq_exact_immed Lbl R C`
    /// and `... C=q => i_is_eq_exact_literal ...`
    /// (`erts/emulator/beam/emu/ops.tab:506-507`). The run loop reads a
    /// checked register slot or the stored term, so it matches no
    /// [`Operand`] for this instruction.
    ///
    /// Erlang: the entry clause `loop(0) -> ok;` of
    /// `fixtures/dispatch/dispatch_probe.erl` compiles to
    /// `{test,is_eq_exact,{f,6},[{x,0},{integer,0}]}`, and the loader
    /// selects this form for it.
    TestExactEqualOperands {
        fail_label: u32,
        left: TestOperand,
        right: TestOperand,
    },
    /// Jumps to `fail_label` when the operands are not equal in the ERTS
    /// arithmetic order (`is_eq`, opcode 41): `1` equals `1.0`, and `0.0`
    /// equals `-0.0`.
    ///
    /// Encoding: opcode, fail label, two operands. A typed register (extended
    /// subtag 5) reads as its register. Example: `compare_probe:eq/2` in
    /// `fixtures/compare/compare_probe.beam` has `[0x29, 0x0D, 0x19, 0x03,
    /// 0x13]`, which fails to label 25 unless `x0` equals `x1`.
    ///
    /// Erlang: the guard `CtrlSync==0` of `logger_h_common:handle_load/2`
    /// (`logger_h_common.erl:250`) compiles to an `is_eq` test.
    /// `eq(A, B) when A == B -> id(true);` compiles to
    /// `{test,is_eq,{f,25},[{x,0},{x,1}]}`.
    TestEqual {
        fail_label: u32,
        left: Operand,
        right: Operand,
    },
    /// Jumps to `fail_label` when the operands are equal in the ERTS
    /// arithmetic order (`is_ne`, opcode 42), as for
    /// [`Instruction::TestEqual`].
    ///
    /// Encoding: opcode, fail label, two operands. A typed register (extended
    /// subtag 5) reads as its register. Example: `compare_probe:ne/2` in
    /// `fixtures/compare/compare_probe.beam` has `[0x2A, 0x0D, 0x1E, 0x03,
    /// 0x13]`, which fails to label 30 when `x0` equals `x1`.
    ///
    /// Erlang: `ne(A, B) when A /= B, A =/= x -> id(true);` compiles to
    /// `{test,is_ne,{f,30},[{x,0},{x,1}]}`, then an `is_eq_exact` test. A
    /// guard `A /= B` alone compiles to `is_eq`.
    /// `String.Tokenizer.continue/6` of Elixir has
    /// `{test,is_ne,{f,34},[{x,6},{integer,95}]}`.
    TestNotEqual {
        fail_label: u32,
        left: Operand,
        right: Operand,
    },
    /// Jumps to `fail_label` when the operands are exactly equal
    /// (`is_ne_exact`, opcode 44).
    ///
    /// Encoding: opcode, fail label, two operands. A typed register (extended
    /// subtag 5) reads as its register. Example: `user_drv:server/3` in the
    /// pinned `user_drv.beam` has `[0x2C, 0x0D, 0x9D, 0x03, 0x33]`, which
    /// fails to label 157 when `x0` equals `x3`.
    ///
    /// Erlang: the guard `element(1,Req) =:= User orelse element(1,Req) =:=
    /// Curr` of `user_drv:server/3` (`user_drv.erl:542`) compiles to
    /// `{test,is_ne_exact,{f,157},[{x,0},{x,3}]}`. When the first test is
    /// true, the jump skips the `orelse` test.
    TestExactNotEqual {
        fail_label: u32,
        left: Operand,
        right: Operand,
    },
    /// Jumps to `fail_label` unless the left operand is less than the right
    /// operand (`is_lt`, opcode 39) in the ERTS term order. Two different
    /// references give a runtime error.
    ///
    /// Encoding: opcode, fail label, two operands. A typed register (extended
    /// subtag 5) reads as its register. Example: `compare_probe:width/1` in
    /// `fixtures/compare/compare_probe.beam` has `[0x27, 0xF5, 0x03, 0x01]`,
    /// which fails to label 15 unless `x0` is less than the integer 0.
    ///
    /// Erlang: `width(F) when F < 0 -> left;` compiles to
    /// `{test,is_lt,{f,15},[{x,0},{integer,0}]}`, as the guard of
    /// `io_lib_format:field_width/3` (`io_lib_format.erl:259`) does, which
    /// compares the atom `none` with 0.
    TestLess {
        fail_label: u32,
        left: Operand,
        right: Operand,
    },
    /// Jumps to `fail_label` when the left operand is less than the right
    /// operand (`is_ge`, opcode 40) in the ERTS term order. Two different
    /// references give a runtime error.
    ///
    /// Encoding: opcode, fail label, two operands. A typed register (extended
    /// subtag 5) reads as its register. Example: `[0x28, 0x0D, 0x16, 0x57,
    /// 0x13, 0x20, 0x21]` fails to label 22 when `x1` is less than the
    /// integer 2.
    ///
    /// Erlang: the guard `when tuple_size(X) >= 2` in
    /// `fixtures/group/group_probe.erl` compiles to
    /// `{test,is_ge,{f,22},[{tr,{x,1},{t_integer,{0,16777215}}},{integer,2}]}`.
    TestGreaterEqual {
        fail_label: u32,
        left: Operand,
        right: Operand,
    },
    /// Jumps to `fail_label` unless the value is a tuple with `arity`
    /// elements and the first element is `tag`.
    ///
    /// Encoding: opcode, fail label, operand, unsigned arity, tag atom.
    /// Example: `[0x9F, 0x35, 0x03, 0x30, 0x12]` tests `x0` for a 3-tuple
    /// tagged with atom 1.
    ///
    /// Erlang: `receive {io_reply, Reference, ok} -> io_ok end` compiles to
    /// `{test,is_tagged_tuple,{f,14},[{x,0},3,{atom,io_reply}]}`.
    TestTaggedTuple {
        fail_label: u32,
        value: Operand,
        arity: u32,
        tag: Term,
    },
    /// Jumps to `fail_label` when `source` is not a binary or a match
    /// context (`bs_start_match3`, opcode 166). Else writes a match context
    /// to `destination`: a new one at the start of a binary, or the same
    /// match context.
    ///
    /// Encoding: opcode, fail label, source, unsigned live count, destination
    /// register. The decoder skips the live count, because PBeam has no
    /// process heap to collect. Example: `binary_match_probe:kind/1` in
    /// `fixtures/binary_match/binary_match_probe.beam` has `[0xA6, 0x55,
    /// 0x03, 0x10, 0x03]`, which fails to label 5 unless `x0` is a binary,
    /// and else writes the match context to `x0`.
    ///
    /// Erlang: `kind(<<_/utf8, _/binary>>) -> utf8;` compiles to
    /// `{test,bs_start_match3,{f,5},1,[{x,0}],{x,0}}`, as
    /// `unicode_util:cp/1` does for its argument.
    ///
    /// The decoder also makes this instruction from `bs_start_match4`
    /// (opcode 170) with a fail label or the atom `no_fail`, as the ERTS
    /// loader makes `bs_start_match3` from it (JIT jit/arm/ops.tab:
    /// 1036-1039; emu ops.tab:1243-1246). `no_fail` gives the fail label 0:
    /// the compiler uses it only for a source that is a binary or a match
    /// context (beam_ssa_codegen.erl:2303-2306).
    ///
    /// Encoding of `bs_start_match4`: opcode, fail label or atom, unsigned
    /// live count, source register, destination register. Example:
    /// `bs_get_probe:split/2` in `fixtures/binary_match/bs_get_probe.beam`
    /// has `[0xAA, 0x32, 0x20, 0x03, 0x03]`: atom 3 (`no_fail`), live
    /// count 2, and the source and destination `x0`.
    ///
    /// Erlang: `case Bin of <<Head:N/binary, Tail/binary>> -> ...` after
    /// the guard `is_binary(Bin)` compiles to
    /// `{bs_start_match4,{atom,no_fail},2,{x,0},{x,0}}`.
    BinaryMatchStart {
        fail_label: u32,
        source: Operand,
        destination: Operand,
    },
    /// Continues a match with the match context in `source`
    /// (`bs_start_match4` with the atom `resume`, opcode 170). On a 64-bit
    /// system, the ERTS loader removes it when `source` is `destination`,
    /// and else makes it `move` (JIT jit/arm/ops.tab:1041-1045; emu
    /// ops.tab:1248-1252). The compiler uses it only for a source that is
    /// a match context (beam_ssa_codegen.erl:2300-2302).
    ///
    /// Encoding: as `BinaryMatchStart` from `bs_start_match4`. Example:
    /// `bs_get_probe:nibbles/2` has `[0xAA, 0xB2, 0x20, 0x03, 0x03]`: atom
    /// 11 (`resume`), live count 2, and the source and destination `x0`.
    ///
    /// Erlang: a local function that gets the match context of its caller,
    /// such as `nibbles(<<N:4, Rest/bits>>, Acc)` after the call
    /// `nibbles(Rest, [])` with `Rest` of a match, compiles to
    /// `{bs_start_match4,{atom,resume},2,{x,0},{x,0}}`.
    BinaryMatchResume {
        source: Operand,
        destination: Operand,
    },
    /// Writes the bits after the position of the match context in
    /// `context` to `destination` as a binary, and keeps the position
    /// (`bs_get_tail`, opcode 165).
    ///
    /// Encoding: opcode, context, destination register, unsigned live
    /// count. The decoder skips the live count. Example:
    /// `bs_get_probe:split/2` has `[0xA5, 0x03, 0x03, 0x20]`, which writes
    /// the rest of the match context in `x0` to `x0`.
    ///
    /// Erlang: `<<Head:N/binary, Tail/binary>>` compiles to
    /// `{bs_get_tail,{x,0},{x,0},2}` for `Tail`.
    BinaryGetTail {
        context: Operand,
        destination: Operand,
    },
    /// Reads `Size * unit` bits from the match context in `context` and
    /// writes them to `destination` as a binary, or jumps to `fail_label`
    /// (`bs_get_binary2`, opcode 119). `size` is a register or a literal.
    ///
    /// Encoding: opcode, fail label, context, unsigned live count, size,
    /// unsigned unit, unsigned flags, destination register. The decoder
    /// skips the live count and the flags, as the ERTS loader does (JIT
    /// jit/arm/ops.tab:1002-1003). Example: `bs_get_probe:split/2` has
    /// `[0x77, 0x35, 0x57, 0x03, 0x10, 0x20, 0x13, 0x80, 0x00, 0x13]`: fail
    /// label 3, the typed register `x0`, live count 2, size `x1`, unit 8,
    /// flags 0, and the destination `x1`. The size `all` decodes as
    /// `Instruction::BinaryMatch`, as the ERTS loader loads it.
    ///
    /// Erlang: `<<Head:N/binary, Tail/binary>>` compiles to
    /// `{test,bs_get_binary2,{f,3},2,[{tr,{x,0},...},{x,1},8,{field_flags,[]}],{x,1}}`
    /// for `Head`.
    BinaryGetBinary {
        fail_label: u32,
        context: Operand,
        size: Operand,
        unit: u64,
        destination: Operand,
    },
    /// Reads a float of `Size * unit` bits from the match context in
    /// `context` and writes it to `destination`, or jumps to `fail_label`
    /// (`bs_get_float2`, opcode 118). `size` is a register or a literal.
    /// `little` is `true` for a little-endian segment.
    ///
    /// Encoding: opcode, fail label, context, unsigned live count, size,
    /// unsigned unit, unsigned flags, destination register. The decoder
    /// skips the live count. It reads the `little` flag (2) and the
    /// `native` flag (16) of the flags (`BSF_LITTLE` and `BSF_NATIVE` in
    /// erl_bits.h:462-465), and gives a native segment the byte order of
    /// the host, as the ERTS loader does (`$NativeEndian` in
    /// `gen.get_float2`, generators.tab:57-71). Example:
    /// `bs_float_probe:little/2` has
    /// `[0x76, 0x65, 0x57, 0x03, 0x20, 0x13, 0x10, 0x20, 0x13]`: fail label
    /// 6, the typed register `x0`, live count 2, size `x1`, unit 1, the
    /// flags 2 (`little`), and the destination `x1`. A literal size that
    /// is not a small integer, such as the bignum 2^64, stays in `size`:
    /// ERTS loads it as `i_bs_get_float2` (jit/arm/ops.tab:1009-1010, `s`
    /// includes literals per `beam_makeops:163`) and jumps to the fail label
    /// at run time (`emit_bs_get_field_size`, jit/arm/instr_bs.cpp:58-86).
    ///
    /// Erlang: `<<F:N/float-little, Rest/binary>>` compiles to
    /// `{test,bs_get_float2,{f,6},2,[{tr,{x,0},...},{x,1},1,{field_flags,[little]}],{x,1}}`
    /// for `F`.
    BinaryGetFloat {
        fail_label: u32,
        context: Operand,
        size: Operand,
        unit: u64,
        little: bool,
        destination: Operand,
    },
    /// Reads an integer of `Size * unit` bits from the match context in
    /// `context` and writes it to `destination`, or jumps to `fail_label`
    /// (`bs_get_integer2`, opcode 117). `size` is a register or a literal.
    /// `little` is `true` for a little-endian segment and `signed` for a
    /// signed segment.
    ///
    /// Encoding: opcode, fail label, context, unsigned live count, size,
    /// unsigned unit, unsigned flags, destination register. The decoder
    /// skips the live count. It reads the `little` flag (2), the `native`
    /// flag (16), and the `signed` flag (4) (`BSF_LITTLE`, `BSF_NATIVE`,
    /// and `BSF_SIGNED` in erl_bits.h:461-465), and gives a native segment
    /// the byte order of the host, as the ERTS loader does. ERTS loads a
    /// small fixed size below 64 bits as a `bs_match` integer command and
    /// each other size as `i_bs_get_integer2`
    /// (`emit_bs_get_integer2`, jit/arm/instr_bs.cpp:312-358). Both read
    /// the same bits. Example: `bs_match_probe:get_size/2` has
    /// `[0x75, 0x0D, 0x34, 0x57, 0x03, 0x30, 0x20, 0x13, 0x10, 0x00,
    /// 0x13]`: fail label 52, the typed register `x0`, live count 2, size
    /// `x1`, unit 1, flags 0, and the destination `x1`.
    ///
    /// Erlang: `<<Value:N/integer, Rest/binary>>` compiles to
    /// `{test,bs_get_integer2,{f,3},2,[{tr,{x,0},...},{x,1},1,{field_flags,[]}],{x,1}}`
    /// for `Value`.
    BinaryGetInteger {
        fail_label: u32,
        context: Operand,
        size: Operand,
        unit: u64,
        little: bool,
        signed: bool,
        destination: Operand,
    },
    /// Writes the bit position of the match context in `context` to
    /// `destination` as an integer (`bs_get_position`, opcode 167).
    ///
    /// Encoding: opcode, context, destination register, unsigned live
    /// count. The decoder skips the live count. Example:
    /// `binary_match_probe:is_usascii/1` has `[0xA7, 0x03, 0x13, 0x10]`,
    /// which writes the position of the match context in `x0` to `x1`.
    ///
    /// Erlang: a function with two clauses that match a binary, such as
    /// `prim_tty:is_usascii/1`, compiles to `{bs_get_position,{x,0},{x,1},1}`
    /// before the first clause.
    BinaryGetPosition {
        context: Operand,
        destination: Operand,
    },
    /// Sets the bit position of the match context in `context` to the
    /// integer in `position` (`bs_set_position`, opcode 168).
    ///
    /// Encoding: opcode, context, position. Example:
    /// `binary_match_probe:is_usascii/1` has `[0xA8, 0x03, 0x13]`, which
    /// sets the position of the match context in `x0` to the integer in
    /// `x1`.
    ///
    /// Erlang: the second clause of `prim_tty:is_usascii/1` starts with
    /// `{bs_set_position,{x,0},{x,1}}`.
    BinarySetPosition { context: Operand, position: Operand },
    /// Moves the position of the match context in `context` by `size *
    /// unit` bits, or jumps to `fail_label` (`bs_skip_bits2`, opcode 120).
    /// `size` is a register or a literal.
    ///
    /// Encoding: opcode, fail label, context, size, unsigned unit, unsigned
    /// flags. The decoder skips the flags, as the ERTS loader does
    /// (jit/arm/ops.tab:1018-1019). Example: `bs_match_probe:skip_size/2`
    /// has `[0x78, 0x0D, 0x2E, 0x57, 0x03, 0x30, 0x13, 0x80, 0x00]`: fail
    /// label 46, the typed register `x0`, the size `x1`, unit 8, and flags
    /// 0.
    ///
    /// Erlang: `<<_:N/unit:8, Rest/binary>>` compiles to
    /// `{test,bs_skip_bits2,{f,3},[{tr,{x,0},...},{tr,{x,1},...},8,{field_flags,[]}]}`.
    BinarySkipBits {
        fail_label: u32,
        context: Operand,
        size: Operand,
        unit: u64,
    },
    /// Reads one UTF-8 character from the match context in `context` and
    /// writes its code point to `destination` (`bs_get_utf8`, opcode 138),
    /// or skips it when `destination` is `None` (`bs_skip_utf8`, opcode
    /// 139), or jumps to `fail_label`.
    ///
    /// Encoding: opcode, fail label, context, unsigned live count, unsigned
    /// flags, and the destination register for `bs_get_utf8`. The decoder
    /// skips the live count and the flags, as the ERTS loader does (emu
    /// `ops.tab:1286-1289`). Example:
    /// `binary_match_probe:is_usascii/1` has `[0x8A, 0x0D, 0x10, 0x57,
    /// 0x03, 0x10, 0x20, 0x00, 0x23]`, which reads a character from the
    /// typed register `x0` to `x2` or fails to label 16.
    ///
    /// Erlang: `<<Char/utf8, T/binary>>` compiles to
    /// `{test,bs_get_utf8,{f,16},[{tr,{x,0},...},2,{field_flags,[]},{x,2}]}`,
    /// and `<<_/utf8, T/binary>>` to
    /// `{test,bs_skip_utf8,{f,3},[{tr,{x,0},...},1,{field_flags,[]}]}`.
    BinaryGetUtf8 {
        fail_label: u32,
        context: Operand,
        destination: Option<Operand>,
    },
    /// Reads one UTF-16 character from the match context in `context`
    /// (`bs_get_utf16`, opcode 140), or skips it when `destination` is
    /// `None` (`bs_skip_utf16`, opcode 141), or jumps to `fail_label`.
    /// `little` is `true` for a little-endian segment.
    ///
    /// Encoding: opcode, fail label, context, unsigned live count,
    /// unsigned flags, and the destination register for `bs_get_utf16`.
    /// The decoder skips the live count. It reads the `little` flag (2)
    /// and the `native` flag (16), and gives a native segment the byte
    /// order of the host, as the ERTS loader does (jit/arm/ops.tab:1080-
    /// 1081, `gen.get_utf16` and `gen.skip_utf16` in generators.tab).
    /// Example: `bs_utf_probe:get16_little/1` has `[0x8C, 0x65, 0x57, 0x03,
    /// 0x10, 0x10, 0x20, 0x13]`: fail label 6, the typed register `x0`,
    /// live count 1, flags 2 (`little`), and the destination `x1`.
    ///
    /// Erlang: `<<C/utf16-little, Rest/binary>>` compiles to
    /// `{test,bs_get_utf16,{f,6},[{tr,{x,0},...},1,{field_flags,[little]},{x,1}]}`.
    BinaryGetUtf16 {
        fail_label: u32,
        context: Operand,
        little: bool,
        destination: Option<Operand>,
    },
    /// Reads one UTF-32 character from the match context in `context`
    /// (`bs_get_utf32`, opcode 142), or skips it when `destination` is
    /// `None` (`bs_skip_utf32`, opcode 143), or jumps to `fail_label`.
    /// `little` is `true` for a little-endian segment.
    ///
    /// Encoding: the operands of `bs_get_utf16`. Example:
    /// `bs_utf_probe:skip32/1` has `[0x8F, 0x0D, 0x1B, 0x57, 0x03, 0x10,
    /// 0x10, 0x00]`: fail label 27, the typed register `x0`, live count 1,
    /// and flags 0. ERTS loads it as a 32-bit `bs_get_integer2` and
    /// `i_bs_validate_unicode_retract` (jit/arm/ops.tab:1086-1095).
    ///
    /// Erlang: `<<_/utf32, Rest/binary>>` compiles to
    /// `{test,bs_skip_utf32,{f,27},[{tr,{x,0},...},1,{field_flags,[]}]}`.
    BinaryGetUtf32 {
        fail_label: u32,
        context: Operand,
        little: bool,
        destination: Option<Operand>,
    },
    /// Runs the commands of `bs_match` (opcode 182) in order on the match
    /// context in `context`. A command that fails jumps to `fail_label`.
    ///
    /// Encoding: opcode, fail label, context, then an extended list with
    /// each command name atom and its operands. Example:
    /// `binary_match_probe:is_usascii/1` has `[0xB6, 0x0D, 0x10, 0x03,
    /// 0x17, 0x30, 0x82, 0x00, 0x80]`: the list has 3 items, the atom 8
    /// (`ensure_at_least`), and the unsigned integers 0 and 8.
    ///
    /// Erlang: `<<Char/utf8, T/binary>>` compiles to
    /// `{bs_match,{f,16},{x,0},{commands,[{ensure_at_least,0,8}]}}` after
    /// `bs_get_utf8`.
    BinaryMatch {
        fail_label: u32,
        context: Operand,
        commands: Vec<BinaryMatchCommand>,
        /// `true` when a `test_heap` follows, as the ERTS loader fuses the
        /// two into `i_bs_match_test_heap` (JIT jit/arm/ops.tab:962-963).
        /// The fused heap need changes how the JIT groups the reads, and
        /// thus the position after a failed `=:=` command.
        test_heap: bool,
    },
    /// Compares the `bits` bits after the position of the match context in
    /// `context` with the first `bits` bits of `text`, moves the position
    /// past them, or jumps to `fail_label` (`bs_match_string`, opcode
    /// 132).
    ///
    /// Encoding: opcode, fail label, context, unsigned `Bits`, unsigned
    /// offset of the string in the `StrT` chunk. The string has
    /// `(Bits + 7) / 8` bytes (beam_disasm.erl:1189-1197). Example:
    /// `bs_match_probe:match_string/1` has `[0x84, 0x0D, 0x25, 0x03, 0x08,
    /// 0x68, 0x00]`: fail label 37, the register `x0`, 104 bits, and the
    /// string at the offset 0 of the `StrT` chunk.
    ///
    /// Erlang: a string above 64 bits in a binary pattern, such as
    /// `<<"dobe.photoshop", Rest/binary>>`, compiles to
    /// `{test,bs_match_string,{f,3},[{tr,{x,0},...},112,{string,<<"dobe.photoshop">>}]}`.
    /// The compiler writes the instruction for such a string, because a
    /// `{:commands,_}` list holds the value of a `=:=` command in one word
    /// (beam_ssa_pre_codegen.erl:631-637; beam_ssa_codegen.erl:2858-2862).
    BinaryMatchString {
        fail_label: u32,
        context: Operand,
        bits: u64,
        text: Vec<u8>,
    },
    /// Makes an empty writable binary in `x0` (`bs_init_writable`, opcode
    /// 133). ERTS reads the size of the buffer from `x0`, 1024 bytes when
    /// it is not a small integer that is not negative
    /// (`erts_bs_init_writable`, erl_bits.c:2020-2049), and keeps no other
    /// register live (JIT `emit_bs_init_writable`,
    /// jit/arm/instr_bs.cpp:985-1000). The size is a capacity hint, so the
    /// binary has no byte.
    ///
    /// Encoding: the opcode alone, `[0x85]`.
    ///
    /// Erlang: `bytes(N) -> bytes(N, <<>>).` with a loop that appends to
    /// the accumulator compiles to `{move,{integer,256},{x,0}}` and
    /// `bs_init_writable`.
    BinaryInitWritable,
    /// Builds a binary from `segments` in order and writes it to
    /// `destination` (`bs_create_bin`, opcode 177). A `binary` segment with
    /// a term that is not a binary jumps to `fail_label`, or raises
    /// `error:badarg` when `fail_label` is 0.
    ///
    /// Encoding: opcode, fail label, unsigned `Alloc`, `Live`, and `Unit`,
    /// destination register, then an extended list with six items for each
    /// segment. The decoder skips `Alloc`, `Live`, and `Unit`: PBeam has no
    /// process heap, and the ERTS JIT loader drops `Unit` (JIT
    /// jit/arm/generators.tab:441-454). Example: `bs_create_bin_probe:prefix/1`
    /// in `fixtures/bs_create_bin/bs_create_bin_probe.beam` has `[0xB1,
    /// 0x05, 0x00, 0x10, 0x80, 0x03, 0x17, 0xC0, 0x32, 0x00, 0x80, 0x02,
    /// 0x00, 0x71, 0x42, 0x20, 0x80, 0x02, 0x03, 0x52]`: fail label 0,
    /// `Alloc` 0, `Live` 1, `Unit` 8, destination `x0`, and a list of 12
    /// items. The first segment is atom 3 (`string`), segment 0, unit 8,
    /// `nil` flags, offset 0 in the `StrT` chunk, and the integer length 7.
    /// The second segment is atom 4 (`binary`), segment 2, unit 8, `nil`
    /// flags, `x0`, and atom 5 (`all`).
    ///
    /// Erlang: `prefix(X) -> <<"count: ", X/binary>>.`, and the Elixir
    /// expression `"count: " <> X`, compile to
    /// `{bs_create_bin,{f,0},0,1,8,{x,0},{list,[{atom,string},0,8,nil,{string,<<"count: ">>},{integer,7},{atom,binary},2,8,nil,{x,0},{atom,all}]}}`.
    BinaryCreate {
        fail_label: u32,
        destination: Operand,
        segments: Vec<BinarySegment>,
    },
    /// Converts `source` to a float in a float register (`fconv`, opcode
    /// 97). An integer converts to the nearest float, a float stays the
    /// same, and each other term raises `error:badarith` (emu
    /// float_instrs.tab:38-52; JIT `emit_fconv`, jit/arm/instr_float.cpp:96-135).
    ///
    /// Encoding: opcode, source, float register (extended subtag 2, then
    /// the register number). Example: `float_test_probe:half/1` in
    /// `fixtures/float_test/float_test_probe.beam` has `[0x61, 0x03, 0x27,
    /// 0x00]`, which converts `x0` to `fr0`.
    ///
    /// Erlang: `half(X) -> X / 2.` compiles to `{fconv,{x,0},{fr,0}}`, as
    /// the `/` operators of `string` and `erl_scan` do.
    FloatConvert { source: Operand, register: u32 },
    /// Moves a float term to a float register (`fmove`, opcode 96, which
    /// ERTS loads as `fload`). The compiler moves only a float: a type test
    /// or a float literal comes first. Example: `[0x60, 0x03, 0x27, 0x00]`
    /// is `{fmove,{x,0},{fr,0}}` in `float_probe:registers/2`.
    FloatLoad { source: Operand, register: u32 },
    /// Moves a float register to an X or a Y register as a float term
    /// (`fmove`, opcode 96, which ERTS loads as `fstore`). Example:
    /// `[0x60, 0x27, 0x20, 0x03]` is `{fmove,{fr,2},{x,0}}` in
    /// `float_probe:registers/2`.
    FloatStore { register: u32, destination: Operand },
    /// Adds, subtracts, multiplies, or divides two float registers into a
    /// float register (`fadd`, `fsub`, `fmul`, `fdiv`, opcodes 98 to 101).
    /// A result that is not finite raises `error:badarith` (JIT
    /// `emit_float_instr` and `emit_i_fdiv`, jit/arm/instr_float.cpp:137-205).
    /// The fail label is always 0: ERTS loads only that form (emu
    /// ops.tab:1329-1332). Example: `[0x62, 0x05, 0x27, 0x00, 0x27, 0x10,
    /// 0x27, 0x20]` is `{bif,fadd,{f,0},[{fr,0},{fr,1}],{fr,2}}` in
    /// `float_probe:registers/2`.
    FloatArith {
        operation: FloatOperation,
        left: u32,
        right: u32,
        destination: u32,
    },
    /// Negates a float register into a float register (`fnegate`, opcode
    /// 102). The fail label is always 0. Example: `[0x66, 0x05, 0x27, 0x00,
    /// 0x27, 0x60]` is `{bif,fnegate,{f,0},[{fr,0}],{fr,6}}` in
    /// `float_probe:registers/2`.
    FloatNegate { source: u32, destination: u32 },
    /// Sends `x1` to the destination in `x0`, then writes the message to
    /// `x0`. The destination is a pid, a registered name, `{Name, Node}`, or
    /// a reference. A name that no process has, a tuple that is not two
    /// atoms, and each other term raise `error:badarg`.
    ///
    /// Encoding: `[0x14]`.
    ///
    /// Erlang: `Pid ! Message` compiles to `send`.
    Send,
    /// Removes the current message from the mailbox and moves the save
    /// pointer to the first message.
    ///
    /// Encoding: `[0x15]`.
    ///
    /// Erlang: a matched `receive` clause compiles to `remove_message`.
    RemoveMessage,
    /// Writes the message at the save pointer to a register, or jumps to
    /// `fail_label` when no message is left.
    ///
    /// Encoding: opcode, fail label, destination register. Example:
    /// `[0x17, 0x35, 0x03]` writes `x0` or fails to label 3.
    ///
    /// Erlang: `receive ... end` starts with `{loop_rec,{f,15},{x,0}}`.
    LoopReceive {
        fail_label: u32,
        destination: Operand,
    },
    /// Moves the save pointer past the current message and jumps to the
    /// label.
    ///
    /// Encoding: opcode, label. Example: `[0x18, 0x35]` jumps to label 3.
    ///
    /// Erlang: a `receive` clause that does not match compiles to
    /// `{loop_rec_end,{f,3}}`.
    LoopReceiveEnd(u32),
    /// Suspends the process until a message arrives, then continues at the
    /// label.
    ///
    /// Encoding: opcode, label. Example: `[0x19, 0x35]` continues at label 3.
    ///
    /// Erlang: `receive ... end` without `after` ends with `{wait,{f,3}}`.
    Wait(u32),
    /// Suspends the process until a message arrives or the timeout in
    /// `source` expires. A message continues at `label`; an expired timeout
    /// continues at the next instruction.
    ///
    /// Encoding: opcode, label, timeout operand. Example: `gen_statem:loop/3`
    /// in the pinned `gen_statem.beam` has `[0x1A, 0x0D, 0xCC, 0x04]`, which
    /// waits with the timeout in `y0` and continues at label 204.
    ///
    /// Erlang: `wait(T) -> receive M -> M after T -> timeout end.` compiles to
    /// `{wait_timeout,{f,16},{y,0}}`.
    WaitTimeout { label: u32, source: Operand },
    /// Ends a receive after its timeout and moves the save pointer to the
    /// first message.
    ///
    /// Encoding: `[0x16]`.
    ///
    /// Erlang: `receive ... after 0 -> no_exit end` compiles to `timeout`
    /// before the `after` body.
    Timeout,
    /// Inserts a receive marker at the end of the mailbox and writes its
    /// identifier, or `undefined` when the mailbox is empty.
    ///
    /// Encoding: opcode, register. Example: `[0xAF, 0x14]` writes `y1`.
    ///
    /// Erlang: `Ref = make_ref(), Pid ! {request, Ref}, receive {Ref, R} -> R end`
    /// compiles to `{recv_marker_reserve,{y,1}}` before `make_ref/0`.
    ReceiveMarkerReserve(Operand),
    /// Binds a reserved receive marker to a reference.
    ///
    /// Encoding: opcode, marker register, reference register. Example:
    /// `[0xAD, 0x14, 0x03]` binds marker `y1` to the reference in `x0`.
    ///
    /// Erlang: the same pattern compiles to `{recv_marker_bind,{y,1},{x,0}}`
    /// after `make_ref/0`.
    ReceiveMarkerBind { marker: Operand, reference: Operand },
    /// Moves the save pointer past the receive marker bound to a reference.
    ///
    /// Encoding: opcode, register. Example: `[0xB0, 0x14]` uses `y1`.
    ///
    /// Erlang: the same pattern compiles to `{recv_marker_use,{y,1}}` before
    /// the `receive`.
    ReceiveMarkerUse(Operand),
    /// Clears the receive marker bound to a reference.
    ///
    /// Encoding: opcode, register. Example: `[0xAE, 0x14]` clears `y1`.
    ///
    /// Erlang: the same pattern compiles to `{recv_marker_clear,{y,1}}` when
    /// the `receive` matches.
    ReceiveMarkerClear(Operand),
    /// Starts a function that the module lists in `-nifs`. `name` and
    /// `arity` come from the `func_info` before it, as the ERTS loader marks
    /// the current function as a NIF (`jit/asm_load.c:651-661`,
    /// `emu/emu_load.c:1437-1445`). A `nif_start` without a `func_info`
    /// before it is a load error.
    ///
    /// The decoder puts this instruction directly after the entry label of
    /// the function, wherever `nif_start` is in the function. ERTS does not
    /// check the position of `nif_start` and does not run it; a NIF
    /// replaces the function from its entry (`beam/erl_nif.c:4922-4928`).
    /// So a body instruction before `nif_start` does not run for a function
    /// with a binding. `erlc` puts `nif_start` directly after the entry
    /// label, so compiled code keeps its order.
    ///
    /// Encoding: `[0xB3]`.
    ///
    /// Erlang: `-nifs([write_nif/2]).` makes `write_nif/2` in `prim_tty`
    /// start with `nif_start`, then `{move,{atom,undef},{x,0}}` and
    /// `{call_ext_only,1,{extfunc,erlang,nif_error,1}}`.
    NifStart { name: String, arity: u32 },
    /// Tests the reduction budget at the entry of a function, as the ERTS
    /// `i_test_yield` instruction (`jit/arm/instr_common.cpp:3086-3119`).
    /// It costs one reduction. When the budget is then 0 or less, the
    /// process switches out and continues after this instruction. The
    /// loader puts it directly after the entry label of each function, as
    /// the ERTS loader puts it in `func_prologue` (`jit/arm/ops.tab:929-938`).
    /// It has no BEAM opcode. A `nif_start` goes after it: a NIF call also
    /// costs one reduction and switches out first when the budget is then 0
    /// or less (`emit_call_nif_yield_helper` in
    /// `jit/arm/instr_bif.cpp:924-952`).
    ///
    /// Erlang: `a() -> ok.` loads as `Label(2)`, `TestYield`, and the body.
    TestYield,
    /// An instruction that has no runtime effect, such as `test_heap` or
    /// `init_yregs`.
    ///
    /// Erlang: `g(X) -> {X, X}.` compiles to `{test_heap,3,1}` first.
    Noop,
    /// Returns the value in `x0` from the active function. It costs one
    /// reduction, and the process switches out after the return when the
    /// budget is then less than 0 (`emit_dispatch_return` in
    /// `jit/arm/instr_call.cpp:49-54`).
    ///
    /// Encoding: `[0x13]`. Execution reports an error if the function has not
    /// set `x0`.
    ///
    /// Erlang: `a() -> ok.` ends with `return`.
    Return,
    /// Marks the end of the `Code` instruction stream.
    ///
    /// Encoding: `[0x03]`. The parser adds this internal sentinel after it
    /// reads the terminator. The executor stops at this sentinel.
    End,
}

#[derive(Debug, Clone)]
pub(crate) enum TestKind {
    Atom,
    Pid,
    /// `is_port`, opcode 51. Example: `[0x33, 0x55, 0x03]` fails to label 5
    /// unless `x0` is a port. Erlang: `Port when erlang:is_port(Port) ->
    /// Port;` in `erlang:open_port/2` (erlang.erl:7990) compiles to
    /// `{test,is_port,{f,5},[{x,0}]}`.
    Port,
    /// `is_reference`, opcode 50. Example: `[0x32, 0x35, 0x13]` fails to label 3
    /// unless `x1` is a reference. Erlang: `when is_reference(R)` in
    /// `fixtures/io_client/io_client_probe.erl`.
    Reference,
    /// `is_tuple`, opcode 57. Erlang: `when is_tuple(X)`; `io:put_chars/2`
    /// tests the request result with `{test,is_tuple,{f,10},[{x,0}]}`.
    Tuple,
    List,
    /// `is_nil`, opcode 52. Example: `[0x34, 0x0D, 0x73, 0x03]` in
    /// `gen_statem:replies/1` fails to label 115 unless `x0` is `[]`.
    /// Erlang: `nil([]) -> nil;` compiles to `{test,is_nil,{f,13},[{x,0}]}`.
    Nil,
    /// `is_nonempty_list`, opcode 56. Example: `[0x38, 0x0D, 0x75, 0x03]` in
    /// `gen_statem:replies/1` fails to label 117 unless `x0` is a list with a
    /// head. Erlang: `hd1([H | _]) -> H.` compiles to
    /// `{test,is_nonempty_list,{f,8},[{x,0}]}`.
    NonEmptyList,
    Binary,
    /// `is_bitstr`, opcode 129. Example: `[0x81, 0x55, 0x03]` in
    /// `bitstr_probe:kind/1` fails to label 5 unless `x0` is a bitstring.
    /// Erlang: `kind(Term) when is_bitstring(Term) -> Term;` compiles to
    /// `{test,is_bitstr,{f,5},[{x,0}]}`, as the consolidated
    /// `String.Chars.impl_for/1` of Elixir does. The test passes for each
    /// bitstring, also one whose size is not a multiple of 8.
    Bitstring,
    Map,
    /// `is_function`, opcode 77. Example: `[0x4D, 0x0D, 0x65, 0x57, 0x03,
    /// 0x40]` in `fun_probe:kind/1` fails to label 101 unless the typed
    /// register `x0` holds a fun. Erlang: `kind(F) when is_function(F) ->
    /// any;` compiles to `{test,is_function,{f,101},[{tr,{x,0},...}]}`.
    Function,
    /// `is_integer`, opcode 45. Example: `[0x2D, 0xB5, 0x03]` in
    /// `compare_probe:kind/1` fails to label 11 unless `x0` is an integer.
    /// Erlang: `kind(X) when is_integer(X) -> integer;` compiles to
    /// `{test,is_integer,{f,11},[{x,0}]}`, as the guard of
    /// `io_lib_format:field_value/2` (`io_lib_format.erl:271`) does.
    Integer,
    /// `is_float`, opcode 46. Example: `[0x2E, 0x55, 0x03]` in
    /// `float_test_probe:kind/1` fails to label 5 unless `x0` is a float.
    /// Erlang: `kind(Term) when is_float(Term) -> float;` compiles to
    /// `{test,is_float,{f,5},[{x,0}]}`, as `lists:thing_to_list/1`
    /// (lists.erl:1283-1286) does.
    Float,
    /// `is_number`, opcode 47. Example: `[0x2F, 0x0D, 0x1B, 0x03]` in
    /// `type_test_probe:number/1` fails to label 27 unless `x0` is an
    /// integer or a float. Erlang: `number(T) when is_number(T) -> yes;`
    /// compiles to `{test,is_number,{f,27},[{x,0}]}`, as the first clause
    /// of `telemetry:execute/3` (telemetry.erl:188) does. The test passes
    /// for a bignum, because `is_integer` covers it (emu
    /// `instrs.tab:999-1003`; JIT `jit/arm/instr_common.cpp:1403-1444`).
    Number,
    /// `is_boolean`, opcode 114. Example: `[0x72, 0xC5, 0x03]` in
    /// `compare_probe:kind/1` fails to label 12 unless `x0` is the atom
    /// `true` or `false`. Erlang: `kind(X) when is_boolean(X) -> boolean;`
    /// compiles to `{test,is_boolean,{f,12},[{x,0}]}`, as the guard of
    /// `standard_error:check_valid_opts/1` (`standard_error.erl:236`) does.
    Boolean,
    /// `is_any_native_record`, opcode 186 (genop.tab:722-724). Example:
    /// `[0xBA, 0x55, 0x03]` fails to label 5 unless `x0` is a native
    /// record. Erlang: `kind(T) when is_record(T) -> record;` compiles to
    /// `{test,is_record,{f,5},[{x,0}]}`, as the guard of
    /// `io_lib:write1/4` (`io_lib.erl:843`) and of
    /// `io_lib_pretty:print_length/7` (`io_lib_pretty.erl:938`) does. PBeam
    /// has no native record term, so the test fails for each term.
    AnyNativeRecord,
}

/// The source of a `put_record` instruction (`ops.tab:1641-1647`).
#[derive(Debug, Clone)]
pub(crate) enum PutRecord {
    /// Create a record of the module from its definition
    /// (`create_local_record`, `generators.tab:255-300`).
    Local(Arc<RecordDef>),
    /// Create a record from the active exported definition of `name` in
    /// `module` (`erl_create_native_record`, `erl_record.c:344-379`).
    Remote { module: Atom, name: Atom },
    /// Update the record in a register (`erl_update_native_record`,
    /// `erl_record.c:381-450`).
    Update(Operand),
}

/// The record name of a `get_record_field` instruction (`ops.tab:1669-1678`).
#[derive(Debug, Clone)]
pub(crate) enum RecordId {
    /// A record name of any module, or `'_'` for any record
    /// (`erl_get_local_record_field`, `erl_record.c:470-510`).
    Local(Atom),
    /// The exported record `name` of `module` (`erl_get_record_field`,
    /// `erl_record.c:512-568`).
    Remote { module: Atom, name: Atom },
}

/// One command of `bs_match` (genop.tab:685-696).
#[derive(Debug, Clone)]
pub(crate) enum BinaryMatchCommand {
    /// `{ensure_at_least,Size,Unit}`: at least `size` bits remain, and the
    /// bits after them are a multiple of `unit`. Example: `[0x82, 0x00,
    /// 0x80]` is `{ensure_at_least,0,8}` when atom 8 is `ensure_at_least`.
    EnsureAtLeast { size: u64, unit: NonZeroU64 },
    /// `{ensure_exactly,Size}`: exactly `size` bits remain. Example:
    /// `[0xF2, 0x00]` is `{ensure_exactly,0}` when atom 15 is
    /// `ensure_exactly`.
    EnsureExactly { size: u64 },
    /// `{integer,Live,Flags,Size,Unit,Dst}`: read an integer of `size`
    /// bits (`Size * Unit`) and write it to `destination`. `Flags` is `[]`
    /// or a literal list: `little` and `native` set `little`, and `signed`
    /// sets `signed` (`bs_get_flags`, jit/beam_jit_bs.cpp:255-285).
    /// Example: `[0x42, 0x10, 0x47, 0x00, 0x08, 0x10, 0x10, 0x13]` in
    /// `bs_match_probe:flagged/1` is `{integer,1,{literal,[little]},16,1,
    /// {x,1}}` when atom 4 is `integer` and literal 0 is `[little]`.
    Integer {
        size: u64,
        little: bool,
        signed: bool,
        destination: Operand,
    },
    /// `{binary,Live,Flags,Size,Unit,Dst}`: read a bitstring of `size`
    /// bits (`Size * Unit`) and write it to `destination`. ERTS does not
    /// read `Flags`. Example: `[0x0A, 0x13, 0x10, 0x02, 0x20, 0x80, 0x13]`
    /// in `bs_match_probe:binaries/1` is `{binary,1,{literal,[]},2,8,
    /// {x,1}}` when atom 19 is `binary`.
    Binary { size: u64, destination: Operand },
    /// `{'=:=',Live,Size,Value}`: the next `size` bits are the unsigned
    /// big-endian integer `value`. Example: `[0xA2, 0x02, 0x10, 0x10]` in
    /// `bs_match_probe:prefix/1` is `{'=:=',nil,1,1}` when atom 10 is
    /// `=:=`.
    Equal { size: u64, value: u64 },
    /// `{get_tail,Live,Unit,Dst}`: write the bits after the position to
    /// `destination` and keep the position. ERTS does not read `Unit`.
    /// Example: `[0xB2, 0x10, 0x80, 0x03]` in `bs_match_probe:tail/1` is
    /// `{get_tail,1,8,{x,0}}` when atom 11 is `get_tail`.
    GetTail { destination: Operand },
    /// `{skip,Size}`: move the position by `size` bits. Example: `[0x0A,
    /// 0x11, 0x80]` in `bs_match_probe:tail/1` is `{skip,8}` when atom 17
    /// is `skip`.
    Skip { size: u64 },
}

/// One segment of `bs_create_bin` (genop.tab:649-651).
#[derive(Debug, Clone)]
pub(crate) enum BinarySegment {
    /// A `string` segment: the bytes at its offset in the `StrT` chunk.
    /// Example: `[0x32, 0x00, 0x80, 0x02, 0x00, 0x71]` is
    /// `{atom,string},0,8,nil,{string,<<"count: ">>},{integer,7}` when atom
    /// 3 is `string` and `StrT` starts with `count: `.
    String(Vec<u8>),
    /// A `binary` segment of size `all`: the bitstring
    /// in `source`. `segment` is the segment number of the compiler, which
    /// the ERTS `error_info` gives. The size in bits of the binary must be a
    /// multiple of `unit` (`erts_bs_put_binary_all`, erl_bits.c:1321-1324).
    /// Example: `[0x42, 0x20, 0x80, 0x02, 0x03, 0x52]` is
    /// `{atom,binary},2,8,nil,{x,0},{atom,all}` when atom 4 is `binary` and
    /// atom 5 is `all`.
    BinaryAll {
        segment: u64,
        unit: u64,
        source: Operand,
    },
    /// A `binary` segment with a size: the first `size` times `unit` bits
    /// of the bitstring in `source`. The size pass checks `size` as the size
    /// of an `integer` segment, and the construct pass fails for a term that
    /// is not a bitstring or is shorter (`erts_bs_put_binary`,
    /// erl_bits.c:1285-1308; jit/arm/instr_bs.cpp:1639-1689, 2048-2071).
    /// Example: `{atom,binary},1,8,nil,{x,0},{x,1}` is `<<A:N/binary>>`.
    Binary {
        segment: u64,
        unit: u64,
        source: Operand,
        size: Operand,
    },
    /// An `append` segment of size `all` as the first segment: the binary
    /// in `source` is the start of the result. The compiler writes it for
    /// `<<A/binary, ...>>`, and the ERTS JIT runs it only as the first
    /// segment (`emit_i_bs_create_bin`, jit/arm/instr_bs.cpp:1811-1864).
    /// `unit` is the segment unit, which the size of the binary must be a
    /// multiple of. Example: `[0x62, 0x10, 0x80, 0x02, 0x03, 0x52]` is
    /// `{atom,append},1,8,nil,{x,0},{atom,all}` when atom 6 is `append`
    /// and atom 5 is `all`.
    Append {
        segment: u64,
        unit: u64,
        source: Operand,
    },
    /// A `private_append` segment of size `all`, the first segment: the
    /// writable binary in `source`, which the compiler makes with
    /// `bs_init_writable` and gives to no other term
    /// (`beam_ssa_private_append`). ERTS writes the later segments in its
    /// buffer (`erts_bs_private_append_checked`, erl_bits.c:1928-2018).
    /// Example: `[0x92, 0x10, 0x10, 0x02, 0x13, 0x52]` is
    /// `{atom,private_append},1,1,nil,{x,1},{atom,all}` when atom 9 is
    /// `private_append` and atom 5 is `all`.
    PrivateAppend { source: Operand },
    /// A `float` segment: the number in `source`, in `size` times `unit`
    /// bits. The flags give the byte order as for an `integer` segment.
    /// Example: `[0xA2, 0x10, 0x10, 0x02, 0x03, 0x13]` is
    /// `{atom,float},1,1,nil,{x,0},{x,1}` when atom 10 is `float`.
    Float {
        segment: u64,
        unit: u64,
        little: bool,
        source: Operand,
        size: Operand,
    },
    /// An `integer` segment: the integer in `source`, in `size` times
    /// `unit` bits. `little` is true when the flags hold `little` or
    /// `native`: the JIT loads `native` as `little` (`bs_get_flags`,
    /// jit/beam_jit_bs.cpp:255-285). The compiler writes a literal or a
    /// register `size`, and the unit 1 for a literal size. Example:
    /// `[0x72, 0x10, 0x10, 0x02, 0x03, 0x81]` is
    /// `{atom,integer},1,1,nil,{x,0},{integer,8}` when atom 7 is `integer`.
    Integer {
        segment: u64,
        unit: u64,
        little: bool,
        source: Operand,
        size: Operand,
    },
    /// A `utf8` segment: the UTF-8 bytes of the code point in `source`.
    /// The compiler writes the unit 0 and the size `undefined`. Example:
    /// `[0x82, 0x10, 0x00, 0x02, 0x03, 0x92]` is
    /// `{atom,utf8},1,0,nil,{x,0},{atom,undefined}` when atom 8 is `utf8`
    /// and atom 9 is `undefined`.
    Utf8 { segment: u64, source: Operand },
    /// A `utf16` segment: the UTF-16 code units of the code point in
    /// `source`. The compiler writes the unit 0 and the size `undefined`.
    /// ERTS writes two bytes for a code point below 0x10000 and four bytes
    /// for each other one, in the byte order of the flags
    /// (`erts_bs_put_utf16`, erl_bits.c:1221-1282; `am_utf16` in
    /// jit/arm/instr_bs.cpp:1751-1765, 2407-2427). Example:
    /// `[0x22, 0x10, 0x00, 0x02, 0x03, 0x32]` is
    /// `{atom,utf16},1,0,nil,{x,0},{atom,undefined}` when atom 2 is
    /// `utf16`, atom 0 is `nil`, and atom 3 is `undefined`.
    Utf16 {
        segment: u64,
        little: bool,
        source: Operand,
    },
    /// A `utf32` segment: the code point in `source` as a 32-bit integer
    /// in the byte order of the flags. The compiler writes the unit 0 and
    /// the size `undefined`. ERTS checks the value at the size pass and
    /// writes the integer at the construct pass (`am_utf32` in
    /// jit/arm/instr_bs.cpp:1766-1799, 2428-2452). Example:
    /// `[0x62, 0x10, 0x00, 0x02, 0x03, 0x32]` is
    /// `{atom,utf32},1,0,nil,{x,0},{atom,undefined}` when atom 6 is
    /// `utf32`.
    Utf32 {
        segment: u64,
        little: bool,
        source: Operand,
    },
    /// A segment that PBeam does not run yet: an `append` or
    /// `private_append` segment that is not the first segment or has a size
    /// other than `all`, or a `binary` segment with another size or a unit
    /// other than 1, 2, 4, or 8. It holds the segment type. Example:
    /// `[0x42, 0x20, 0x80, 0x02, 0x03, 0x81]` is
    /// `{atom,binary},2,8,nil,{x,0},{integer,8}` when atom 4 is `binary`.
    Unsupported(Atom),
}

/// A source or destination in the supported `move` instruction.
///
/// `Value` holds a supported immediate term. `X` uses compact register tag 3,
/// and `Y` uses compact register tag 4. For example, `move x0, y0` is encoded
/// as `[0x40, 0x03, 0x04]`. An integer operand with 9 or more payload
/// bytes, such as `{integer,18446744073709551616}`, encoded as
/// `[0xF9, 0x00, 0x01, 0, 0, 0, 0, 0, 0, 0, 0]`, is a `Value` that holds a
/// [`Term::BigInt`], as ERTS loads it as a bignum literal.
#[derive(Debug, Clone)]
pub(crate) enum Operand {
    Value(Term),
    UnsupportedLiteral(usize),
    X(u32),
    Y(u32),
}

/// One operand of [`Instruction::TestExactEqualOperands`], classified by the
/// loader.
///
/// `X` and `Y` are the register slots. `Value` is a supported immediate or
/// literal term. The loader classifies each operand one time, so the run
/// loop reads a checked register slot or the stored term and matches no
/// [`Operand`].
#[derive(Debug, Clone)]
pub(crate) enum TestOperand {
    X(u32),
    Y(u32),
    Value(Term),
}

/// Classify one operand of `test is_eq_exact` for the
/// [`Instruction::TestExactEqualOperands`] form.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(classify_test_operand(&Operand::X(0)), Some(TestOperand::X(0))));
/// ```
fn classify_test_operand(operand: &Operand) -> Option<TestOperand> {
    match operand {
        Operand::X(index) => Some(TestOperand::X(*index)),
        Operand::Y(index) => Some(TestOperand::Y(*index)),
        Operand::Value(value) => Some(TestOperand::Value(value.clone())),
        Operand::UnsupportedLiteral(_) => None,
    }
}

/// Return the classified operands of `test is_eq_exact`, or `None` when the
/// generic [`Instruction::TestExactEqual`] form must hold them.
///
/// The classified form reads one register and one immediate, or two
/// registers, so it needs at least one register operand.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(test_operands(&Operand::X(0), &Operand::Value(Term::nil())).is_some());
/// assert!(test_operands(&Operand::Value(Term::nil()), &Operand::Value(Term::nil())).is_none());
/// ```
fn test_operands(left: &Operand, right: &Operand) -> Option<(TestOperand, TestOperand)> {
    let left = classify_test_operand(left)?;
    let right = classify_test_operand(right)?;
    let has_register = matches!(left, TestOperand::X(_) | TestOperand::Y(_))
        || matches!(right, TestOperand::X(_) | TestOperand::Y(_));
    if has_register {
        Some((left, right))
    } else {
        None
    }
}

/// Decode the `Code` header and instruction stream. Return the
/// instructions, the number of labels, and the label of the `-on_load`
/// function. The `on_load` instruction (opcode 149, no operands) emits no
/// instruction: it marks the last label before it, as the ERTS loader keeps
/// the current label as the `on_load` entry
/// (`jit/arm/beam_asm_module.cpp:561-563`). The compiler puts it after the
/// entry label of the function. An `on_load` before any label is an error.
///
/// # Examples
///
/// ```rust,ignore
/// let mut code = vec![0, 0, 0, 16]; // sub-size
/// code.extend_from_slice(&[0; 16]); // instruction set, opcode max, labels, functions
/// code.push(0x03); // end
/// let (instructions, labels, on_load) = parse_code(&code, &AtomTable::new(), &[], &[], &[], &[], &[], &mut LineTable::default())?;
/// assert_eq!((labels, on_load), (0, None));
/// assert!(matches!(instructions.as_slice(), [Instruction::End]));
/// // [label 1, func_info, label 2, on_load] gives Some(2).
/// ```
#[expect(
    clippy::too_many_arguments,
    reason = "each argument is one decoded chunk that the code refers to"
)]
#[expect(
    clippy::too_many_lines,
    reason = "decode table: one arm for each BEAM opcode that reads its operands in the order of `genop.tab`; each arm is short"
)]
pub(super) fn parse_code(
    data: &[u8],
    atom_table: &AtomTable,
    atoms: &[Atom],
    imports: &[(String, String, u32)],
    literals: &[Option<Term>],
    lambdas: &[(Arc<Lambda>, usize)],
    strings: &[u8],
    records: &[Arc<RecordDef>],
    lines: &mut LineTable,
) -> Result<(Vec<Instruction>, u32, Option<u32>), BeamError> {
    if data.len() < CODE_HEADER_SIZE
        || read_u32(&data[..WORD_SIZE])? != CODE_HEADER_SUB_SIZE
        || read_u32(&data[WORD_SIZE..WORD_SIZE * 2])? != INSTRUCTION_SET_ERLANG
    {
        return Err(error("unsupported BEAM code header"));
    }
    let number_of_labels =
        read_u32(&data[CODE_LABEL_COUNT_OFFSET..CODE_LABEL_COUNT_OFFSET + WORD_SIZE])?;
    let declared = read_u32(&data[CODE_FUNCTION_COUNT_OFFSET..CODE_HEADER_SIZE])?;
    let declared_function_count = usize::try_from(declared).expect("a u32 fits in usize");
    let mut cursor = CODE_HEADER_SIZE;
    let mut instructions = Vec::new();
    let mut function_info_count = 0;
    let mut function = None;
    let mut on_load = None;
    // A `line` directly before `func_info` is the line of the function.
    let mut function_line = false;
    loop {
        let opcode = *data
            .get(cursor)
            .ok_or_else(|| error("truncated BEAM instruction stream"))?;
        cursor += 1;
        let instruction = match opcode {
            OPCODE_LABEL => {
                let label = read_operand_u32(data, &mut cursor, TAG_UNSIGNED)?;
                if label < FIRST_LABEL_NUMBER || label > number_of_labels {
                    return Err(error(format!(
                        "BEAM label {label} is outside declared range 1..={number_of_labels}"
                    )));
                }
                // The label directly after `func_info` is the function entry.
                // `TestYield` follows it, and a `nif_start` goes after that.
                let entry_label =
                    matches!(instructions.last(), Some(Instruction::FunctionInfo { .. }));
                instructions.push(Instruction::Label(label));
                if entry_label && let Some((_, _, entry)) = &mut function {
                    instructions.push(Instruction::TestYield);
                    *entry += 2;
                }
                continue;
            }
            OPCODE_ON_LOAD => {
                let label = instructions
                    .iter()
                    .rev()
                    .find_map(|instruction| match instruction {
                        Instruction::Label(label) => Some(*label),
                        _ => None,
                    });
                on_load =
                    Some(label.ok_or_else(|| error("BEAM on_load instruction has no label"))?);
                continue;
            }
            OPCODE_FUNCTION_INFO => {
                if !std::mem::take(&mut function_line) {
                    lines.function(instructions.len(), None)?;
                }
                function_info_count += 1;
                let module_index = read_operand_usize(data, &mut cursor, TAG_ATOM)?;
                let module = atom_table.name_of(atom_at(atoms, module_index)?).to_owned();
                let name_index = read_operand_usize(data, &mut cursor, TAG_ATOM)?;
                let name = atom_table.name_of(atom_at(atoms, name_index)?).to_owned();
                let arity = read_operand_u32(data, &mut cursor, TAG_UNSIGNED)?;
                function = Some((name.clone(), arity, instructions.len() + 1));
                Instruction::FunctionInfo {
                    module,
                    name,
                    arity,
                }
            }
            OPCODE_RETURN => {
                // The ERTS loader transforms `call_fun Arity | deallocate D |
                // return` into `i_call_fun_last Arity D`, and `call_fun2` in
                // the same way (`jit/arm/ops.tab:868-880`; the emu loader
                // turns `call_fun2` into `call_fun` first,
                // `emu/generators.tab:26-52`). The fun call then replaces the
                // frame of its caller.
                if let [
                    ..,
                    Instruction::CallFun {
                        tail, deallocate, ..
                    },
                    Instruction::Deallocate { stack_size },
                ] = instructions.as_mut_slice()
                {
                    *tail = true;
                    *deallocate = *stack_size;
                    instructions.pop();
                    continue;
                }
                Instruction::Return
            }
            OPCODE_CALL => {
                let arity = read_operand_u32(data, &mut cursor, TAG_UNSIGNED)?;
                let label = read_operand_u32(data, &mut cursor, TAG_LABEL)?;
                Instruction::Call {
                    arity,
                    function: label,
                    target: label,
                }
            }
            OPCODE_CALL_ONLY => {
                let arity = read_operand_u32(data, &mut cursor, TAG_UNSIGNED)?;
                let label = read_operand_u32(data, &mut cursor, TAG_LABEL)?;
                Instruction::CallOnly {
                    arity,
                    function: label,
                    target: label,
                }
            }
            OPCODE_JUMP => Instruction::Jump(read_operand_u32(data, &mut cursor, TAG_LABEL)?),
            OPCODE_SELECT_VAL => {
                let value = read_move_operand(data, &mut cursor, atoms, literals)?;
                let fail_label = read_operand_u32(data, &mut cursor, TAG_LABEL)?;
                let pair_count = read_pair_count(data, &mut cursor)?;
                let mut choices = Vec::with_capacity(pair_count);
                for _ in 0..pair_count {
                    let choice = read_move_operand(data, &mut cursor, atoms, literals)?;
                    let Operand::Value(choice) = choice else {
                        return Err(error("unsupported BEAM selection value"));
                    };
                    let target = read_operand_u32(data, &mut cursor, TAG_LABEL)?;
                    choices.push((choice, target));
                }
                Instruction::SelectValue {
                    value,
                    fail_label,
                    choices: prepare_select_value_choices(choices, atom_table),
                }
            }
            OPCODE_SELECT_TUPLE_ARITY => {
                let value = read_move_operand(data, &mut cursor, atoms, literals)?;
                let fail_label = read_operand_u32(data, &mut cursor, TAG_LABEL)?;
                let pair_count = read_pair_count(data, &mut cursor)?;
                let mut choices = Vec::with_capacity(pair_count);
                for _ in 0..pair_count {
                    let arity = read_operand_u32(data, &mut cursor, TAG_UNSIGNED)?;
                    let target = read_operand_u32(data, &mut cursor, TAG_LABEL)?;
                    choices.push((arity, target));
                }
                Instruction::SelectTupleArity {
                    value,
                    fail_label,
                    choices,
                }
            }
            OPCODE_TEST_ARITY => Instruction::TestArity {
                fail_label: read_operand_u32(data, &mut cursor, TAG_LABEL)?,
                value: read_move_operand(data, &mut cursor, atoms, literals)?,
                arity: read_operand_u32(data, &mut cursor, TAG_UNSIGNED)?,
            },
            OPCODE_CALL_LAST => {
                let arity = read_operand_u32(data, &mut cursor, TAG_UNSIGNED)?;
                let label = read_operand_u32(data, &mut cursor, TAG_LABEL)?;
                let deallocate = read_operand_u32(data, &mut cursor, TAG_UNSIGNED)?;
                Instruction::CallLast {
                    arity,
                    function: label,
                    target: label,
                    deallocate,
                }
            }
            OPCODE_TEST_TAGGED_TUPLE => {
                let fail_label = read_operand_u32(data, &mut cursor, TAG_LABEL)?;
                let value = read_move_operand(data, &mut cursor, atoms, literals)?;
                let arity = read_operand_u32(data, &mut cursor, TAG_UNSIGNED)?;
                let tag = read_value(data, &mut cursor, atoms)?;
                Instruction::TestTaggedTuple {
                    fail_label,
                    value,
                    arity,
                    tag,
                }
            }
            OPCODE_FCONV => {
                let source = read_move_operand(data, &mut cursor, atoms, literals)?;
                let register = read_float_register(data, &mut cursor)?;
                Instruction::FloatConvert { source, register }
            }
            OPCODE_FMOVE => {
                if is_float_register(data, cursor)? {
                    let register = read_float_register(data, &mut cursor)?;
                    let destination = read_register(data, &mut cursor)?;
                    Instruction::FloatStore {
                        register,
                        destination,
                    }
                } else {
                    let source = read_move_operand(data, &mut cursor, atoms, literals)?;
                    let register = read_float_register(data, &mut cursor)?;
                    Instruction::FloatLoad { source, register }
                }
            }
            OPCODE_FADD | OPCODE_FSUB | OPCODE_FMUL | OPCODE_FDIV => {
                read_zero_fail_label(data, &mut cursor)?;
                let left = read_float_register(data, &mut cursor)?;
                let right = read_float_register(data, &mut cursor)?;
                let destination = read_float_register(data, &mut cursor)?;
                let operation = match opcode {
                    OPCODE_FADD => FloatOperation::Add,
                    OPCODE_FSUB => FloatOperation::Subtract,
                    OPCODE_FMUL => FloatOperation::Multiply,
                    _ => FloatOperation::Divide,
                };
                Instruction::FloatArith {
                    operation,
                    left,
                    right,
                    destination,
                }
            }
            OPCODE_FNEGATE => {
                read_zero_fail_label(data, &mut cursor)?;
                let source = read_float_register(data, &mut cursor)?;
                let destination = read_float_register(data, &mut cursor)?;
                Instruction::FloatNegate {
                    source,
                    destination,
                }
            }
            OPCODE_BS_START_MATCH3 => {
                let fail_label = read_operand_u32(data, &mut cursor, TAG_LABEL)?;
                let source = read_move_operand(data, &mut cursor, atoms, literals)?;
                skip_live_registers(data, &mut cursor)?;
                Instruction::BinaryMatchStart {
                    fail_label,
                    source,
                    destination: read_register(data, &mut cursor)?,
                }
            }
            OPCODE_BS_START_MATCH4 => {
                // `Fail` is a label, or the atom `no_fail` or `resume`
                // (genop.tab:613-617). `None` is `resume`.
                let fail_label = if data
                    .get(cursor)
                    .is_some_and(|first| first & COMPACT_TAG_MASK == TAG_ATOM)
                {
                    let atom = atom_at(atoms, read_operand_usize(data, &mut cursor, TAG_ATOM)?)?;
                    if atom.is_static("no_fail") {
                        Some(0)
                    } else if atom.is_static("resume") {
                        None
                    } else {
                        return Err(error(format!(
                            "unsupported BEAM bs_start_match4 fail operand {}",
                            atom_table.name_of(atom)
                        )));
                    }
                } else {
                    Some(read_operand_u32(data, &mut cursor, TAG_LABEL)?)
                };
                skip_live_registers(data, &mut cursor)?;
                let source = read_register(data, &mut cursor)?;
                let destination = read_register(data, &mut cursor)?;
                match fail_label {
                    Some(fail_label) => Instruction::BinaryMatchStart {
                        fail_label,
                        source,
                        destination,
                    },
                    None => Instruction::BinaryMatchResume {
                        source,
                        destination,
                    },
                }
            }
            OPCODE_BS_GET_TAIL => {
                let context = read_move_operand(data, &mut cursor, atoms, literals)?;
                let destination = read_register(data, &mut cursor)?;
                skip_live_registers(data, &mut cursor)?;
                Instruction::BinaryGetTail {
                    context,
                    destination,
                }
            }
            OPCODE_BS_GET_BINARY2 => {
                let fail_label = read_operand_u32(data, &mut cursor, TAG_LABEL)?;
                let context = read_move_operand(data, &mut cursor, atoms, literals)?;
                skip_live_registers(data, &mut cursor)?;
                let size = read_move_operand(data, &mut cursor, atoms, literals)?;
                let unit = read_operand(data, &mut cursor, TAG_UNSIGNED)?;
                let _flags = read_operand(data, &mut cursor, TAG_UNSIGNED)?;
                let destination = read_register(data, &mut cursor)?;
                // ERTS loads the size `all` as `i_bs_get_binary_all2` (JIT
                // jit/arm/ops.tab:999-1000), which runs the `bs_match`
                // commands `{ensure_at_least,0,Unit}` and `get_tail`
                // (`emit_i_bs_get_binary_all2`, jit/arm/instr_bs.cpp:
                // 386-405). The pinned compiler makes it only with
                // `no_ssa_opt`.
                if matches!(&size, Operand::Value(Term::Atom(all)) if all.is_static("all")) {
                    let unit = NonZeroU64::new(unit)
                        .ok_or_else(|| error("BEAM bs_get_binary2 has unit 0"))?;
                    Instruction::BinaryMatch {
                        fail_label,
                        context,
                        commands: vec![
                            BinaryMatchCommand::EnsureAtLeast { size: 0, unit },
                            BinaryMatchCommand::GetTail { destination },
                        ],
                        test_heap: false,
                    }
                } else {
                    Instruction::BinaryGetBinary {
                        fail_label,
                        context,
                        size,
                        unit,
                        destination,
                    }
                }
            }
            OPCODE_BS_GET_FLOAT2 => {
                let fail_label = read_operand_u32(data, &mut cursor, TAG_LABEL)?;
                let context = read_move_operand(data, &mut cursor, atoms, literals)?;
                skip_live_registers(data, &mut cursor)?;
                let size = read_move_operand(data, &mut cursor, atoms, literals)?;
                let unit = read_operand(data, &mut cursor, TAG_UNSIGNED)?;
                let little = read_little_flag(data, &mut cursor)?;
                let destination = read_register(data, &mut cursor)?;
                Instruction::BinaryGetFloat {
                    fail_label,
                    context,
                    size,
                    unit,
                    little,
                    destination,
                }
            }
            OPCODE_BS_GET_INTEGER2 => {
                let fail_label = read_operand_u32(data, &mut cursor, TAG_LABEL)?;
                let context = read_move_operand(data, &mut cursor, atoms, literals)?;
                skip_live_registers(data, &mut cursor)?;
                let size = read_move_operand(data, &mut cursor, atoms, literals)?;
                let unit = read_operand(data, &mut cursor, TAG_UNSIGNED)?;
                let (little, signed) = read_segment_flags(data, &mut cursor)?;
                Instruction::BinaryGetInteger {
                    fail_label,
                    context,
                    size,
                    unit,
                    little,
                    signed,
                    destination: read_register(data, &mut cursor)?,
                }
            }
            OPCODE_BS_SKIP_BITS2 => {
                let fail_label = read_operand_u32(data, &mut cursor, TAG_LABEL)?;
                let context = read_move_operand(data, &mut cursor, atoms, literals)?;
                let size = read_move_operand(data, &mut cursor, atoms, literals)?;
                let unit = read_operand(data, &mut cursor, TAG_UNSIGNED)?;
                let _flags = read_operand(data, &mut cursor, TAG_UNSIGNED)?;
                Instruction::BinarySkipBits {
                    fail_label,
                    context,
                    size,
                    unit,
                }
            }
            OPCODE_BS_GET_POSITION => {
                let context = read_move_operand(data, &mut cursor, atoms, literals)?;
                let destination = read_register(data, &mut cursor)?;
                skip_live_registers(data, &mut cursor)?;
                Instruction::BinaryGetPosition {
                    context,
                    destination,
                }
            }
            OPCODE_BS_SET_POSITION => Instruction::BinarySetPosition {
                context: read_move_operand(data, &mut cursor, atoms, literals)?,
                position: read_move_operand(data, &mut cursor, atoms, literals)?,
            },
            OPCODE_BS_GET_UTF8 | OPCODE_BS_SKIP_UTF8 => {
                let fail_label = read_operand_u32(data, &mut cursor, TAG_LABEL)?;
                let context = read_move_operand(data, &mut cursor, atoms, literals)?;
                skip_live_registers(data, &mut cursor)?;
                let _flags = read_operand(data, &mut cursor, TAG_UNSIGNED)?;
                let destination = match opcode {
                    OPCODE_BS_GET_UTF8 => Some(read_register(data, &mut cursor)?),
                    _ => None,
                };
                Instruction::BinaryGetUtf8 {
                    fail_label,
                    context,
                    destination,
                }
            }
            OPCODE_BS_GET_UTF16 | OPCODE_BS_SKIP_UTF16 | OPCODE_BS_GET_UTF32
            | OPCODE_BS_SKIP_UTF32 => {
                let fail_label = read_operand_u32(data, &mut cursor, TAG_LABEL)?;
                let context = read_move_operand(data, &mut cursor, atoms, literals)?;
                skip_live_registers(data, &mut cursor)?;
                let little = read_little_flag(data, &mut cursor)?;
                let destination = match opcode {
                    OPCODE_BS_GET_UTF16 | OPCODE_BS_GET_UTF32 => {
                        Some(read_register(data, &mut cursor)?)
                    }
                    _ => None,
                };
                if matches!(opcode, OPCODE_BS_GET_UTF16 | OPCODE_BS_SKIP_UTF16) {
                    Instruction::BinaryGetUtf16 {
                        fail_label,
                        context,
                        little,
                        destination,
                    }
                } else {
                    Instruction::BinaryGetUtf32 {
                        fail_label,
                        context,
                        little,
                        destination,
                    }
                }
            }
            OPCODE_BS_MATCH => {
                let fail_label = read_operand_u32(data, &mut cursor, TAG_LABEL)?;
                let context = read_move_operand(data, &mut cursor, atoms, literals)?;
                let commands =
                    read_binary_match_commands(data, &mut cursor, atom_table, atoms, literals)?;
                Instruction::BinaryMatch {
                    fail_label,
                    context,
                    commands,
                    test_heap: false,
                }
            }
            OPCODE_BS_MATCH_STRING => {
                let fail_label = read_operand_u32(data, &mut cursor, TAG_LABEL)?;
                let context = read_move_operand(data, &mut cursor, atoms, literals)?;
                let bits = read_operand(data, &mut cursor, TAG_UNSIGNED)?;
                Instruction::BinaryMatchString {
                    fail_label,
                    context,
                    bits,
                    text: read_segment_string(data, &mut cursor, strings, bits)?,
                }
            }
            OPCODE_BS_INIT_WRITABLE => Instruction::BinaryInitWritable,
            OPCODE_BS_CREATE_BIN => {
                let fail_label = read_operand_u32(data, &mut cursor, TAG_LABEL)?;
                skip_allocation(data, &mut cursor)?;
                skip_live_registers(data, &mut cursor)?;
                let _unit = read_operand(data, &mut cursor, TAG_UNSIGNED)?;
                Instruction::BinaryCreate {
                    fail_label,
                    destination: read_register(data, &mut cursor)?,
                    segments: read_binary_segments(data, &mut cursor, atoms, literals, strings)?,
                }
            }
            OPCODE_TEST_EQ => Instruction::TestEqual {
                fail_label: read_operand_u32(data, &mut cursor, TAG_LABEL)?,
                left: read_move_operand(data, &mut cursor, atoms, literals)?,
                right: read_move_operand(data, &mut cursor, atoms, literals)?,
            },
            OPCODE_TEST_NE => Instruction::TestNotEqual {
                fail_label: read_operand_u32(data, &mut cursor, TAG_LABEL)?,
                left: read_move_operand(data, &mut cursor, atoms, literals)?,
                right: read_move_operand(data, &mut cursor, atoms, literals)?,
            },
            OPCODE_TEST_EXACT_EQ => {
                let fail_label = read_operand_u32(data, &mut cursor, TAG_LABEL)?;
                let left = read_move_operand(data, &mut cursor, atoms, literals)?;
                let right = read_move_operand(data, &mut cursor, atoms, literals)?;
                match test_operands(&left, &right) {
                    Some((left, right)) => Instruction::TestExactEqualOperands {
                        fail_label,
                        left,
                        right,
                    },
                    None => Instruction::TestExactEqual {
                        fail_label,
                        left,
                        right,
                    },
                }
            }
            OPCODE_TEST_EXACT_NE => Instruction::TestExactNotEqual {
                fail_label: read_operand_u32(data, &mut cursor, TAG_LABEL)?,
                left: read_move_operand(data, &mut cursor, atoms, literals)?,
                right: read_move_operand(data, &mut cursor, atoms, literals)?,
            },
            OPCODE_TEST_LT => Instruction::TestLess {
                fail_label: read_operand_u32(data, &mut cursor, TAG_LABEL)?,
                left: read_move_operand(data, &mut cursor, atoms, literals)?,
                right: read_move_operand(data, &mut cursor, atoms, literals)?,
            },
            OPCODE_TEST_GE => Instruction::TestGreaterEqual {
                fail_label: read_operand_u32(data, &mut cursor, TAG_LABEL)?,
                left: read_move_operand(data, &mut cursor, atoms, literals)?,
                right: read_move_operand(data, &mut cursor, atoms, literals)?,
            },
            OPCODE_TRY | OPCODE_CATCH => Instruction::Catch {
                register: read_operand_u32(data, &mut cursor, TAG_Y_REGISTER)?,
                label: read_operand_u32(data, &mut cursor, TAG_LABEL)?,
            },
            OPCODE_TRY_END => Instruction::TryEnd {
                register: read_operand_u32(data, &mut cursor, TAG_Y_REGISTER)?,
            },
            OPCODE_TRY_CASE => Instruction::TryCase {
                register: read_operand_u32(data, &mut cursor, TAG_Y_REGISTER)?,
            },
            OPCODE_CATCH_END => Instruction::CatchEnd {
                register: read_operand_u32(data, &mut cursor, TAG_Y_REGISTER)?,
            },
            OPCODE_RAISE => Instruction::Raise {
                trace: read_move_operand(data, &mut cursor, atoms, literals)?,
                value: read_move_operand(data, &mut cursor, atoms, literals)?,
            },
            OPCODE_BADMATCH => Instruction::TaggedError {
                tag: "badmatch",
                value: read_move_operand(data, &mut cursor, atoms, literals)?,
            },
            OPCODE_CASE_END => Instruction::TaggedError {
                tag: "case_clause",
                value: read_move_operand(data, &mut cursor, atoms, literals)?,
            },
            OPCODE_TRY_CASE_END => Instruction::TaggedError {
                tag: "try_clause",
                value: read_move_operand(data, &mut cursor, atoms, literals)?,
            },
            OPCODE_BADRECORD => Instruction::TaggedError {
                tag: "badrecord",
                value: read_move_operand(data, &mut cursor, atoms, literals)?,
            },
            OPCODE_IF_END => Instruction::IfEnd,
            OPCODE_RAW_RAISE => Instruction::RawRaise,
            OPCODE_BUILD_STACKTRACE => Instruction::BuildStacktrace,
            OPCODE_SEND => Instruction::Send,
            OPCODE_REMOVE_MESSAGE => Instruction::RemoveMessage,
            OPCODE_LOOP_REC => Instruction::LoopReceive {
                fail_label: read_operand_u32(data, &mut cursor, TAG_LABEL)?,
                destination: read_register(data, &mut cursor)?,
            },
            OPCODE_LOOP_REC_END => {
                Instruction::LoopReceiveEnd(read_operand_u32(data, &mut cursor, TAG_LABEL)?)
            }
            OPCODE_WAIT => Instruction::Wait(read_operand_u32(data, &mut cursor, TAG_LABEL)?),
            OPCODE_WAIT_TIMEOUT => Instruction::WaitTimeout {
                label: read_operand_u32(data, &mut cursor, TAG_LABEL)?,
                source: read_move_operand(data, &mut cursor, atoms, literals)?,
            },
            OPCODE_TIMEOUT => Instruction::Timeout,
            OPCODE_RECV_MARKER_RESERVE => {
                Instruction::ReceiveMarkerReserve(read_register(data, &mut cursor)?)
            }
            OPCODE_RECV_MARKER_BIND => Instruction::ReceiveMarkerBind {
                marker: read_register(data, &mut cursor)?,
                reference: read_register(data, &mut cursor)?,
            },
            OPCODE_RECV_MARKER_USE => {
                Instruction::ReceiveMarkerUse(read_register(data, &mut cursor)?)
            }
            OPCODE_RECV_MARKER_CLEAR => {
                Instruction::ReceiveMarkerClear(read_register(data, &mut cursor)?)
            }
            OPCODE_GET_TUPLE_ELEMENT => Instruction::GetTupleElement {
                tuple: read_register(data, &mut cursor)?,
                index: read_operand_u32(data, &mut cursor, TAG_UNSIGNED)?,
                destination: read_register(data, &mut cursor)?,
            },
            OPCODE_UPDATE_RECORD => {
                let _hint = read_operand(data, &mut cursor, TAG_ATOM)?;
                let _size = read_operand(data, &mut cursor, TAG_UNSIGNED)?;
                Instruction::UpdateRecord {
                    source: read_move_operand(data, &mut cursor, atoms, literals)?,
                    destination: read_register(data, &mut cursor)?,
                    updates: read_record_updates(data, &mut cursor, atoms, literals)?,
                }
            }
            OPCODE_GET_LIST => Instruction::GetList {
                source: read_move_operand(data, &mut cursor, atoms, literals)?,
                head: read_register(data, &mut cursor)?,
                tail: read_register(data, &mut cursor)?,
            },
            OPCODE_GET_HD => Instruction::GetHead {
                source: read_move_operand(data, &mut cursor, atoms, literals)?,
                destination: read_register(data, &mut cursor)?,
            },
            OPCODE_GET_TL => Instruction::GetTail {
                source: read_move_operand(data, &mut cursor, atoms, literals)?,
                destination: read_register(data, &mut cursor)?,
            },
            OPCODE_PUT_TUPLE2 => {
                let destination = read_register(data, &mut cursor)?;
                if read_integer(data, &mut cursor, TAG_LITERAL)? != LIST_SUBTAG {
                    return Err(error("unsupported BEAM tuple element list"));
                }
                let item_count = usize::try_from(read_operand(data, &mut cursor, TAG_UNSIGNED)?)
                    .map_err(|_| error("BEAM tuple has too many elements"))?;
                let mut elements = Vec::with_capacity(item_count);
                for _ in 0..item_count {
                    elements.push(read_move_operand(data, &mut cursor, atoms, literals)?);
                }
                Instruction::PutTuple {
                    elements,
                    destination,
                }
            }
            OPCODE_GET_MAP_ELEMENTS => Instruction::GetMapElements {
                fail_label: read_operand_u32(data, &mut cursor, TAG_LABEL)?,
                map: read_move_operand(data, &mut cursor, atoms, literals)?,
                pairs: sort_map_key_pairs(
                    read_operand_pairs(data, &mut cursor, atoms, literals)?,
                    atom_table,
                )?,
            },
            OPCODE_HAS_MAP_FIELDS => {
                let fail_label = read_operand_u32(data, &mut cursor, TAG_LABEL)?;
                let map = read_move_operand(data, &mut cursor, atoms, literals)?;
                let key_count = read_list_count(data, &mut cursor)?;
                let keys = (0..key_count)
                    .map(|_| read_move_operand(data, &mut cursor, atoms, literals))
                    .collect::<Result<_, _>>()?;
                Instruction::HasMapFields {
                    fail_label,
                    map,
                    keys,
                }
            }
            OPCODE_PUT_MAP_ASSOC => {
                let _fail_label = read_operand(data, &mut cursor, TAG_LABEL)?;
                let map = read_move_operand(data, &mut cursor, atoms, literals)?;
                let destination = read_register(data, &mut cursor)?;
                skip_live_registers(data, &mut cursor)?;
                Instruction::PutMapAssoc {
                    map,
                    destination,
                    pairs: read_operand_pairs(data, &mut cursor, atoms, literals)?,
                }
            }
            OPCODE_PUT_MAP_EXACT => {
                let fail_label = read_operand_u32(data, &mut cursor, TAG_LABEL)?;
                let map = read_move_operand(data, &mut cursor, atoms, literals)?;
                let destination = read_register(data, &mut cursor)?;
                skip_live_registers(data, &mut cursor)?;
                Instruction::PutMapExact {
                    fail_label,
                    map,
                    destination,
                    pairs: read_operand_pairs(data, &mut cursor, atoms, literals)?,
                }
            }
            OPCODE_MAKE_FUN3 => {
                let number = read_operand(data, &mut cursor, TAG_UNSIGNED)?;
                let destination = read_register(data, &mut cursor)?;
                let free_count = read_list_count(data, &mut cursor)?;
                // ERTS `gen.make_fun3` gives `i_lambda_error` for these
                // cases (`generators.tab:128-151`), which stops the pinned
                // JIT emulator when it runs (`jit/arm/instr_fun.cpp:472-475`).
                // The compiler never emits them, so the load fails.
                let (lambda, _) = usize::try_from(number)
                    .ok()
                    .and_then(|number| lambdas.get(number))
                    .filter(|(_, count)| *count == free_count)
                    .ok_or_else(|| {
                        error(format!(
                            "BEAM make_fun3 lambda {number} with {free_count} free \
                             variables does not match the lambda table"
                        ))
                    })?;
                let free = (0..free_count)
                    .map(|_| read_move_operand(data, &mut cursor, atoms, literals))
                    .collect::<Result<_, _>>()?;
                Instruction::MakeFun {
                    lambda: Arc::clone(lambda),
                    free,
                    destination,
                }
            }
            OPCODE_CALL_FUN => {
                let arity = read_operand_u32(data, &mut cursor, TAG_UNSIGNED)?;
                Instruction::CallFun {
                    arity,
                    function: Operand::X(arity),
                    tail: false,
                    deallocate: 0,
                }
            }
            OPCODE_CALL_FUN2 => {
                // The tag is an atom or a lambda number (`aF` in
                // `jit/arm/ops.tab:879`). Atom index 0 is `[]`, not an atom.
                let tag_kind = match data.get(cursor).map(|byte| byte & COMPACT_TAG_MASK) {
                    Some(TAG_ATOM) => TAG_ATOM,
                    _ => TAG_UNSIGNED,
                };
                let tag = read_operand(data, &mut cursor, tag_kind)?;
                if tag_kind == TAG_ATOM {
                    atom_at(atoms, usize::try_from(tag).expect("a u64 fits in usize"))?;
                }
                let arity = read_operand_u32(data, &mut cursor, TAG_UNSIGNED)?;
                let function = read_move_operand(data, &mut cursor, atoms, literals)?;
                if !matches!(function, Operand::X(_) | Operand::Y(_)) {
                    return Err(error("BEAM call_fun2 function is not a register"));
                }
                Instruction::CallFun {
                    arity,
                    function,
                    tail: false,
                    deallocate: 0,
                }
            }
            OPCODE_SWAP => Instruction::Swap(
                read_register(data, &mut cursor)?,
                read_register(data, &mut cursor)?,
            ),
            OPCODE_TEST_IS_ATOM
            | OPCODE_TEST_IS_PID
            | OPCODE_TEST_IS_PORT
            | OPCODE_TEST_IS_REFERENCE
            | OPCODE_TEST_IS_TUPLE
            | OPCODE_TEST_IS_LIST
            | OPCODE_TEST_IS_NIL
            | OPCODE_TEST_IS_NONEMPTY_LIST
            | OPCODE_TEST_IS_BINARY
            | OPCODE_TEST_IS_BITSTR
            | OPCODE_TEST_IS_MAP
            | OPCODE_TEST_IS_FUNCTION
            | OPCODE_TEST_IS_INTEGER
            | OPCODE_TEST_IS_FLOAT
            | OPCODE_TEST_IS_NUMBER
            | OPCODE_TEST_IS_BOOLEAN => {
                let fail_label = read_operand_u32(data, &mut cursor, TAG_LABEL)?;
                let value = read_move_operand(data, &mut cursor, atoms, literals)?;
                let kind = test_kind(opcode)?;
                Instruction::Test {
                    kind,
                    fail_label,
                    value,
                }
            }
            OPCODE_IS_ANY_NATIVE_RECORD => {
                let fail_label = read_operand_u32(data, &mut cursor, TAG_LABEL)?;
                let value = read_move_operand(data, &mut cursor, atoms, literals)?;
                // ERTS loads the source only from an x or a y register (emu
                // `ops.tab:1631`; JIT `jit/arm/ops.tab:1400`).
                if !matches!(value, Operand::X(_) | Operand::Y(_)) {
                    return Err(error("BEAM is_any_native_record source is not a register"));
                }
                Instruction::Test {
                    kind: TestKind::AnyNativeRecord,
                    fail_label,
                    value,
                }
            }
            OPCODE_IS_NATIVE_RECORD => {
                let fail_label = read_operand_u32(data, &mut cursor, TAG_LABEL)?;
                let value = read_record_source(data, &mut cursor, atoms, literals)?;
                Instruction::IsNativeRecord {
                    fail_label,
                    value,
                    module: read_atom(data, &mut cursor, atoms)?,
                    name: read_atom(data, &mut cursor, atoms)?,
                }
            }
            OPCODE_IS_RECORD_ACCESSIBLE => Instruction::IsRecordAccessible {
                fail_label: read_operand_u32(data, &mut cursor, TAG_LABEL)?,
                value: read_record_source(data, &mut cursor, atoms, literals)?,
                external: read_atom(data, &mut cursor, atoms)?.is_one_of(&["external"]),
            },
            OPCODE_GET_RECORD_ELEMENTS => Instruction::GetRecordElements {
                fail_label: read_operand_u32(data, &mut cursor, TAG_LABEL)?,
                value: read_record_source(data, &mut cursor, atoms, literals)?,
                pairs: read_record_pairs(data, &mut cursor, atoms, literals)?,
            },
            OPCODE_PUT_RECORD => {
                // The label is reserved and must be 0 (genop.tab:736-740).
                (read_operand(data, &mut cursor, TAG_LABEL)? == 0)
                    .then_some(())
                    .ok_or_else(|| error("BEAM put_record has a fail label"))?;
                let id = read_move_operand(data, &mut cursor, atoms, literals)?;
                let source = read_move_operand(data, &mut cursor, atoms, literals)?;
                let destination = read_register(data, &mut cursor)?;
                skip_live_registers(data, &mut cursor)?;
                let updates = read_record_pairs(data, &mut cursor, atoms, literals)?;
                let kind = match (source, &id) {
                    (Operand::Value(Term::Nil), Operand::Value(Term::Atom(name))) => {
                        // The loader finds the definition in the `Recs`
                        // chunk of the module (`generators.tab:255-300`).
                        let def =
                            records
                                .iter()
                                .find(|def| def.name == *name)
                                .ok_or_else(|| {
                                    error("BEAM put_record names no record of the module")
                                })?;
                        PutRecord::Local(Arc::clone(def))
                    }
                    (Operand::Value(Term::Nil), id) => {
                        let (module, name) = record_id_pair(id)?;
                        PutRecord::Remote { module, name }
                    }
                    (source @ (Operand::X(_) | Operand::Y(_)), _) => PutRecord::Update(source),
                    _ => return Err(error("unsupported BEAM put_record source")),
                };
                Instruction::PutRecord {
                    kind,
                    destination,
                    updates,
                }
            }
            OPCODE_GET_RECORD_FIELD => {
                let fail_label = read_operand_u32(data, &mut cursor, TAG_LABEL)?;
                let value = read_move_operand(data, &mut cursor, atoms, literals)?;
                let id = match read_move_operand(data, &mut cursor, atoms, literals)? {
                    Operand::Value(Term::Atom(ref name)) => RecordId::Local(name.clone()),
                    id => {
                        let (module, name) = record_id_pair(&id)?;
                        RecordId::Remote { module, name }
                    }
                };
                Instruction::GetRecordField {
                    fail_label,
                    value,
                    id,
                    field: read_atom(data, &mut cursor, atoms)?,
                    destination: read_register(data, &mut cursor)?,
                }
            }
            OPCODE_TEST_IS_FUNCTION2 => Instruction::TestFunctionArity {
                fail_label: read_operand_u32(data, &mut cursor, TAG_LABEL)?,
                value: read_move_operand(data, &mut cursor, atoms, literals)?,
                arity: read_move_operand(data, &mut cursor, atoms, literals)?,
            },
            OPCODE_NIF_START => {
                let (name, arity, entry) = function
                    .clone()
                    .ok_or_else(|| error("BEAM nif_start has no func_info before it"))?;
                instructions.insert(entry, Instruction::NifStart { name, arity });
                continue;
            }
            OPCODE_TEST_HEAP | OPCODE_INIT_YREGS => {
                skip_unsupported_instruction(opcode, data, &mut cursor)?;
                // ERTS fuses `bs_match` and the `test_heap` after it
                // (JIT jit/arm/ops.tab:962-963).
                if opcode == OPCODE_TEST_HEAP
                    && let Some(Instruction::BinaryMatch { test_heap, .. }) =
                        instructions.last_mut()
                {
                    *test_heap = true;
                }
                Instruction::Noop
            }
            OPCODE_PUT_LIST => Instruction::PutList {
                head: read_move_operand(data, &mut cursor, atoms, literals)?,
                tail: read_move_operand(data, &mut cursor, atoms, literals)?,
                destination: read_register(data, &mut cursor)?,
            },
            // The entry starts at the next instruction. `nif_start` directly
            // follows the entry label, so its insertion moves no entry.
            OPCODE_LINE => {
                let item = read_operand(data, &mut cursor, TAG_UNSIGNED)?;
                function_line = data.get(cursor) == Some(&OPCODE_FUNCTION_INFO);
                if function_line {
                    lines.function(instructions.len(), Some(item))?;
                } else {
                    lines.line(instructions.len(), item)?;
                }
                continue;
            }
            OPCODE_CALL_EXT | OPCODE_CALL_EXT_LAST | OPCODE_CALL_EXT_ONLY => {
                let arity = read_operand_u32(data, &mut cursor, TAG_UNSIGNED)?;
                let index = read_operand_usize(data, &mut cursor, TAG_UNSIGNED)?;
                let (module, name, imported_arity) = imports
                    .get(index)
                    .ok_or_else(|| error(format!("BEAM import index {index} out of range")))?;
                if arity != *imported_arity {
                    return Err(error(format!(
                        "BEAM external call arity {arity} does not match import arity {imported_arity}"
                    )));
                }
                let deallocate = if opcode == OPCODE_CALL_EXT_LAST {
                    read_operand_u32(data, &mut cursor, TAG_UNSIGNED)?
                } else {
                    0
                };
                // `i_hibernate` and `i_yield` return to the next
                // instruction, so a tail call hibernates or yields first and
                // returns after that (`jit/arm/ops.tab:664-673`;
                // `emu/ops.tab:896-898`). A tail call of a BIF returns to
                // the `deallocate` and the `return` after it
                // (`jit/arm/ops.tab:687-714`).
                instructions.push(match (module.as_str(), name.as_str(), arity) {
                    ("erlang", "hibernate", 0) => Instruction::Hibernate,
                    ("erlang", "yield", 0) => Instruction::Yield,
                    // An import index of the `ImpT` chunk is below 2^32,
                    // so the conversion keeps every value.
                    _ => Instruction::CallExternal {
                        import: u32::try_from(index).expect("an import index is below 2^32"),
                        tail: opcode != OPCODE_CALL_EXT,
                        deallocate,
                    },
                });
                if opcode == OPCODE_CALL_EXT_LAST {
                    instructions.push(Instruction::Deallocate {
                        stack_size: deallocate,
                    });
                }
                if opcode != OPCODE_CALL_EXT {
                    instructions.push(Instruction::Return);
                }
                continue;
            }
            OPCODE_APPLY | OPCODE_APPLY_LAST => {
                let arity = read_operand_u32(data, &mut cursor, TAG_UNSIGNED)?;
                let deallocate = if opcode == OPCODE_APPLY_LAST {
                    read_operand_u32(data, &mut cursor, TAG_UNSIGNED)?
                } else {
                    0
                };
                Instruction::Apply {
                    arity,
                    tail: opcode == OPCODE_APPLY_LAST,
                    deallocate,
                }
            }
            OPCODE_BIF0 => {
                let bif = bif_import(
                    imports,
                    read_operand_usize(data, &mut cursor, TAG_UNSIGNED)?,
                    0,
                )?;
                Instruction::Bif {
                    fail_label: 0,
                    bif,
                    arguments: Vec::new(),
                    destination: read_register(data, &mut cursor)?,
                }
            }
            OPCODE_BIF1 | OPCODE_BIF2 | OPCODE_BIF3 | OPCODE_GC_BIF1 | OPCODE_GC_BIF2
            | OPCODE_GC_BIF3 => {
                let fail_label = read_operand_u32(data, &mut cursor, TAG_LABEL)?;
                if matches!(opcode, OPCODE_GC_BIF1 | OPCODE_GC_BIF2 | OPCODE_GC_BIF3) {
                    skip_live_registers(data, &mut cursor)?;
                }
                let argument_count = match opcode {
                    OPCODE_BIF1 | OPCODE_GC_BIF1 => 1,
                    OPCODE_BIF3 | OPCODE_GC_BIF3 => 3,
                    _ => 2,
                };
                let bif = bif_import(
                    imports,
                    read_operand_usize(data, &mut cursor, TAG_UNSIGNED)?,
                    argument_count,
                )?;
                let arguments = (0..argument_count)
                    .map(|_| read_move_operand(data, &mut cursor, atoms, literals))
                    .collect::<Result<Vec<_>, _>>()?;
                Instruction::Bif {
                    fail_label,
                    bif,
                    arguments,
                    destination: read_register(data, &mut cursor)?,
                }
            }
            OPCODE_ALLOCATE | OPCODE_ALLOCATE_HEAP => {
                let stack_size = read_operand_u32(data, &mut cursor, TAG_UNSIGNED)?;
                if opcode == OPCODE_ALLOCATE_HEAP {
                    skip_operand(data, &mut cursor)?;
                }
                skip_live_registers(data, &mut cursor)?;
                Instruction::Allocate { stack_size }
            }
            OPCODE_DEALLOCATE => Instruction::Deallocate {
                stack_size: read_operand_u32(data, &mut cursor, TAG_UNSIGNED)?,
            },
            OPCODE_TRIM => Instruction::Trim {
                count: read_operand_u32(data, &mut cursor, TAG_UNSIGNED)?,
                remaining: read_operand_u32(data, &mut cursor, TAG_UNSIGNED)?,
            },
            OPCODE_MOVE => {
                let source = read_move_operand(data, &mut cursor, atoms, literals)?;
                let destination = read_register(data, &mut cursor)?;
                Instruction::Move(source, destination)
            }
            OPCODE_INT_CODE_END => {
                if cursor != data.len() || function_info_count != declared_function_count {
                    return Err(error("invalid BEAM code terminator or function count"));
                }
                instructions.push(Instruction::End);
                return Ok((instructions, number_of_labels, on_load));
            }
            _ => return Err(error(format!("unsupported BEAM opcode {opcode}"))),
        };
        instructions.push(instruction);
    }
}

/// Return the identity of the BIF that import `index` of a `bif`
/// instruction calls at `arity`.
///
/// The arity is the one of the opcode, which is the one of the call. The
/// import index of a malformed module and the MFA of an import that no
/// `bif.tab` entry names are loader errors.
///
/// # Examples
///
/// ```rust,ignore
/// let imports = vec![("erlang".to_owned(), "node".to_owned(), 0)];
/// assert_eq!(bif_import(&imports, 0, 0)?, BifId::ErlangNode0);
/// assert!(bif_import(&imports, 1, 0).is_err());
/// ```
fn bif_import(
    imports: &[(String, String, u32)],
    index: usize,
    arity: u32,
) -> Result<BifId, BeamError> {
    let (module, name, _arity) = imports
        .get(index)
        .ok_or_else(|| error(format!("BEAM BIF import index {index} out of range")))?;
    resolve_bif(module, name, arity)
        .ok_or_else(|| error(format!("unsupported BIF {module}:{name}/{arity}")))
}

/// Return the type checked by a BEAM type-test opcode.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(test_kind(OPCODE_TEST_IS_BINARY)?, TestKind::Binary);
/// ```
fn test_kind(opcode: u8) -> Result<TestKind, BeamError> {
    match opcode {
        OPCODE_TEST_IS_ATOM => Ok(TestKind::Atom),
        OPCODE_TEST_IS_PID => Ok(TestKind::Pid),
        OPCODE_TEST_IS_PORT => Ok(TestKind::Port),
        OPCODE_TEST_IS_REFERENCE => Ok(TestKind::Reference),
        OPCODE_TEST_IS_TUPLE => Ok(TestKind::Tuple),
        OPCODE_TEST_IS_LIST => Ok(TestKind::List),
        OPCODE_TEST_IS_NIL => Ok(TestKind::Nil),
        OPCODE_TEST_IS_NONEMPTY_LIST => Ok(TestKind::NonEmptyList),
        OPCODE_TEST_IS_BINARY => Ok(TestKind::Binary),
        OPCODE_TEST_IS_BITSTR => Ok(TestKind::Bitstring),
        OPCODE_TEST_IS_MAP => Ok(TestKind::Map),
        OPCODE_TEST_IS_FUNCTION => Ok(TestKind::Function),
        OPCODE_TEST_IS_INTEGER => Ok(TestKind::Integer),
        OPCODE_TEST_IS_FLOAT => Ok(TestKind::Float),
        OPCODE_TEST_IS_NUMBER => Ok(TestKind::Number),
        OPCODE_TEST_IS_BOOLEAN => Ok(TestKind::Boolean),
        _ => Err(error(format!("unsupported BEAM type-test opcode {opcode}"))),
    }
}

/// Read an extended operand list of pairs, such as the key and value list
/// of `put_map_assoc`.
///
/// # Examples
///
/// ```rust,ignore
/// // {list,[{atom,1},{x,1}]}
/// let pairs = read_operand_pairs(&[0x17, 0x20, 0x12, 0x13], &mut 0, &atoms, &[])?;
/// assert_eq!(pairs.len(), 1);
/// ```
fn read_operand_pairs(
    data: &[u8],
    cursor: &mut usize,
    atoms: &[Atom],
    literals: &[Option<Term>],
) -> Result<Vec<(Operand, Operand)>, BeamError> {
    if read_integer(data, cursor, TAG_LITERAL)? != LIST_SUBTAG {
        return Err(error("unsupported BEAM operand list"));
    }
    let item_count = usize::try_from(read_operand(data, cursor, TAG_UNSIGNED)?)
        .map_err(|_| error("BEAM operand list is too large"))?;
    if item_count % 2 != 0 {
        return Err(error("BEAM operand list has an incomplete pair"));
    }
    let mut pairs = Vec::with_capacity(item_count / 2);
    for _ in 0..item_count / 2 {
        let first = read_move_operand(data, cursor, atoms, literals)?;
        let second = read_move_operand(data, cursor, atoms, literals)?;
        pairs.push((first, second));
    }
    Ok(pairs)
}

/// Sort the key and destination pairs of `get_map_elements` in flatmap key
/// order, as the ERTS loader does (`beam_load_map_key_sort`,
/// `beam_transform_helpers.c:53-115`, with `erts_cmp_flatmap_keys`; JIT
/// `jit/arm/ops.tab:1190-1197`). The run loop then finds all keys in one
/// walk of a flatmap. One pair stays as it is. With more pairs, each key
/// must be a literal: ERTS loads no `get_map_elements` with a register key
/// among other keys. A literal key that PBeam does not support yet sorts
/// first, so that the run loop reports it at its first comparison.
///
/// # Examples
///
/// ```rust,ignore
/// let pairs = vec![(Operand::Value(Term::Integer(2)), Operand::X(0)), (Operand::Value(Term::Integer(1)), Operand::X(1))];
/// let sorted = sort_map_key_pairs(pairs, &AtomTable::new())?;
/// assert!(matches!(sorted[0], (Operand::Value(Term::Integer(1)), Operand::X(1))));
/// ```
///
/// # Errors
///
/// Return an error for a register key among other keys.
fn sort_map_key_pairs(
    mut pairs: Vec<(Operand, Operand)>,
    atoms: &AtomTable,
) -> Result<Vec<(Operand, Operand)>, BeamError> {
    if pairs.len() < 2 {
        return Ok(pairs);
    }
    if pairs
        .iter()
        .any(|(key, _)| matches!(key, Operand::X(_) | Operand::Y(_)))
    {
        return Err(error(
            "unsupported BEAM get_map_elements: a register key among other keys",
        ));
    }
    pairs.sort_by(|(left, _), (right, _)| match (left, right) {
        (Operand::Value(left), Operand::Value(right)) => flatmap_order(left, right, atoms),
        _ => matches!(left, Operand::Value(_)).cmp(&matches!(right, Operand::Value(_))),
    });
    Ok(pairs)
}

/// Read the command list of `bs_match`: each command is a name atom and
/// its operands (genop.tab:685-696). The command names and operand counts
/// are those of the ERTS loader (`gen.bs_match` in JIT
/// `jit/arm/generators.tab:532-575` and `beam_jit_bsm_init` in
/// `jit/beam_jit_bs.cpp:287-364`), which fails to load a module with
/// another command name. `Live` is unsigned, except for `=:=`, where the
/// compiler writes `nil` (beam_ssa_codegen.erl:2817-2862). `Size` and
/// `Unit` are unsigned. `Flags` is `nil` or a literal list, and `Dst` is a
/// register.
///
/// # Examples
///
/// ```rust,ignore
/// // {list,[ensure_at_least,0,8]} with atom 8 `ensure_at_least`
/// let commands = read_binary_match_commands(&[0x17, 0x30, 0x82, 0x00, 0x80], &mut 0, &atom_table, &atoms, &[])?;
/// assert!(matches!(commands.as_slice(), [BinaryMatchCommand::EnsureAtLeast { size: 0, .. }]));
/// // {list,[skip,8]} with atom 17 `skip`
/// let commands = read_binary_match_commands(&[0x17, 0x20, 0x0A, 0x11, 0x80], &mut 0, &atom_table, &atoms, &[])?;
/// assert!(matches!(commands.as_slice(), [BinaryMatchCommand::Skip { size: 8 }]));
/// ```
///
/// # Errors
///
/// An unknown command name, `ensure_at_least` with unit 0, a `Size *
/// Unit` above `u64::MAX`, flags that are not a list, an operand with an
/// unexpected tag, and a list with an incomplete command are errors.
fn read_binary_match_commands(
    data: &[u8],
    cursor: &mut usize,
    atom_table: &AtomTable,
    atoms: &[Atom],
    literals: &[Option<Term>],
) -> Result<Vec<BinaryMatchCommand>, BeamError> {
    let item_count = read_list_count(data, cursor)?;
    let mut commands = Vec::new();
    let mut items = 0;
    while items < item_count {
        let name = atom_at(atoms, read_operand_usize(data, cursor, TAG_ATOM)?)?;
        let (command, operand_count) = if name.is_static("ensure_at_least") {
            let size = read_operand(data, cursor, TAG_UNSIGNED)?;
            let unit = NonZeroU64::new(read_operand(data, cursor, TAG_UNSIGNED)?)
                .ok_or_else(|| error("BEAM bs_match ensure_at_least has unit 0"))?;
            (BinaryMatchCommand::EnsureAtLeast { size, unit }, 2)
        } else if name.is_static("ensure_exactly") {
            let size = read_operand(data, cursor, TAG_UNSIGNED)?;
            (BinaryMatchCommand::EnsureExactly { size }, 1)
        } else if name.is_one_of(&["integer", "binary"]) {
            let kind = atom_table.name_of(name);
            let _live = read_operand(data, cursor, TAG_UNSIGNED)?;
            let flags = read_move_operand(data, cursor, atoms, literals)?;
            let size = read_operand(data, cursor, TAG_UNSIGNED)?
                .checked_mul(read_operand(data, cursor, TAG_UNSIGNED)?)
                .ok_or_else(|| error(format!("BEAM bs_match {kind} size overflow")))?;
            let destination = read_register(data, cursor)?;
            let Some(flags) = (match &flags {
                Operand::Value(flags) => flags.proper_list(),
                _ => None,
            }) else {
                return Err(error(format!(
                    "unsupported BEAM bs_match {kind} flags {flags:?}"
                )));
            };
            let has = |names: &[&str]| {
                flags
                    .clone()
                    .any(|flag| matches!(flag, Term::Atom(atom) if atom.is_one_of(names)))
            };
            let command = if name.is_static("integer") {
                BinaryMatchCommand::Integer {
                    size,
                    little: has(&["little", "native"]),
                    signed: has(&["signed"]),
                    destination,
                }
            } else {
                BinaryMatchCommand::Binary { size, destination }
            };
            (command, 5)
        } else if name.is_static("get_tail") {
            let _live = read_operand(data, cursor, TAG_UNSIGNED)?;
            let _unit = read_operand(data, cursor, TAG_UNSIGNED)?;
            let destination = read_register(data, cursor)?;
            (BinaryMatchCommand::GetTail { destination }, 3)
        } else if name.is_static("=:=") {
            let _live = read_move_operand(data, cursor, atoms, literals)?;
            let size = read_operand(data, cursor, TAG_UNSIGNED)?;
            let value = read_operand(data, cursor, TAG_UNSIGNED)?;
            (BinaryMatchCommand::Equal { size, value }, 3)
        } else if name.is_static("skip") {
            let size = read_operand(data, cursor, TAG_UNSIGNED)?;
            (BinaryMatchCommand::Skip { size }, 1)
        } else {
            return Err(error(format!(
                "unsupported BEAM bs_match command {}",
                atom_table.name_of(name)
            )));
        };
        commands.push(command);
        items += 1 + operand_count;
    }
    if items != item_count {
        return Err(error(
            "BEAM bs_match command list has an incomplete command",
        ));
    }
    Ok(commands)
}

/// Read the segment list of `bs_create_bin`: `Type Segment Unit Flags Src
/// Size` for each segment (genop.tab:649-651; JIT `gen.create_bin` in
/// jit/arm/generators.tab:441-465). `Type` is an atom, `Segment` and `Unit`
/// are unsigned, and `Flags` is `nil` or a literal list. The `Src` of a
/// `string` segment is an unsigned offset in the `StrT` chunk, and its
/// `Size` is the integer length in bytes (beam_asm.erl:768-770). Each
/// other `Src` and `Size` is a move operand.
///
/// # Examples
///
/// ```rust,ignore
/// // {list,[{atom,string},0,8,nil,{string,<<"count: ">>},{integer,7},
/// //        {atom,binary},2,8,nil,{x,0},{atom,all}]}
/// let bytes = [0x17, 0xC0, 0x32, 0x00, 0x80, 0x02, 0x00, 0x71, 0x42, 0x20, 0x80, 0x02, 0x03, 0x52];
/// let segments = read_binary_segments(&bytes, &mut 0, &atoms, &[], b"count: ")?;
/// assert!(matches!(segments.as_slice(), [BinarySegment::String(text), BinarySegment::BinaryAll { segment: 2, source: Operand::X(0) }] if text == b"count: "));
/// ```
///
/// # Errors
///
/// A list with an incomplete segment, an operand with an unexpected tag,
/// and a string outside the `StrT` chunk are errors.
fn read_binary_segments(
    data: &[u8],
    cursor: &mut usize,
    atoms: &[Atom],
    literals: &[Option<Term>],
    strings: &[u8],
) -> Result<Vec<BinarySegment>, BeamError> {
    let item_count = read_list_count(data, cursor)?;
    if item_count % BINARY_SEGMENT_ITEMS != 0 {
        return Err(error(
            "BEAM bs_create_bin segment list has an incomplete segment",
        ));
    }
    (0..item_count / BINARY_SEGMENT_ITEMS)
        .map(|index| {
            let kind = atom_at(atoms, read_operand_usize(data, cursor, TAG_ATOM)?)?.clone();
            let segment = read_operand(data, cursor, TAG_UNSIGNED)?;
            let unit = read_operand(data, cursor, TAG_UNSIGNED)?;
            let flags = read_move_operand(data, cursor, atoms, literals)?;
            if kind.is_static("string") {
                let offset = read_operand(data, cursor, TAG_UNSIGNED)?;
                let length = read_operand(data, cursor, TAG_INTEGER)?;
                return usize::try_from(offset)
                    .ok()
                    .zip(usize::try_from(length).ok())
                    .and_then(|(offset, length)| strings.get(offset..offset.checked_add(length)?))
                    .map(|text| BinarySegment::String(text.to_vec()))
                    .ok_or_else(|| error("BEAM bs_create_bin string is outside the StrT chunk"));
            }
            let source = read_move_operand(data, cursor, atoms, literals)?;
            let size = read_move_operand(data, cursor, atoms, literals)?;
            Ok(match &size {
                Operand::Value(Term::Atom(size))
                    if kind.is_static("binary") && size.is_static("all") =>
                {
                    BinarySegment::BinaryAll {
                        segment,
                        unit,
                        source,
                    }
                }
                Operand::Value(Term::Atom(size))
                    if kind.is_static("append") && index == 0 && size.is_static("all") =>
                {
                    BinarySegment::Append {
                        segment,
                        unit,
                        source,
                    }
                }
                _ if kind.is_static("binary") => BinarySegment::Binary {
                    segment,
                    unit,
                    source,
                    size,
                },
                _ if kind.is_static("integer") => BinarySegment::Integer {
                    segment,
                    unit,
                    little: is_little_endian(&flags),
                    source,
                    size,
                },
                Operand::Value(Term::Atom(size))
                    if kind.is_static("private_append") && index == 0 && size.is_static("all") =>
                {
                    BinarySegment::PrivateAppend { source }
                }
                _ if kind.is_static("float") => BinarySegment::Float {
                    segment,
                    unit,
                    little: is_little_endian(&flags),
                    source,
                    size,
                },
                _ if kind.is_static("utf8") => BinarySegment::Utf8 { segment, source },
                _ if kind.is_static("utf16") => BinarySegment::Utf16 {
                    segment,
                    little: is_little_endian(&flags),
                    source,
                },
                _ if kind.is_static("utf32") => BinarySegment::Utf32 {
                    segment,
                    little: is_little_endian(&flags),
                    source,
                },
                _ => BinarySegment::Unsupported(kind),
            })
        })
        .collect()
}

/// Return true when the `Flags` operand of a `bs_create_bin` segment is a
/// list that holds the atom `little` or `native`. The ERTS JIT reads both
/// as `BSF_LITTLE` and ignores `signed` for construction (`bs_get_flags`,
/// jit/beam_jit_bs.cpp:255-285).
///
/// # Examples
///
/// ```rust,ignore
/// assert!(is_little_endian(&Operand::Value(Term::list_from(vec![Term::static_atom("native")]))));
/// assert!(!is_little_endian(&Operand::Value(Term::nil())));
/// ```
fn is_little_endian(flags: &Operand) -> bool {
    matches!(flags, Operand::Value(flags) if flags.proper_list().is_some_and(|mut items| items.any(|item| {
        matches!(item, Term::Atom(atom) if atom.is_one_of(&["little", "native"]))
    })))
}

/// Read the source of a record test or of `get_record_elements`, which ERTS
/// loads only from an x or a y register (`ops.tab:1631-1667`).
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(read_record_source(&[0x03], &mut 0, &[], &[])?, Operand::X(0)));
/// ```
fn read_record_source(
    data: &[u8],
    cursor: &mut usize,
    atoms: &[Atom],
    literals: &[Option<Term>],
) -> Result<Operand, BeamError> {
    match read_move_operand(data, cursor, atoms, literals)? {
        source @ (Operand::X(_) | Operand::Y(_)) => Ok(source),
        _ => Err(error("BEAM native record source is not a register")),
    }
}

/// Read an atom operand.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(read_atom(&[0x12], &mut 0, &atoms)?, atoms[0]);
/// ```
fn read_atom(data: &[u8], cursor: &mut usize, atoms: &[Atom]) -> Result<Atom, BeamError> {
    match &read_value(data, cursor, atoms)? {
        Term::Atom(atom) => Ok(atom.clone()),
        _ => Err(error("BEAM record operand is not an atom")),
    }
}

/// Read the `{Module, Name}` literal of a remote record.
///
/// # Examples
///
/// ```rust,ignore
/// let id = Operand::Value(Term::tuple(vec![Term::Atom(m), Term::Atom(r)]));
/// assert_eq!(record_id_pair(&id)?, (m, r));
/// ```
fn record_id_pair(id: &Operand) -> Result<(Atom, Atom), BeamError> {
    match id {
        Operand::Value(Term::Tuple(pair)) => match &pair[..] {
            [Term::Atom(module), Term::Atom(name)] => Ok((module.clone(), name.clone())),
            _ => Err(error("unsupported BEAM record identifier")),
        },
        _ => Err(error("unsupported BEAM record identifier")),
    }
}

/// Read the field and operand pairs of a record instruction and sort them in
/// flatmap key order, as `map_key_sort` does (`ops.tab:1638-1651`).
///
/// # Examples
///
/// ```rust,ignore
/// // {list,[{atom,y},{x,1},{atom,x},{x,2}]}
/// let pairs = read_record_pairs(&[0x17, 0x40, 0x22, 0x13, 0x12, 0x23], &mut 0, &atoms, &[])?;
/// assert_eq!(pairs[0].0, atoms[0]);
/// ```
fn read_record_pairs(
    data: &[u8],
    cursor: &mut usize,
    atoms: &[Atom],
    literals: &[Option<Term>],
) -> Result<Vec<(Atom, Operand)>, BeamError> {
    let mut pairs = read_operand_pairs(data, cursor, atoms, literals)?
        .into_iter()
        .map(|(field, operand)| match &field {
            Operand::Value(Term::Atom(field)) => Ok((field.clone(), operand)),
            _ => Err(error("BEAM record field is not an atom")),
        })
        .collect::<Result<Vec<_>, _>>()?;
    pairs.sort_by_key(|(field, _)| field.index());
    Ok(pairs)
}

/// Read the extended list of `update_record`: an unsigned 1-based position
/// and a value operand for each update.
///
/// # Examples
///
/// ```rust,ignore
/// // {list,[5,{tr,{y,2},3}]} from `gen_statem:enter/8`
/// let updates = read_record_updates(&[0x17, 0x20, 0x50, 0x57, 0x24, 0x30], &mut 0, &[], &[])?;
/// assert!(matches!(updates.as_slice(), [(5, Operand::Y(2))]));
/// ```
fn read_record_updates(
    data: &[u8],
    cursor: &mut usize,
    atoms: &[Atom],
    literals: &[Option<Term>],
) -> Result<Vec<(u32, Operand)>, BeamError> {
    let pair_count = read_pair_count(data, cursor)?;
    let mut updates = Vec::with_capacity(pair_count);
    for _ in 0..pair_count {
        let position = read_operand_u32(data, cursor, TAG_UNSIGNED)?;
        updates.push((position, read_move_operand(data, cursor, atoms, literals)?));
    }
    Ok(updates)
}

/// Skip operands for a known instruction that the runtime cannot execute.
///
/// # Examples
///
/// ```rust,ignore
/// let mut cursor = 0;
/// skip_unsupported_instruction(OPCODE_TEST_HEAP, &[0, 0], &mut cursor)?;
/// assert_eq!(cursor, 2);
/// ```
fn skip_unsupported_instruction(
    opcode: u8,
    data: &[u8],
    cursor: &mut usize,
) -> Result<(), BeamError> {
    let operand_count = unsupported_opcode_operand_count(opcode)
        .ok_or_else(|| error(format!("unsupported BEAM opcode {opcode}")))?;
    skip_operands(data, cursor, operand_count)?;
    if matches!(
        opcode,
        OPCODE_PUT_MAP_ASSOC | OPCODE_PUT_TUPLE2 | OPCODE_INIT_YREGS
    ) {
        skip_counted_operands(data, cursor)?;
    }
    Ok(())
}

/// Return the fixed operand count for a known unsupported opcode.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(unsupported_opcode_operand_count(OPCODE_TEST_HEAP), Some(2));
/// assert_eq!(unsupported_opcode_operand_count(OPCODE_BS_SKIP_UTF8), None);
/// assert_eq!(unsupported_opcode_operand_count(255), None);
/// ```
fn unsupported_opcode_operand_count(opcode: u8) -> Option<usize> {
    match opcode {
        OPCODE_TEST_HEAP
        | OPCODE_BIF0
        | OPCODE_TEST_IS_ATOM
        | OPCODE_TEST_IS_PID
        | OPCODE_TEST_IS_BINARY
        | OPCODE_TEST_IS_LIST
        | OPCODE_TEST_IS_MAP
        | OPCODE_PUT_TUPLE2 => Some(2),
        OPCODE_TEST_EXACT_EQ => Some(3),
        OPCODE_TEST_TAGGED_TUPLE => Some(4),
        OPCODE_PUT_MAP_ASSOC => Some(5),
        OPCODE_INIT_YREGS => Some(1),
        _ => None,
    }
}

/// Read a compact operand count, then skip that many operands.
///
/// # Examples
///
/// ```rust,ignore
/// let mut cursor = 0;
/// skip_counted_operands(&[0x10, 0x20], &mut cursor)?;
/// assert_eq!(cursor, 2);
/// ```
fn skip_counted_operands(data: &[u8], cursor: &mut usize) -> Result<(), BeamError> {
    let operand_count = usize::try_from(read_operand(data, cursor, TAG_UNSIGNED)?)
        .map_err(|_| error("BEAM instruction operand list is too large"))?;
    if operand_count > data.len().saturating_sub(*cursor) {
        return Err(error("BEAM instruction operand list exceeds code chunk"));
    }
    skip_operands(data, cursor, operand_count)
}

/// Skip the `Live` operand of an instruction that can collect garbage.
///
/// ERTS gives `Live` the operand type `t` (`ops.tab`), and the loader matches
/// `t` only to the `u` tag (`beam_makeops`, `define_type_bit('t', ...)`). The
/// ERTS garbage collector keeps X registers `0..Live` alive. PBeam has no
/// process heap to collect, so this function does not keep the count. It
/// validates the `u` tag and moves the cursor past the operand.
///
/// # Examples
///
/// ```rust,ignore
/// let mut cursor = 0;
/// skip_live_registers(&[0x20], &mut cursor)?;
/// assert_eq!(cursor, 1);
/// assert!(skip_live_registers(&[0x03], &mut 0).is_err());
/// ```
fn skip_live_registers(data: &[u8], cursor: &mut usize) -> Result<(), BeamError> {
    read_operand(data, cursor, TAG_UNSIGNED).map(|_live_registers| ())
}

/// Read the unsigned `Flags` operand of a segment and return `true` for a
/// little-endian segment. The `native` flag (16) gives the byte order of
/// the host, as `$NativeEndian` does in the ERTS loader (generators.tab);
/// otherwise the `little` flag (2) decides (`BSF_LITTLE` and `BSF_NATIVE`
/// in erl_bits.h:462-465).
///
/// # Examples
///
/// ```rust,ignore
/// assert!(read_little_flag(&[0x20], &mut 0)?);
/// assert!(!read_little_flag(&[0x00], &mut 0)?);
/// assert_eq!(read_little_flag(&[0x08, 0x10], &mut 0)?, cfg!(target_endian = "little"));
/// ```
fn read_little_flag(data: &[u8], cursor: &mut usize) -> Result<bool, BeamError> {
    read_operand(data, cursor, TAG_UNSIGNED).map(read_flag_byte_order)
}

/// Read the unsigned `Flags` operand of a segment and return its byte order
/// and its sign, as the ERTS loader does. The `native` flag (16) gives the
/// byte order of the host, as `$NativeEndian` does in the ERTS loader
/// (generators.tab); otherwise the `little` flag (2) decides, and the
/// `signed` flag (4) gives a signed segment (`BSF_LITTLE`, `BSF_SIGNED`,
/// and `BSF_NATIVE` in erl_bits.h:461-465).
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(read_segment_flags(&[0x45], &mut 0)?, (false, true));
/// assert_eq!(read_segment_flags(&[0x00], &mut 0)?, (false, false));
/// assert_eq!(
///     read_segment_flags(&[0x08, 0x10], &mut 0)?,
///     (cfg!(target_endian = "little"), false)
/// );
/// ```
fn read_segment_flags(data: &[u8], cursor: &mut usize) -> Result<(bool, bool), BeamError> {
    let flags = read_operand(data, cursor, TAG_UNSIGNED)?;
    Ok((read_flag_byte_order(flags), flags & BSF_SIGNED != 0))
}

/// Return the byte order of the segment flags `flags`: `true` for a
/// little-endian segment.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(read_flag_byte_order(2));
/// assert_eq!(read_flag_byte_order(16), cfg!(target_endian = "little"));
/// assert!(!read_flag_byte_order(0));
/// ```
fn read_flag_byte_order(flags: u64) -> bool {
    if flags & BSF_NATIVE != 0 {
        cfg!(target_endian = "little")
    } else {
        flags & BSF_LITTLE != 0
    }
}

/// Read the string of a `bs_match_string` instruction at the cursor: the
/// unsigned byte offset of the string in the `StrT` chunk. The string has
/// `(Bits + 7) / 8` bytes (`beam_disasm.erl:1189-1197`).
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(read_segment_string(&[0x00], &mut 0, b"count: ", 16)?, b"co".to_vec());
/// assert!(read_segment_string(&[0x00], &mut 0, b"count: ", 64).is_err());
/// ```
///
/// # Errors
///
/// A string that ends after the `StrT` chunk is an error.
fn read_segment_string(
    data: &[u8],
    cursor: &mut usize,
    strings: &[u8],
    bits: u64,
) -> Result<Vec<u8>, BeamError> {
    let offset = read_operand(data, cursor, TAG_UNSIGNED)?;
    let length = bits.div_ceil(8);
    usize::try_from(offset)
        .ok()
        .zip(usize::try_from(length).ok())
        .and_then(|(offset, length)| strings.get(offset..offset.checked_add(length)?))
        .map(<[u8]>::to_vec)
        .ok_or_else(|| error("BEAM bs_match_string is outside the StrT chunk"))
}

/// Skip the `Alloc` operand of `bs_create_bin`: a `u` heap word count or an
/// allocation list (extended subtag 3).
///
/// ERTS gives `Alloc` the operand type `u` (`jit/arm/ops.tab:1104`). Its
/// loader changes an allocation list into a `u` operand with the sum of the
/// heap words (`beam_file.c:2384-2395`). `marshal_allocation_list`
/// (`beam_file.c:2234-2280`) rejects a list of more than three items, an
/// item that is not two `u` operands, a number larger than `i32::MAX / 3`,
/// a kind other than words (0), floats (1), and funs (2), and a sum larger
/// than `i32::MAX`. A float uses `FLOAT_SIZE_OBJECT` words and a fun uses
/// `ERL_FUN_SIZE` words, both 2 on a 64-bit ERTS (`erl_term.h:526`,
/// `erl_fun.h:99`). PBeam has no process heap, so this function does not
/// keep the sum. It applies the same checks and moves the cursor past the
/// operand. An operand of another tag is a load error, as in ERTS.
///
/// # Examples
///
/// ```rust,ignore
/// // {alloc,[{words,1},{floats,0},{funs,1}]}
/// let mut cursor = 0;
/// skip_allocation(&[0x37, 0x30, 0x00, 0x10, 0x10, 0x00, 0x20, 0x10], &mut cursor)?;
/// assert_eq!(cursor, 8);
/// skip_allocation(&[0x70], &mut 0)?; // 7
/// assert!(skip_allocation(&[0x37, 0x10, 0x30, 0x10], &mut 0).is_err()); // kind 3
/// ```
fn skip_allocation(data: &[u8], cursor: &mut usize) -> Result<(), BeamError> {
    const ALLOCATION_LIST_SUBTAG: i128 = 3;
    const MAX_ALLOCATION_ITEMS: u64 = 3;
    /// `i32::MAX`.
    const MAX_HEAP_WORDS: u64 = (1 << 31) - 1;
    /// `FLOAT_SIZE_OBJECT` on a 64-bit ERTS: a header and one data word.
    const FLOAT_WORDS: u64 = 2;
    /// `ERL_FUN_SIZE` on a 64-bit ERTS: a header and one entry pointer.
    const FUN_WORDS: u64 = 2;
    /// `MAX(FLOAT_SIZE_OBJECT, ERL_FUN_SIZE + 1)`, which is 3, divides the limit.
    const MAX_ITEM_NUMBER: u64 = MAX_HEAP_WORDS / (FUN_WORDS + 1);
    let first_byte = *data
        .get(*cursor)
        .ok_or_else(|| error("truncated BEAM operand"))?;
    if first_byte & COMPACT_TAG_MASK != TAG_LITERAL {
        return read_operand(data, cursor, TAG_UNSIGNED).map(|_heap_words| ());
    }
    if read_integer(data, cursor, TAG_LITERAL)? != ALLOCATION_LIST_SUBTAG {
        return Err(error("unsupported BEAM allocation operand"));
    }
    let item_count = read_operand(data, cursor, TAG_UNSIGNED)?;
    if item_count > MAX_ALLOCATION_ITEMS {
        return Err(error("BEAM allocation list has more than three items"));
    }
    let mut heap_words = 0;
    for _ in 0..item_count {
        let kind = read_operand(data, cursor, TAG_UNSIGNED)?;
        let number = read_operand(data, cursor, TAG_UNSIGNED)?;
        if number > MAX_ITEM_NUMBER {
            return Err(error("BEAM allocation number is too large"));
        }
        let words_per_item = match kind {
            0 => 1,
            1 => FLOAT_WORDS,
            2 => FUN_WORDS,
            _ => return Err(error(format!("invalid BEAM allocation kind {kind}"))),
        };
        heap_words += words_per_item * number;
        if heap_words > MAX_HEAP_WORDS {
            return Err(error("BEAM allocation list exceeds the heap word limit"));
        }
    }
    Ok(())
}

/// Skip a fixed number of compact operands.
///
/// # Examples
///
/// ```rust,ignore
/// let mut cursor = 0;
/// skip_operands(&[0x10, 0x20], &mut cursor, 2)?;
/// assert_eq!(cursor, 2);
/// ```
fn skip_operands(data: &[u8], cursor: &mut usize, count: usize) -> Result<(), BeamError> {
    if count > data.len().saturating_sub(*cursor) {
        return Err(error("truncated BEAM instruction operands"));
    }
    for _ in 0..count {
        skip_operand(data, cursor)?;
    }
    Ok(())
}

/// Skip one compact operand, including a supported extended payload.
///
/// # Examples
///
/// ```rust,ignore
/// let mut cursor = 0;
/// skip_operand(&[0x07, 0, 0, 0, 0, 0, 0, 0, 0], &mut cursor)?;
/// assert_eq!(cursor, 9);
/// ```
fn skip_operand(data: &[u8], cursor: &mut usize) -> Result<(), BeamError> {
    let first_byte = *data
        .get(*cursor)
        .ok_or_else(|| error("truncated BEAM operand"))?;
    *cursor += 1;
    let tag = first_byte & COMPACT_TAG_MASK;
    if tag != TAG_LITERAL {
        return skip_integer_payload(data, cursor, first_byte);
    }
    if first_byte & COMPACT_EXTENDED_FLAG != 0 {
        return Err(error(format!(
            "unsupported extended BEAM operand {first_byte:#04x}"
        )));
    }
    match first_byte >> COMPACT_SHORT_VALUE_SHIFT {
        0 => skip_bytes(data, cursor, 8),
        1 => Ok(()),
        2 | 4 => skip_operand(data, cursor),
        3 => {
            let count = usize::try_from(read_operand(data, cursor, TAG_UNSIGNED)?)
                .map_err(|_| error("BEAM allocation list is too large"))?;
            let item_count = count
                .checked_mul(2)
                .ok_or_else(|| error("BEAM allocation list size overflow"))?;
            skip_operands(data, cursor, item_count)
        }
        5 => skip_operands(data, cursor, 2),
        tag => Err(error(format!(
            "unsupported BEAM extended operand tag {tag}"
        ))),
    }
}

/// Skip the extension bytes described by a compact integer's first byte.
///
/// # Examples
///
/// ```rust,ignore
/// let mut cursor = 0;
/// skip_integer_payload(&[0, 7], &mut cursor, 0x19)?;
/// assert_eq!(cursor, 2);
/// ```
fn skip_integer_payload(data: &[u8], cursor: &mut usize, first_byte: u8) -> Result<(), BeamError> {
    if first_byte & COMPACT_EXTENDED_FLAG == 0 {
        return Ok(());
    }
    if first_byte & COMPACT_SIGN_FLAG == 0 {
        return skip_bytes(data, cursor, 1);
    }
    let size_code = usize::from(first_byte >> 5);
    let byte_count = if size_code == 7 {
        usize::try_from(read_operand(data, cursor, TAG_UNSIGNED)?)
            .map_err(|_| error("BEAM integer operand is too large"))?
            .checked_add(9)
            .ok_or_else(|| error("BEAM integer operand size overflow"))?
    } else {
        size_code + 2
    };
    skip_bytes(data, cursor, byte_count)
}

/// Advance the cursor by a byte count if the full range exists.
///
/// # Examples
///
/// ```rust,ignore
/// let mut cursor = 0;
/// skip_bytes(&[0, 0], &mut cursor, 2)?;
/// assert_eq!(cursor, 2);
/// ```
fn skip_bytes(data: &[u8], cursor: &mut usize, count: usize) -> Result<(), BeamError> {
    let end = cursor
        .checked_add(count)
        .filter(|&end| end <= data.len())
        .ok_or_else(|| error("truncated BEAM operand"))?;
    *cursor = end;
    Ok(())
}

/// Read the extended list header of an operand list, such as the
/// environment of `make_fun3`, and return the number of items in the list.
///
/// The header is the literal tag with `LIST_SUBTAG`, then an unsigned item
/// count. A count larger than the bytes left in the chunk is malformed.
///
/// # Examples
///
/// ```rust,ignore
/// let mut cursor = 0;
/// // {list,[{x,0}]}
/// assert_eq!(read_list_count(&[0x17, 0x10, 0x03], &mut cursor)?, 1);
/// assert_eq!(cursor, 2);
/// ```
fn read_list_count(data: &[u8], cursor: &mut usize) -> Result<usize, BeamError> {
    if read_integer(data, cursor, TAG_LITERAL)? != LIST_SUBTAG {
        return Err(error("unsupported BEAM list operand"));
    }
    let item_count = usize::try_from(read_operand(data, cursor, TAG_UNSIGNED)?)
        .map_err(|_| error("BEAM list operand is too large"))?;
    // Each item takes at least one byte, so a larger count is malformed.
    if item_count > data.len().saturating_sub(*cursor) {
        return Err(error("BEAM list operand exceeds code chunk"));
    }
    Ok(item_count)
}

/// Read the extended list header of `select_val`, `select_tuple_arity`, or
/// `update_record` and return the number of pairs in the list. An odd item
/// count is malformed.
///
/// # Examples
///
/// ```rust,ignore
/// let mut cursor = 0;
/// // {list,[...]} with four items: two pairs.
/// assert_eq!(read_pair_count(&[0x17, 0x40], &mut cursor)?, 2);
/// ```
fn read_pair_count(data: &[u8], cursor: &mut usize) -> Result<usize, BeamError> {
    let item_count = read_list_count(data, cursor)?;
    if item_count % 2 != 0 {
        return Err(error("BEAM pair list has an incomplete pair"));
    }
    Ok(item_count / 2)
}

/// Read an X or Y register operand.
///
/// # Examples
///
/// ```rust,ignore
/// let mut cursor = 0;
/// assert!(matches!(read_register(&[0x03], &mut cursor)?, Operand::X(0)));
/// assert_eq!(cursor, 1);
/// ```
fn read_register(data: &[u8], cursor: &mut usize) -> Result<Operand, BeamError> {
    let first = *data
        .get(*cursor)
        .ok_or_else(|| error("truncated BEAM operand"))?;
    let tag = first & COMPACT_TAG_MASK;
    match tag {
        TAG_REGISTER => Ok(Operand::X(read_operand_u32(data, cursor, TAG_REGISTER)?)),
        TAG_Y_REGISTER => Ok(Operand::Y(read_operand_u32(data, cursor, TAG_Y_REGISTER)?)),
        _ => Err(error(format!("unsupported BEAM register tag {tag}"))),
    }
}

/// The operation of an [`Instruction::FloatArith`].
#[derive(Debug, Clone, Copy)]
pub(crate) enum FloatOperation {
    Add,
    Subtract,
    Multiply,
    Divide,
}

/// Return true when the next operand is a float register (extended subtag
/// 2), and leave the cursor in place.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(is_float_register(&[0x27, 0x00], 0)?);
/// assert!(!is_float_register(&[0x03], 0)?);
/// ```
fn is_float_register(data: &[u8], cursor: usize) -> Result<bool, BeamError> {
    let first = *data
        .get(cursor)
        .ok_or_else(|| error("truncated BEAM operand"))?;
    if first & COMPACT_TAG_MASK != TAG_LITERAL {
        return Ok(false);
    }
    let mut peek = cursor;
    Ok(read_integer(data, &mut peek, TAG_LITERAL)? == FLOAT_REGISTER_SUBTAG)
}

/// Read a float register operand `{fr, N}`: extended subtag 2, then the
/// register number as an unsigned operand.
///
/// # Examples
///
/// ```rust,ignore
/// let mut cursor = 0;
/// assert_eq!(read_float_register(&[0x27, 0x20], &mut cursor)?, 2);
/// assert_eq!(cursor, 2);
/// ```
///
/// # Errors
///
/// An operand that is not a float register is an error.
fn read_float_register(data: &[u8], cursor: &mut usize) -> Result<u32, BeamError> {
    is_float_register(data, *cursor)?
        .then_some(())
        .ok_or_else(|| error("expected a BEAM float register operand"))?;
    read_integer(data, cursor, TAG_LITERAL)?;
    u32::try_from(read_operand(data, cursor, TAG_UNSIGNED)?)
        .map_err(|_| error("BEAM float register number is too large"))
}

/// Read the fail label of a float arithmetic instruction, which must be 0,
/// as the only form that ERTS loads (emu ops.tab:1329-1333).
///
/// # Examples
///
/// ```rust,ignore
/// let mut cursor = 0;
/// read_zero_fail_label(&[0x05], &mut cursor)?;
/// assert!(read_zero_fail_label(&[0x15], &mut 0).is_err());
/// ```
fn read_zero_fail_label(data: &[u8], cursor: &mut usize) -> Result<(), BeamError> {
    (read_operand(data, cursor, TAG_LABEL)? == 0)
        .then_some(())
        .ok_or_else(|| error("unsupported BEAM float instruction with a fail label"))
}

/// Read one supported source operand for a `move` instruction.
///
/// # Examples
///
/// ```rust,ignore
/// let literals = vec![Some(Term::binary(b"ok".to_vec()))];
/// let mut cursor = 0;
/// let operand = read_move_operand(&[0x47, 0], &mut cursor, &[], &literals)?;
/// assert!(matches!(operand, Operand::Value(Term::Binary(bytes)) if bytes == b"ok"));
/// ```
fn read_move_operand(
    data: &[u8],
    cursor: &mut usize,
    atoms: &[Atom],
    literals: &[Option<Term>],
) -> Result<Operand, BeamError> {
    let first = *data
        .get(*cursor)
        .ok_or_else(|| error("truncated BEAM operand"))?;
    match first & COMPACT_TAG_MASK {
        TAG_INTEGER | TAG_ATOM => Ok(Operand::Value(read_value(data, cursor, atoms)?)),
        TAG_LITERAL => {
            let subtag = read_integer(data, cursor, TAG_LITERAL)?;
            match subtag {
                1 => Ok(Operand::Value(Term::nil())),
                2 => read_move_operand(data, cursor, atoms, literals),
                4 => {
                    let literal_index = usize::try_from(read_operand(data, cursor, TAG_UNSIGNED)?)
                        .map_err(|_| error("BEAM literal index exceeds platform limit"))?;
                    match literals.get(literal_index) {
                        Some(Some(value)) => Ok(Operand::Value(value.clone())),
                        Some(None) => Ok(Operand::UnsupportedLiteral(literal_index)),
                        None => Err(error(format!(
                            "BEAM literal index {literal_index} is out of range"
                        ))),
                    }
                }
                5 => {
                    let register = read_move_operand(data, cursor, atoms, literals)?;
                    skip_operand(data, cursor)?;
                    Ok(register)
                }
                _ => Err(error(format!(
                    "unsupported extended BEAM move operand {subtag}"
                ))),
            }
        }
        TAG_REGISTER | TAG_Y_REGISTER => read_register(data, cursor),
        tag => Err(error(format!("unsupported move operand tag {tag}"))),
    }
}

#[cfg(test)]
#[path = "../../../tests/beam/code/instruction.rs"]
mod tests;
