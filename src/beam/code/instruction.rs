//! Parse Code chunk instructions into the runtime instruction model.

use super::{
    COMPACT_EXTENDED_FLAG, COMPACT_SHORT_VALUE_SHIFT, COMPACT_SIGN_FLAG, COMPACT_TAG_MASK,
    FIRST_LABEL_NUMBER, LIST_SUBTAG, TAG_ATOM, TAG_INTEGER, TAG_LABEL, TAG_LITERAL, TAG_REGISTER,
    TAG_UNSIGNED, TAG_Y_REGISTER, WORD_SIZE, atom_at, read_integer, read_operand, read_u32,
    read_value,
};
use crate::error::{BeamError, error};
use crate::term::{Atom, Term};

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
const OPCODE_IS_FUNCTION: u8 = 77;
const OPCODE_APPLY: u8 = 112;
const OPCODE_APPLY_LAST: u8 = 113;
const OPCODE_IS_FUNCTION2: u8 = 115;
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
const OPCODE_BS_GET_BINARY2: u8 = 119;
const OPCODE_BS_SKIP_BITS2: u8 = 120;
const OPCODE_GC_BIF1: u8 = 124;
const OPCODE_GC_BIF2: u8 = 125;
const OPCODE_BS_GET_TAIL: u8 = 165;
const OPCODE_BS_START_MATCH3: u8 = 166;
const OPCODE_BS_GET_POSITION: u8 = 167;
const OPCODE_BS_SET_POSITION: u8 = 168;
const OPCODE_BS_START_MATCH4: u8 = 170;
const OPCODE_BS_GET_UTF8: u8 = 138;
const OPCODE_BS_SKIP_UTF8: u8 = 139;
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
const OPCODE_BS_CREATE_BIN: u8 = 177;
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
const OPCODE_MAKE_FUN3: u8 = 171;
const OPCODE_BS_MATCH: u8 = 182;
const OPCODE_UPDATE_RECORD: u8 = 181;
const OPCODE_RECV_MARKER_BIND: u8 = 173;
const OPCODE_RECV_MARKER_CLEAR: u8 = 174;
const OPCODE_RECV_MARKER_RESERVE: u8 = 175;
const OPCODE_RECV_MARKER_USE: u8 = 176;
#[derive(Debug, Clone)]
/// An instruction that the parser can read from a BEAM `Code` chunk.
///
/// Byte examples use hexadecimal values. Compact operands use the short form
/// in the Code chunk grammar. `End` represents the stream terminator; the
/// executor does not run it.
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
    /// Erlang: `b(X) -> c(X), ok.` compiles to `{call,1,{f,6}}`.
    Call { arity: u32, label: u32 },
    /// Releases `deallocate` Y registers and calls a local function in place
    /// of the active one.
    ///
    /// Encoding: opcode, arity, label, deallocate. Example:
    /// `[0x05, 0x30, 0x35, 0x20]` calls label 3 with arity 3 and releases two
    /// Y registers.
    ///
    /// Erlang: `io:request/3` compiles `request(group_leader(), Request, Error)`
    /// to `{call_last,3,{io,request,3},2}`.
    CallLast {
        arity: u32,
        label: u32,
        deallocate: u32,
    },
    /// Tail-calls a local function.
    ///
    /// Erlang: `c(X) -> d(X).` compiles to `{call_only,1,{f,8}}`.
    CallOnly { arity: u32, label: u32 },
    /// Calls an imported function. `tail` marks `call_ext_last` and
    /// `call_ext_only`; `deallocate` is the stack size to release first.
    ///
    /// Erlang: `d(X) -> lists:reverse(X).` compiles to
    /// `{call_ext_only,1,{extfunc,lists,reverse,1}}`.
    CallExternal {
        arity: u32,
        module: String,
        name: String,
        tail: bool,
        deallocate: u32,
    },
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
    /// Calls a BIF through `bif0` (opcode 9), `bif1` (opcode 10), or `bif2`
    /// (opcode 11) and writes its value to a register. `bif0` has no fail
    /// label and decodes with `fail_label` 0. With fail label 0, a BIF failure
    /// raises its exception; with another fail label, it jumps to the label.
    ///
    /// Encoding: `bif0` has opcode, import index, destination register;
    /// `bif1` and `bif2` have opcode, fail label, import index, one or two
    /// arguments, destination register. An argument can be a typed register
    /// (extended subtag 5). Example: `io:io_request/2` in the pinned `io.beam`
    /// has `[0x0A, 0x05, 0xE0, 0x57, 0x03, 0x40, 0x23]`, which calls import 14
    /// (`erlang:node/1`) with `x0` typed as a pid and writes `x2`, and
    /// `[0x09, 0xF0, 0x53]`, which calls import 15 (`erlang:node/0`) and
    /// writes `x5`. The pinned `gen_statem.beam` has
    /// `[0x0B, 0x05, 0x08, 0x21, 0x21, 0x03, 0x03]`, which calls import 33
    /// (`erlang:element/2`) with the integer 2 and `x0` and writes `x0`.
    ///
    /// Erlang: `h() -> self().` compiles to `{bif,self,{f,0},[],{x,0}}`, and
    /// `node(Pid) =:= node()` in `io:io_request/2` compiles to
    /// `{bif,node,{f,0},[{tr,{x,0},pid}],{x,2}}` and
    /// `{bif,node,nofail,[],{x,5}}`. The `STACKTRACE()` macro of
    /// `gen_statem`, `element(2, erlang:process_info(self(),
    /// current_stacktrace))`, compiles to
    /// `{bif,element,{f,0},[{integer,2},{x,0}],{x,0}}`. The guard
    /// `when tuple_size(X) >= 2` in `fixtures/group/group_probe.erl`
    /// compiles to `{bif,tuple_size,{f,5},[{x,0}],{x,1}}`, which is
    /// `[0x0A, 0x55, 0x10, 0x03, 0x13]` with import 1.
    Bif {
        /// Label to jump to when the BIF fails, or 0 to raise the exception.
        fail_label: u32,
        /// Module of the BIF, for example `erlang`.
        module: String,
        /// Name of the BIF, for example `node`.
        name: String,
        /// Argument operands of the BIF, in argument order.
        arguments: Vec<Operand>,
        /// Register for the BIF value.
        destination: Operand,
    },
    /// An instruction that is parsed so the module can load, but cannot run.
    Unsupported(u8),
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
    /// stack trace.
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
    /// `x0` becomes `{'EXIT', {Reason, Stack}}` for class `error` and
    /// `{'EXIT', Reason}` for class `exit`.
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
    /// Erlang: `f(1) -> one; f(2) -> two; f(_) -> many.` compiles to
    /// `{select_val,{x,0},{f,16},{list,[{integer,1},{f,15},{integer,2},{f,14}]}}`.
    SelectValue {
        value: Operand,
        fail_label: u32,
        choices: Vec<(Term, u32)>,
    },
    /// Copies the value of each key in a map to its destination register, or
    /// jumps to `fail_label` when a key is missing. `pairs` holds each key
    /// and its destination.
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
    /// Jumps to `fail_label` when the left operand is less than the right
    /// operand (`is_ge`, opcode 40). PBeam compares two integers only; any
    /// other operand pair gives a runtime error.
    ///
    /// Encoding: opcode, fail label, two operands. A typed register (extended
    /// subtag 5) reads as its register. Example: `[0x28, 0x55, 0x57, 0x13,
    /// 0x10, 0x21]` fails to label 5 when `x1` is less than the integer 2.
    ///
    /// Erlang: the guard `when tuple_size(X) >= 2` in
    /// `fixtures/group/group_probe.erl` compiles to
    /// `{test,is_ge,{f,5},[{tr,{x,1},{t_integer,{0,16777215}}},{integer,2}]}`.
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
    /// Sends `x1` to the process in `x0`, then writes the message to `x0`.
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
    /// An instruction that has no runtime effect, such as `test_heap` or
    /// `init_yregs`.
    ///
    /// Erlang: `g(X) -> {X, X}.` compiles to `{test_heap,3,1}` first.
    Noop,
    /// Returns the value in `x0` from the active function.
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
    Map,
}

/// A source or destination in the supported `move` instruction.
///
/// `Value` holds a supported immediate term. `X` uses compact register tag 3,
/// and `Y` uses compact register tag 4. For example, `move x0, y0` is encoded
/// as `[0x40, 0x03, 0x04]`.
#[derive(Debug, Clone)]
pub(crate) enum Operand {
    Value(Term),
    UnsupportedLiteral(usize),
    X(u32),
    Y(u32),
}

/// Decode the `Code` header and instruction stream.
///
/// # Examples
///
/// ```rust,ignore
/// let mut code = vec![0, 0, 0, 16]; // sub-size
/// code.extend_from_slice(&[0; 16]); // instruction set, opcode max, labels, functions
/// code.push(0x03); // end
/// let (instructions, labels) = parse_code(&code, &[], &[], &[])?;
/// assert_eq!(labels, 0);
/// assert!(matches!(instructions.as_slice(), [Instruction::End]));
/// ```
pub(super) fn parse_code(
    data: &[u8],
    atoms: &[Atom],
    imports: &[(String, String, u32)],
    literals: &[Option<Term>],
) -> Result<(Vec<Instruction>, u32), BeamError> {
    if data.len() < CODE_HEADER_SIZE
        || read_u32(&data[..WORD_SIZE])? != CODE_HEADER_SUB_SIZE
        || read_u32(&data[WORD_SIZE..WORD_SIZE * 2])? != INSTRUCTION_SET_ERLANG
    {
        return Err(error("unsupported BEAM code header"));
    }
    let number_of_labels =
        read_u32(&data[CODE_LABEL_COUNT_OFFSET..CODE_LABEL_COUNT_OFFSET + WORD_SIZE])?;
    let declared_function_count =
        read_u32(&data[CODE_FUNCTION_COUNT_OFFSET..CODE_HEADER_SIZE])? as usize;
    let mut cursor = CODE_HEADER_SIZE;
    let mut instructions = Vec::new();
    let mut function_info_count = 0;
    loop {
        let opcode = *data
            .get(cursor)
            .ok_or_else(|| error("truncated BEAM instruction stream"))?;
        cursor += 1;
        let instruction = match opcode {
            OPCODE_LABEL => {
                let label = read_operand(data, &mut cursor, TAG_UNSIGNED)? as u32;
                if label < FIRST_LABEL_NUMBER || label > number_of_labels {
                    return Err(error(format!(
                        "BEAM label {label} is outside declared range 1..={number_of_labels}"
                    )));
                }
                Instruction::Label(label)
            }
            OPCODE_FUNCTION_INFO => {
                function_info_count += 1;
                let module = atom_at(atoms, read_operand(data, &mut cursor, TAG_ATOM)? as usize)?
                    .name()
                    .to_owned();
                let name = atom_at(atoms, read_operand(data, &mut cursor, TAG_ATOM)? as usize)?
                    .name()
                    .to_owned();
                let arity = read_operand(data, &mut cursor, TAG_UNSIGNED)? as u32;
                Instruction::FunctionInfo {
                    module,
                    name,
                    arity,
                }
            }
            OPCODE_RETURN => Instruction::Return,
            OPCODE_CALL => Instruction::Call {
                arity: read_operand(data, &mut cursor, TAG_UNSIGNED)? as u32,
                label: read_operand(data, &mut cursor, TAG_LABEL)? as u32,
            },
            OPCODE_CALL_ONLY => Instruction::CallOnly {
                arity: read_operand(data, &mut cursor, TAG_UNSIGNED)? as u32,
                label: read_operand(data, &mut cursor, TAG_LABEL)? as u32,
            },
            OPCODE_JUMP => Instruction::Jump(read_operand(data, &mut cursor, TAG_LABEL)? as u32),
            OPCODE_SELECT_VAL => {
                let value = read_move_operand(data, &mut cursor, atoms, literals)?;
                let fail_label = read_operand(data, &mut cursor, TAG_LABEL)? as u32;
                let pair_count = read_pair_count(data, &mut cursor)?;
                let mut choices = Vec::with_capacity(pair_count);
                for _ in 0..pair_count {
                    let choice = read_move_operand(data, &mut cursor, atoms, literals)?;
                    let Operand::Value(choice) = choice else {
                        return Err(error("unsupported BEAM selection value"));
                    };
                    let target = read_operand(data, &mut cursor, TAG_LABEL)? as u32;
                    choices.push((choice, target));
                }
                Instruction::SelectValue {
                    value,
                    fail_label,
                    choices,
                }
            }
            OPCODE_SELECT_TUPLE_ARITY => {
                let value = read_move_operand(data, &mut cursor, atoms, literals)?;
                let fail_label = read_operand(data, &mut cursor, TAG_LABEL)? as u32;
                let pair_count = read_pair_count(data, &mut cursor)?;
                let mut choices = Vec::with_capacity(pair_count);
                for _ in 0..pair_count {
                    let arity = read_operand(data, &mut cursor, TAG_UNSIGNED)? as u32;
                    let target = read_operand(data, &mut cursor, TAG_LABEL)? as u32;
                    choices.push((arity, target));
                }
                Instruction::SelectTupleArity {
                    value,
                    fail_label,
                    choices,
                }
            }
            OPCODE_TEST_ARITY => Instruction::TestArity {
                fail_label: read_operand(data, &mut cursor, TAG_LABEL)? as u32,
                value: read_move_operand(data, &mut cursor, atoms, literals)?,
                arity: read_operand(data, &mut cursor, TAG_UNSIGNED)? as u32,
            },
            OPCODE_CALL_LAST => Instruction::CallLast {
                arity: read_operand(data, &mut cursor, TAG_UNSIGNED)? as u32,
                label: read_operand(data, &mut cursor, TAG_LABEL)? as u32,
                deallocate: read_operand(data, &mut cursor, TAG_UNSIGNED)? as u32,
            },
            OPCODE_TEST_TAGGED_TUPLE => {
                let fail_label = read_operand(data, &mut cursor, TAG_LABEL)? as u32;
                let value = read_move_operand(data, &mut cursor, atoms, literals)?;
                let arity = read_operand(data, &mut cursor, TAG_UNSIGNED)? as u32;
                let tag = read_value(data, &mut cursor, atoms)?;
                Instruction::TestTaggedTuple {
                    fail_label,
                    value,
                    arity,
                    tag,
                }
            }
            OPCODE_TEST_EXACT_EQ => Instruction::TestExactEqual {
                fail_label: read_operand(data, &mut cursor, TAG_LABEL)? as u32,
                left: read_move_operand(data, &mut cursor, atoms, literals)?,
                right: read_move_operand(data, &mut cursor, atoms, literals)?,
            },
            OPCODE_TEST_GE => Instruction::TestGreaterEqual {
                fail_label: read_operand(data, &mut cursor, TAG_LABEL)? as u32,
                left: read_move_operand(data, &mut cursor, atoms, literals)?,
                right: read_move_operand(data, &mut cursor, atoms, literals)?,
            },
            OPCODE_TRY | OPCODE_CATCH => Instruction::Catch {
                register: read_operand(data, &mut cursor, TAG_Y_REGISTER)? as u32,
                label: read_operand(data, &mut cursor, TAG_LABEL)? as u32,
            },
            OPCODE_TRY_END => Instruction::TryEnd {
                register: read_operand(data, &mut cursor, TAG_Y_REGISTER)? as u32,
            },
            OPCODE_TRY_CASE => Instruction::TryCase {
                register: read_operand(data, &mut cursor, TAG_Y_REGISTER)? as u32,
            },
            OPCODE_CATCH_END => Instruction::CatchEnd {
                register: read_operand(data, &mut cursor, TAG_Y_REGISTER)? as u32,
            },
            OPCODE_SEND => Instruction::Send,
            OPCODE_REMOVE_MESSAGE => Instruction::RemoveMessage,
            OPCODE_LOOP_REC => Instruction::LoopReceive {
                fail_label: read_operand(data, &mut cursor, TAG_LABEL)? as u32,
                destination: read_register(data, &mut cursor)?,
            },
            OPCODE_LOOP_REC_END => {
                Instruction::LoopReceiveEnd(read_operand(data, &mut cursor, TAG_LABEL)? as u32)
            }
            OPCODE_WAIT => Instruction::Wait(read_operand(data, &mut cursor, TAG_LABEL)? as u32),
            OPCODE_WAIT_TIMEOUT => Instruction::WaitTimeout {
                label: read_operand(data, &mut cursor, TAG_LABEL)? as u32,
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
                index: read_operand(data, &mut cursor, TAG_UNSIGNED)? as u32,
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
                fail_label: read_operand(data, &mut cursor, TAG_LABEL)? as u32,
                map: read_move_operand(data, &mut cursor, atoms, literals)?,
                pairs: read_operand_pairs(data, &mut cursor, atoms, literals)?,
            },
            OPCODE_PUT_MAP_ASSOC => {
                let _fail_label = read_operand(data, &mut cursor, TAG_LABEL)?;
                let map = read_move_operand(data, &mut cursor, atoms, literals)?;
                let destination = read_register(data, &mut cursor)?;
                let _live_registers = read_operand(data, &mut cursor, TAG_UNSIGNED)?;
                Instruction::PutMapAssoc {
                    map,
                    destination,
                    pairs: read_operand_pairs(data, &mut cursor, atoms, literals)?,
                }
            }
            OPCODE_SWAP => Instruction::Swap(
                read_register(data, &mut cursor)?,
                read_register(data, &mut cursor)?,
            ),
            OPCODE_TEST_IS_ATOM
            | OPCODE_TEST_IS_PID
            | OPCODE_TEST_IS_REFERENCE
            | OPCODE_TEST_IS_TUPLE
            | OPCODE_TEST_IS_LIST
            | OPCODE_TEST_IS_NIL
            | OPCODE_TEST_IS_NONEMPTY_LIST
            | OPCODE_TEST_IS_BINARY
            | OPCODE_TEST_IS_MAP => {
                let fail_label = read_operand(data, &mut cursor, TAG_LABEL)? as u32;
                let value = read_move_operand(data, &mut cursor, atoms, literals)?;
                let kind = test_kind(opcode)?;
                Instruction::Test {
                    kind,
                    fail_label,
                    value,
                }
            }
            OPCODE_TEST_HEAP | OPCODE_INIT_YREGS => {
                skip_unsupported_instruction(opcode, data, &mut cursor)?;
                Instruction::Noop
            }
            OPCODE_PUT_LIST => Instruction::PutList {
                head: read_move_operand(data, &mut cursor, atoms, literals)?,
                tail: read_move_operand(data, &mut cursor, atoms, literals)?,
                destination: read_register(data, &mut cursor)?,
            },
            OPCODE_LINE => {
                let _line_index = read_operand(data, &mut cursor, TAG_UNSIGNED)?;
                continue;
            }
            OPCODE_CALL_EXT | OPCODE_CALL_EXT_LAST | OPCODE_CALL_EXT_ONLY => {
                let arity = read_operand(data, &mut cursor, TAG_UNSIGNED)? as u32;
                let index = read_operand(data, &mut cursor, TAG_UNSIGNED)? as usize;
                let (module, name, imported_arity) = imports
                    .get(index)
                    .ok_or_else(|| error(format!("BEAM import index {index} out of range")))?;
                if arity != *imported_arity {
                    return Err(error(format!(
                        "BEAM external call arity {arity} does not match import arity {imported_arity}"
                    )));
                }
                let deallocate = if opcode == OPCODE_CALL_EXT_LAST {
                    read_operand(data, &mut cursor, TAG_UNSIGNED)? as u32
                } else {
                    0
                };
                Instruction::CallExternal {
                    arity,
                    module: module.clone(),
                    name: name.clone(),
                    tail: opcode != OPCODE_CALL_EXT,
                    deallocate,
                }
            }
            OPCODE_APPLY | OPCODE_APPLY_LAST => {
                let arity = read_operand(data, &mut cursor, TAG_UNSIGNED)? as u32;
                let deallocate = if opcode == OPCODE_APPLY_LAST {
                    read_operand(data, &mut cursor, TAG_UNSIGNED)? as u32
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
                let (module, name) = bif_import(
                    imports,
                    read_operand(data, &mut cursor, TAG_UNSIGNED)? as usize,
                )?;
                Instruction::Bif {
                    fail_label: 0,
                    module,
                    name,
                    arguments: Vec::new(),
                    destination: read_register(data, &mut cursor)?,
                }
            }
            OPCODE_BIF1 | OPCODE_BIF2 => {
                let fail_label = read_operand(data, &mut cursor, TAG_LABEL)? as u32;
                let (module, name) = bif_import(
                    imports,
                    read_operand(data, &mut cursor, TAG_UNSIGNED)? as usize,
                )?;
                let argument_count = if opcode == OPCODE_BIF1 { 1 } else { 2 };
                let arguments = (0..argument_count)
                    .map(|_| read_move_operand(data, &mut cursor, atoms, literals))
                    .collect::<Result<Vec<_>, _>>()?;
                Instruction::Bif {
                    fail_label,
                    module,
                    name,
                    arguments,
                    destination: read_register(data, &mut cursor)?,
                }
            }
            OPCODE_ALLOCATE | OPCODE_ALLOCATE_HEAP => {
                let stack_size = read_operand(data, &mut cursor, TAG_UNSIGNED)? as u32;
                if opcode == OPCODE_ALLOCATE_HEAP {
                    skip_operand(data, &mut cursor)?;
                }
                let _live_registers = read_operand(data, &mut cursor, TAG_UNSIGNED)?;
                Instruction::Allocate { stack_size }
            }
            OPCODE_DEALLOCATE => Instruction::Deallocate {
                stack_size: read_operand(data, &mut cursor, TAG_UNSIGNED)? as u32,
            },
            OPCODE_TRIM => Instruction::Trim {
                count: read_operand(data, &mut cursor, TAG_UNSIGNED)? as u32,
                remaining: read_operand(data, &mut cursor, TAG_UNSIGNED)? as u32,
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
                return Ok((instructions, number_of_labels));
            }
            _ => {
                skip_unsupported_instruction(opcode, data, &mut cursor)?;
                Instruction::Unsupported(opcode)
            }
        };
        instructions.push(instruction);
    }
}

/// Return the module and name of the import that a `bif` instruction calls.
///
/// # Examples
///
/// ```rust,ignore
/// let imports = vec![("erlang".to_owned(), "node".to_owned(), 0)];
/// assert_eq!(bif_import(&imports, 0)?, ("erlang".to_owned(), "node".to_owned()));
/// assert!(bif_import(&imports, 1).is_err());
/// ```
fn bif_import(
    imports: &[(String, String, u32)],
    index: usize,
) -> Result<(String, String), BeamError> {
    let (module, name, _arity) = imports
        .get(index)
        .ok_or_else(|| error(format!("BEAM BIF import index {index} out of range")))?;
    Ok((module.clone(), name.clone()))
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
        OPCODE_TEST_IS_REFERENCE => Ok(TestKind::Reference),
        OPCODE_TEST_IS_TUPLE => Ok(TestKind::Tuple),
        OPCODE_TEST_IS_LIST => Ok(TestKind::List),
        OPCODE_TEST_IS_NIL => Ok(TestKind::Nil),
        OPCODE_TEST_IS_NONEMPTY_LIST => Ok(TestKind::NonEmptyList),
        OPCODE_TEST_IS_BINARY => Ok(TestKind::Binary),
        OPCODE_TEST_IS_MAP => Ok(TestKind::Map),
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
        let position = read_operand(data, cursor, TAG_UNSIGNED)? as u32;
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
/// skip_unsupported_instruction(OPCODE_GC_BIF2, &[0, 0, 0, 0, 0, 0], &mut cursor)?;
/// assert_eq!(cursor, 6);
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
        OPCODE_PUT_MAP_ASSOC
            | OPCODE_PUT_MAP_EXACT
            | OPCODE_HAS_MAP_FIELDS
            | OPCODE_GET_MAP_ELEMENTS
            | OPCODE_GET_RECORD_ELEMENTS
            | OPCODE_PUT_RECORD
            | OPCODE_PUT_TUPLE2
            | OPCODE_MAKE_FUN3
            | OPCODE_INIT_YREGS
            | OPCODE_BS_CREATE_BIN
            | OPCODE_BS_MATCH
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
/// assert_eq!(unsupported_opcode_operand_count(OPCODE_GC_BIF2), Some(6));
/// assert_eq!(unsupported_opcode_operand_count(255), None);
/// ```
fn unsupported_opcode_operand_count(opcode: u8) -> Option<usize> {
    match opcode {
        OPCODE_RAISE => Some(2),
        OPCODE_CALL_FUN2 | OPCODE_IS_FUNCTION2 => Some(3),
        OPCODE_BS_GET_BINARY2 => Some(7),
        OPCODE_BS_SKIP_BITS2 => Some(5),
        OPCODE_BS_START_MATCH3 | OPCODE_BS_START_MATCH4 => Some(4),
        OPCODE_BS_GET_TAIL | OPCODE_BS_GET_POSITION => Some(3),
        OPCODE_BS_GET_UTF8 => Some(5),
        OPCODE_BS_SKIP_UTF8 => Some(4),
        OPCODE_BS_SET_POSITION => Some(2),
        OPCODE_GC_BIF1 => Some(5),
        OPCODE_GC_BIF2 => Some(6),
        OPCODE_BADRECORD => Some(1),
        OPCODE_NIF_START | OPCODE_IF_END | OPCODE_BUILD_STACKTRACE => Some(0),
        // `raw_raise` is in `gen_statem:call/3`.
        OPCODE_RAW_RAISE => Some(0),
        OPCODE_TEST_HEAP => Some(2),
        OPCODE_BIF0 => Some(2),
        OPCODE_TEST_LT | OPCODE_TEST_EQ | OPCODE_TEST_NE | OPCODE_TEST_EXACT_EQ
        | OPCODE_TEST_EXACT_NE => Some(3),
        OPCODE_TEST_TAGGED_TUPLE => Some(4),
        OPCODE_TEST_IS_INTEGER
        | OPCODE_TEST_IS_FLOAT
        | OPCODE_TEST_IS_NUMBER
        | OPCODE_TEST_IS_ATOM
        | OPCODE_TEST_IS_PID
        | OPCODE_TEST_IS_BINARY
        | OPCODE_TEST_IS_LIST
        | OPCODE_TEST_IS_MAP
        | OPCODE_TEST_IS_BOOLEAN
        | OPCODE_TEST_IS_BITSTR
        | OPCODE_IS_FUNCTION => Some(2),
        // `is_port Fail Src` is in `erlang:is_port/1` and `erlang:open_port/2`.
        OPCODE_TEST_IS_PORT => Some(2),
        // `try_case_end Src` is in `erlang:processes_next/1`.
        OPCODE_BADMATCH | OPCODE_CASE_END | OPCODE_CALL_FUN | OPCODE_TRY_CASE_END => Some(1),
        OPCODE_PUT_MAP_ASSOC | OPCODE_PUT_MAP_EXACT => Some(5),
        OPCODE_GET_MAP_ELEMENTS | OPCODE_HAS_MAP_FIELDS | OPCODE_GET_RECORD_ELEMENTS => Some(3),
        OPCODE_PUT_RECORD => Some(6),
        OPCODE_GET_TL => Some(2),
        OPCODE_PUT_TUPLE2 => Some(2),
        OPCODE_INIT_YREGS => Some(1),
        OPCODE_MAKE_FUN3 => Some(3),
        OPCODE_BS_CREATE_BIN => Some(6),
        OPCODE_BS_INIT_WRITABLE => Some(0),
        OPCODE_BS_MATCH => Some(3),
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
    let size_code = (first_byte >> 5) as usize;
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

/// Read the extended list header of `select_val`, `select_tuple_arity`, or
/// `update_record` and return the number of pairs in the list.
///
/// The header is the literal tag with `LIST_SUBTAG`, then an unsigned item
/// count. A count that is odd, or larger than the bytes left in the chunk,
/// is malformed.
///
/// # Examples
///
/// ```rust,ignore
/// let mut cursor = 0;
/// // {list,[...]} with four items: two pairs.
/// assert_eq!(read_pair_count(&[0x17, 0x40], &mut cursor)?, 2);
/// ```
fn read_pair_count(data: &[u8], cursor: &mut usize) -> Result<usize, BeamError> {
    if read_integer(data, cursor, TAG_LITERAL)? != LIST_SUBTAG {
        return Err(error("unsupported BEAM pair list"));
    }
    let item_count = usize::try_from(read_operand(data, cursor, TAG_UNSIGNED)?)
        .map_err(|_| error("BEAM pair list is too large"))?;
    // Each item takes at least one byte, so a larger count is malformed.
    if item_count > data.len().saturating_sub(*cursor) {
        return Err(error("BEAM pair list exceeds code chunk"));
    }
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
        TAG_REGISTER => Ok(Operand::X(read_operand(data, cursor, TAG_REGISTER)? as u32)),
        TAG_Y_REGISTER => Ok(Operand::Y(
            read_operand(data, cursor, TAG_Y_REGISTER)? as u32
        )),
        _ => Err(error(format!("unsupported BEAM register tag {tag}"))),
    }
}

/// Read one supported source operand for a `move` instruction.
///
/// # Examples
///
/// ```rust,ignore
/// let literals = vec![Some(Term::Binary(b"ok".to_vec()))];
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
                1 => Ok(Operand::Value(Term::List(Vec::new()))),
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
mod tests {
    use super::super::{read_integer, read_integer_after_first, read_operand, read_value};
    use super::{
        OPCODE_BADMATCH, OPCODE_BIF0, OPCODE_BS_CREATE_BIN, OPCODE_BS_INIT_WRITABLE,
        OPCODE_BS_MATCH, OPCODE_GC_BIF2, OPCODE_GET_RECORD_ELEMENTS, OPCODE_GET_TL,
        OPCODE_INIT_YREGS, OPCODE_MAKE_FUN3, OPCODE_PUT_MAP_ASSOC, OPCODE_PUT_RECORD,
        OPCODE_PUT_TUPLE2, OPCODE_RAW_RAISE, OPCODE_TEST_HEAP, OPCODE_TEST_IS_ATOM,
        OPCODE_TEST_IS_BINARY, OPCODE_TEST_IS_LIST, OPCODE_TEST_IS_MAP, OPCODE_TEST_IS_PID,
        OPCODE_TEST_IS_PORT, OPCODE_TEST_LT, OPCODE_TEST_TAGGED_TUPLE, OPCODE_TRY_CASE_END,
        parse_code, read_move_operand, read_operand_pairs, read_pair_count, read_record_updates,
        skip_counted_operands, skip_integer_payload, skip_operand, skip_unsupported_instruction,
        test_kind, unsupported_opcode_operand_count,
    };
    use crate::{
        beam::code::{Instruction, Operand, TestKind},
        term::{Atom, Term},
    };

    #[test]
    fn reads_operand_pairs_and_rejects_other_lists() {
        let atoms = vec![Atom::from_static("output").unwrap()];
        // {list,[{atom,output},{x,1}]}
        let pairs = read_operand_pairs(&[0x17, 0x20, 0x12, 0x13], &mut 0, &atoms, &[]).unwrap();
        assert!(matches!(
            pairs.as_slice(),
            [(Operand::Value(Term::Atom(key)), Operand::X(1))] if key.name() == "output"
        ));
        assert!(read_operand_pairs(&[0x27, 0x20], &mut 0, &atoms, &[]).is_err());
        assert!(read_operand_pairs(&[0x17, 0x10, 0x12], &mut 0, &atoms, &[]).is_err());
    }

    #[test]
    fn rejects_invalid_operands_and_code_headers() {
        let mut cursor = 0;
        assert!(read_integer(&[0x12], &mut cursor, 1).is_err());
        assert!(read_integer_after_first(&[], &mut 0, 0x19).is_err());
        assert!(read_integer_after_first(&[], &mut 0, 0x09).is_err());
        assert!(read_operand(&[], &mut 0, 0).is_err());
        assert!(read_value(&[0x03], &mut 0, &[]).is_err());
        assert!(read_value(&[0x12], &mut 0, &[]).is_err());
        assert!(parse_code(&[], &[], &[], &[]).is_err());
        assert!(parse_code(&[0; 20], &[], &[], &[]).is_err());
        assert!(
            parse_code(
                &[0, 0, 0, 16, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                &[],
                &[],
                &[],
            )
            .is_err()
        );
    }

    #[test]
    fn sign_extends_large_integer_operands_as_erts() {
        let integer = |bytes: &[u8]| read_value(bytes, &mut 0, &[]).unwrap();
        // erlc 29.1.1 writes {integer,-1} as [0x19, 0xFF, 0xFF].
        assert_eq!(integer(&[0x19, 0xFF, 0xFF]), Term::Integer(-1));
        // {integer,-101714049538469044}: size code 6 gives eight bytes.
        assert_eq!(
            integer(&[0xD9, 0xFE, 0x96, 0xA3, 0x9C, 0x60, 0x97, 0xD3, 0x4C]),
            Term::Integer(-101714049538469044)
        );
        // A leading zero byte keeps a large payload positive.
        assert_eq!(integer(&[0x39, 0x00, 0xFF, 0xFF]), Term::Integer(65535));
        // The short and extended forms are never negative.
        assert_eq!(integer(&[0xF1]), Term::Integer(15));
        assert_eq!(integer(&[0xE9, 0xFF]), Term::Integer(2047));
    }

    #[test]
    fn reads_only_supported_move_literals() {
        let literals = vec![Some(Term::Binary(b"ok".to_vec())), None];
        assert!(matches!(
            &read_move_operand(&[0x47, 0x00], &mut 0, &[], &literals).unwrap(),
            Operand::Value(Term::Binary(value)) if value == b"ok"
        ));
        let mut cursor = 0;
        assert!(matches!(
            read_move_operand(&[0x57, 0x03, 0x50], &mut cursor, &[], &literals).unwrap(),
            Operand::X(0)
        ));
        assert_eq!(cursor, 3);
        assert!(matches!(
            read_move_operand(&[0x47, 0x10], &mut 0, &[], &literals).unwrap(),
            Operand::UnsupportedLiteral(1)
        ));
        assert!(matches!(
            &read_move_operand(&[0x17, 0x10], &mut 0, &[], &literals).unwrap(),
            Operand::Value(Term::List(values)) if values.is_empty()
        ));
        assert!(read_move_operand(&[0x47, 0x20], &mut 0, &[], &literals).is_err());
        assert!(read_move_operand(&[0x27, 0x00], &mut 0, &[], &literals).is_err());
        assert!(read_move_operand(&[0x06], &mut 0, &[], &literals).is_err());
        assert_eq!(
            read_value(&[0x02], &mut 0, &[]).unwrap(),
            Term::List(Vec::new())
        );
    }

    #[test]
    fn rejects_unsupported_select_val_forms() {
        let code = |instructions: &[u8]| {
            let mut code = vec![0, 0, 0, 16];
            code.extend([0; 16]);
            code.extend_from_slice(instructions);
            code
        };

        assert!(parse_code(&code(&[59, 3, 21, 39]), &[], &[], &[]).is_err());
        assert!(parse_code(&code(&[59, 3, 21, 23, 16]), &[], &[], &[]).is_err());
        // A register as a selection value, with two items present.
        assert!(
            parse_code(&code(&[59, 3, 21, 23, 32, 3, 21]), &[], &[], &[])
                .unwrap_err()
                .to_string()
                .contains("unsupported BEAM selection value")
        );
        // A count larger than the remaining bytes fails before any allocation.
        for opcode in [59, 60] {
            assert!(
                parse_code(&code(&[opcode, 3, 21, 23, 32, 3]), &[], &[], &[])
                    .unwrap_err()
                    .to_string()
                    .contains("exceeds code chunk")
            );
        }
    }

    #[test]
    fn decodes_try_and_catch_instructions_with_a_y_register() {
        let code = |instructions: &[u8]| {
            let mut code = vec![0, 0, 0, 16];
            code.extend([0; 16]);
            code.extend_from_slice(instructions);
            code.push(0x03);
            code
        };
        let (instructions, _) = parse_code(
            &code(&[
                0x68, 0x04, 0x75, // try y0 f7
                0x68, 0x14, 0x0D, 0x11, // try y1 f17
                0x3E, 0x04, 0xA5, // catch y0 f10
                0x69, 0x04, // try_end y0
                0x6A, 0x04, // try_case y0
                0x3F, 0x04, // catch_end y0
            ]),
            &[],
            &[],
            &[],
        )
        .unwrap();
        assert!(matches!(
            instructions.as_slice(),
            [
                Instruction::Catch {
                    register: 0,
                    label: 7
                },
                Instruction::Catch {
                    register: 1,
                    label: 17
                },
                Instruction::Catch {
                    register: 0,
                    label: 10
                },
                Instruction::TryEnd { register: 0 },
                Instruction::TryCase { register: 0 },
                Instruction::CatchEnd { register: 0 },
                Instruction::End,
            ]
        ));
        // An X register cannot hold a catch tag.
        assert!(
            parse_code(&code(&[0x68, 0x03, 0x75]), &[], &[], &[])
                .unwrap_err()
                .to_string()
                .contains("unexpected BEAM operand tag 3")
        );
    }

    #[test]
    fn decodes_bif0_and_bif1_into_one_bif_instruction_with_its_fail_label() {
        let code = |instructions: &[u8]| {
            let mut code = vec![0, 0, 0, 16];
            code.extend([0; 16]);
            code.extend_from_slice(instructions);
            code.push(0x03);
            code
        };
        let imports = vec![
            ("erlang".to_owned(), "node".to_owned(), 1),
            ("erlang".to_owned(), "node".to_owned(), 0),
        ];
        // The bytes of `io:io_request/2` in the pinned `io.beam`, with the
        // import indexes 14 and 15 changed to 0 and 1.
        let (instructions, _) = parse_code(
            &code(&[
                0x0A, 0x05, 0x00, 0x57, 0x03, 0x40, 0x23, // bif1 f0 node/1 {tr,x0,pid} x2
                0x09, 0x10, 0x53, // bif0 node/0 x5
                0x0A, 0x35, 0x00, 0x03, 0x13, // bif1 f3 node/1 x0 x1
            ]),
            &[],
            &imports,
            &[],
        )
        .unwrap();
        assert!(matches!(
            instructions.as_slice(),
            [
                Instruction::Bif {
                    fail_label: 0,
                    module: node1_module,
                    name: node1,
                    arguments: node1_arguments,
                    destination: Operand::X(2),
                },
                Instruction::Bif {
                    fail_label: 0,
                    module: node0_module,
                    name: node0,
                    arguments: node0_arguments,
                    destination: Operand::X(5),
                },
                Instruction::Bif {
                    fail_label: 3,
                    module: guard_module,
                    name: guard,
                    arguments: guard_arguments,
                    destination: Operand::X(1),
                },
                Instruction::End,
            ] if node1_module == "erlang"
                && node1 == "node"
                && matches!(node1_arguments.as_slice(), [Operand::X(0)])
                && node0_module == "erlang"
                && node0 == "node"
                && node0_arguments.is_empty()
                && guard_module == "erlang"
                && guard == "node"
                && matches!(guard_arguments.as_slice(), [Operand::X(0)])
        ));
        for bif in [&[0x09, 0x20, 0x03][..], &[0x0A, 0x05, 0x20, 0x03, 0x03]] {
            assert!(
                parse_code(&code(bif), &[], &imports, &[])
                    .unwrap_err()
                    .to_string()
                    .contains("BEAM BIF import index 2 out of range")
            );
        }
    }

    #[test]
    fn decodes_bif2_into_a_bif_instruction_with_its_fail_label() {
        let mut code = vec![0, 0, 0, 16];
        code.extend([0; 16]);
        // Bytes of the pinned OTP 29.1.1 `gen_statem.beam` and `gen.beam`,
        // with the import indexes 33 and 8 changed to 0 and 1.
        code.extend([
            // gen_statem STACKTRACE(): {bif,element,{f,0},[{integer,2},{x,0}],{x,0}}
            0x0B, 0x05, 0x00, 0x21, 0x03, 0x03,
            // gen:do_call: {bif,element,{f,46},[{integer,2},{x,0}],{x,4}}
            0x0B, 0x0D, 0x2E, 0x10, 0x21, 0x03, 0x43, 0x03,
        ]);
        let imports = vec![
            ("erlang".to_owned(), "element".to_owned(), 2),
            ("erlang".to_owned(), "element".to_owned(), 2),
        ];
        let (instructions, _) = parse_code(&code, &[], &imports, &[]).unwrap();
        assert!(matches!(
            instructions.as_slice(),
            [
                Instruction::Bif {
                    fail_label: 0,
                    module,
                    name,
                    arguments,
                    destination: Operand::X(0),
                },
                Instruction::Bif {
                    fail_label: 46,
                    module: call_module,
                    name: call_name,
                    arguments: call_arguments,
                    destination: Operand::X(4),
                },
                Instruction::End,
            ] if module == "erlang"
                && name == "element"
                && matches!(
                    arguments.as_slice(),
                    [Operand::Value(Term::Integer(2)), Operand::X(0)]
                )
                && call_module == "erlang"
                && call_name == "element"
                && matches!(
                    call_arguments.as_slice(),
                    [Operand::Value(Term::Integer(2)), Operand::X(0)]
                )
        ));
    }

    #[test]
    fn decodes_is_ge_with_a_typed_register() {
        let mut code = vec![0, 0, 0, 16];
        code.extend([0; 16]);
        // Bytes of `group_probe:kind/1` in `fixtures/group/group_probe.beam`:
        // {test,is_ge,{f,5},[{tr,{x,1},{t_integer,{0,16777215}}},{integer,2}]}
        code.extend([0x28, 0x55, 0x57, 0x13, 0x10, 0x21, 0x03]);
        let (instructions, _) = parse_code(&code, &[], &[], &[]).unwrap();
        assert!(matches!(
            instructions.as_slice(),
            [
                Instruction::TestGreaterEqual {
                    fail_label: 5,
                    left: Operand::X(1),
                    right: Operand::Value(Term::Integer(2)),
                },
                Instruction::End,
            ]
        ));
    }

    #[test]
    fn decodes_the_list_record_and_receive_wait_instructions_of_pinned_gen_statem() {
        let mut code = vec![0, 0, 0, 16];
        code.extend([0; 16]);
        // Bytes of the pinned OTP 29.1.1 `gen_statem.beam`.
        code.extend([
            0x38, 0x0D, 0x75, 0x03, // replies/1: {test,is_nonempty_list,{f,117},[{x,0}]}
            0x41, 0x03, 0x13, 0x23, // replies/1: {get_list,{x,0},{x,1},{x,2}}
            0x34, 0x0D, 0x73, 0x03, // replies/1: {test,is_nil,{f,115},[{x,0}]}
            0xA2, 0x53, 0x73, // system_code_change/4: {get_hd,{x,5},{x,7}}
            // enter/8: {update_record,{atom,copy},7,{tr,{y,9},_},{x,3},{list,[5,{tr,{y,2},_}]}}
            0xB5, 0x0A, 0x31, 0x70, 0x57, 0x94, 0x10, 0x33, 0x17, 0x20, 0x50, 0x57, 0x24, 0x30,
            // loop_t0q/5: {update_record,{atom,inplace},6,{tr,{x,2},_},{x,2},{list,[4,{x,3},5,nil]}}
            0xB5, 0x0A, 0x9F, 0x60, 0x57, 0x23, 0x10, 0x23, 0x17, 0x40, 0x40, 0x33, 0x50, 0x02,
            0x1A, 0x0D, 0xCC, 0x04, // loop/3: {wait_timeout,{f,204},{y,0}}
            0xA1, // call/3: raw_raise
            0x03,
        ]);
        let (instructions, _) = parse_code(&code, &[], &[], &[]).unwrap();
        assert!(matches!(
            instructions.as_slice(),
            [
                Instruction::Test {
                    kind: TestKind::NonEmptyList,
                    fail_label: 117,
                    value: Operand::X(0),
                },
                Instruction::GetList {
                    source: Operand::X(0),
                    head: Operand::X(1),
                    tail: Operand::X(2),
                },
                Instruction::Test {
                    kind: TestKind::Nil,
                    fail_label: 115,
                    value: Operand::X(0),
                },
                Instruction::GetHead {
                    source: Operand::X(5),
                    destination: Operand::X(7),
                },
                Instruction::UpdateRecord {
                    source: Operand::Y(9),
                    destination: Operand::X(3),
                    updates: copy,
                },
                Instruction::UpdateRecord {
                    source: Operand::X(2),
                    destination: Operand::X(2),
                    updates: inplace,
                },
                Instruction::WaitTimeout {
                    label: 204,
                    source: Operand::Y(0),
                },
                Instruction::Unsupported(OPCODE_RAW_RAISE),
                Instruction::End,
            ] if matches!(copy.as_slice(), [(5, Operand::Y(2))])
                && matches!(
                    inplace.as_slice(),
                    [(4, Operand::X(3)), (5, Operand::Value(Term::List(nil)))] if nil.is_empty()
                )
        ));
    }

    #[test]
    fn decodes_the_apply_instructions_of_pinned_gen_statem() {
        let mut code = vec![0, 0, 0, 16];
        code.extend([0; 16]);
        // Bytes of the pinned OTP 29.1.1 `gen_statem.beam`.
        code.extend([
            0x70, 0x00, // get_callback_mode/2: {apply,0}
            0x70, 0x30, // terminate/7: {apply,3}
            0x71, 0x10, 0xB0, // terminate/7: {apply_last,1,11}
            0x03,
        ]);
        let (instructions, _) = parse_code(&code, &[], &[], &[]).unwrap();
        assert!(matches!(
            instructions.as_slice(),
            [
                Instruction::Apply {
                    arity: 0,
                    tail: false,
                    deallocate: 0,
                },
                Instruction::Apply {
                    arity: 3,
                    tail: false,
                    deallocate: 0,
                },
                Instruction::Apply {
                    arity: 1,
                    tail: true,
                    deallocate: 11,
                },
                Instruction::End,
            ]
        ));
        // `apply_last` without its deallocate operand is truncated.
        code.truncate(code.len() - 2);
        assert!(parse_code(&code, &[], &[], &[]).is_err());
    }

    #[test]
    fn rejects_malformed_record_update_lists() {
        let error = |bytes: &[u8]| {
            read_record_updates(bytes, &mut 0, &[], &[])
                .unwrap_err()
                .to_string()
        };
        // The shared header reader rejects a list that is not a pair list.
        assert!(error(&[0x17, 0x10, 0x50]).contains("incomplete pair"));
        // A position must be an unsigned operand.
        assert!(error(&[0x17, 0x20, 0x03, 0x03]).contains("unexpected BEAM operand tag 3"));
    }

    #[test]
    fn rejects_a_put_tuple2_without_an_element_list() {
        let mut code = vec![0, 0, 0, 16];
        code.extend([0; 16]);
        code.extend([OPCODE_PUT_TUPLE2, 0x03, 0x27]);
        assert!(parse_code(&code, &[], &[], &[]).is_err());
    }

    #[test]
    fn maps_supported_type_tests_and_rejects_unknown_opcodes() {
        assert!(test_kind(OPCODE_TEST_IS_ATOM).is_ok());
        assert!(test_kind(OPCODE_TEST_IS_PID).is_ok());
        assert!(test_kind(OPCODE_TEST_IS_LIST).is_ok());
        assert!(test_kind(OPCODE_TEST_IS_BINARY).is_ok());
        assert!(test_kind(255).is_err());
    }

    #[test]
    fn reads_pair_list_headers_and_rejects_malformed_forms() {
        let mut cursor = 0;
        let list = [0x17, 0x40, 0x10, 0x15, 0x20, 0x25];
        assert_eq!(read_pair_count(&list, &mut cursor).unwrap(), 2);
        assert_eq!(cursor, 2);
        assert_eq!(read_pair_count(&[0x17, 0x00], &mut 0).unwrap(), 0);
        assert!(read_pair_count(&[], &mut 0).is_err());
        assert!(read_pair_count(&[0x17], &mut 0).is_err());
        assert!(read_pair_count(&[0x27, 0x00], &mut 0).is_err());
        assert!(read_pair_count(&[0x17, 0x10, 0x00], &mut 0).is_err());
        // Eight items with no bytes left: rejected before any allocation.
        assert!(
            read_pair_count(&[0x17, 0x80], &mut 0)
                .unwrap_err()
                .to_string()
                .contains("exceeds code chunk")
        );
    }

    #[test]
    fn skips_pinned_module_instruction_operands_and_rejects_truncation() {
        for (opcode, count) in [
            (OPCODE_TEST_HEAP, 2),
            (OPCODE_BIF0, 2),
            (OPCODE_GC_BIF2, 6),
            (OPCODE_TEST_LT, 3),
            (OPCODE_TEST_TAGGED_TUPLE, 4),
            (OPCODE_TEST_IS_MAP, 2),
            (OPCODE_GET_TL, 2),
            (OPCODE_BADMATCH, 1),
            (OPCODE_TEST_IS_PORT, 2),
            (OPCODE_TRY_CASE_END, 1),
            (OPCODE_PUT_MAP_ASSOC, 5),
            (OPCODE_GET_RECORD_ELEMENTS, 3),
            (OPCODE_PUT_RECORD, 6),
            (OPCODE_PUT_TUPLE2, 2),
            (OPCODE_INIT_YREGS, 1),
            (OPCODE_MAKE_FUN3, 3),
            (OPCODE_BS_CREATE_BIN, 6),
            (OPCODE_BS_INIT_WRITABLE, 0),
            (OPCODE_BS_MATCH, 3),
            (OPCODE_RAW_RAISE, 0),
        ] {
            assert_eq!(unsupported_opcode_operand_count(opcode), Some(count));
        }
        assert_eq!(unsupported_opcode_operand_count(255), None);

        let mut cursor = 0;
        assert!(skip_unsupported_instruction(255, &[], &mut cursor).is_err());
        assert!(skip_unsupported_instruction(OPCODE_GC_BIF2, &[0], &mut cursor).is_err());
        assert!(skip_counted_operands(&[0x10], &mut 0).is_err());
        assert!(skip_counted_operands(&[0x20], &mut 0).is_err());
    }

    #[test]
    fn skips_extended_operands_and_reports_invalid_or_truncated_forms() {
        for (operand, expected_cursor) in [
            (vec![0x07, 0, 0, 0, 0, 0, 0, 0, 0], 9),
            (vec![0x17], 1),
            (vec![0x27, 0x10], 2),
            (vec![0x37, 0x10, 0x10, 0x10], 4),
            (vec![0x47, 0x10], 2),
        ] {
            let mut cursor = 0;
            skip_operand(&operand, &mut cursor).unwrap();
            assert_eq!(cursor, expected_cursor);
        }
        assert!(skip_operand(&[0x07], &mut 0).is_err());
        assert!(skip_operand(&[0x37, 0x10], &mut 0).is_err());
        assert!(skip_operand(&[0x57], &mut 0).is_err());
        assert!(skip_operand(&[0x87], &mut 0).is_err());
        assert!(skip_operand(&[0x0f], &mut 0).is_err());

        let mut cursor = 0;
        assert!(skip_integer_payload(&[0], &mut cursor, 0xf8).is_err());
        assert!(skip_integer_payload(&[0, 0], &mut 0, 0x19).is_ok());
        assert!(read_integer_after_first(&[0], &mut 0, 0xf9).is_err());
    }
}
