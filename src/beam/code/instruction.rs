//! Parse Code chunk instructions into the runtime instruction model.

use super::{
    COMPACT_EXTENDED_FLAG, COMPACT_SHORT_VALUE_SHIFT, COMPACT_SIGN_FLAG, COMPACT_TAG_MASK,
    FIRST_LABEL_NUMBER, LIST_SUBTAG, TAG_ATOM, TAG_INTEGER, TAG_LABEL, TAG_LITERAL, TAG_REGISTER,
    TAG_UNSIGNED, TAG_Y_REGISTER, WORD_SIZE, atom_at, read_integer, read_operand, read_u32,
    read_value,
};
use crate::error::{BeamError, error};
use crate::term::{Atom, Lambda, Term};
use std::num::NonZeroU64;

const CODE_HEADER_SIZE: usize = WORD_SIZE * 5;
/// The first byte of an integer operand with 9 or more payload bytes: size
/// code 7, the sign and extended flags, and `TAG_INTEGER`. Its value is
/// outside the 64-bit signed range, so ERTS loads it as a bignum.
const HUGE_INTEGER_FIRST_BYTE: u8 =
    0b1110_0000 | COMPACT_SIGN_FLAG | COMPACT_EXTENDED_FLAG | TAG_INTEGER;
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
const OPCODE_BS_GET_BINARY2: u8 = 119;
const OPCODE_BS_SKIP_BITS2: u8 = 120;
/// `bs_get_integer2 Fail Ctx Live Size Unit Flags Dst` (genop.tab:447).
const OPCODE_BS_GET_INTEGER2: u8 = 117;
/// `gc_bif3 Fail Live Bif Arg1 Arg2 Arg3 Dst` (genop.tab:533).
const OPCODE_GC_BIF3: u8 = 152;
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
const OPCODE_GC_BIF1: u8 = 124;
const OPCODE_GC_BIF2: u8 = 125;
const OPCODE_BS_GET_TAIL: u8 = 165;
/// `bs_start_match3 Fail Bin Live Dst` (genop.tab:597).
const OPCODE_BS_START_MATCH3: u8 = 166;
/// `bs_get_position Ctx Dst Live` (genop.tab:599-601).
const OPCODE_BS_GET_POSITION: u8 = 167;
/// `bs_set_position Ctx Pos` (genop.tab:603-605).
const OPCODE_BS_SET_POSITION: u8 = 168;
const OPCODE_BS_START_MATCH4: u8 = 170;
/// `bs_get_utf8 Fail Ctx Live Flags Dst` (genop.tab:501).
const OPCODE_BS_GET_UTF8: u8 = 138;
const OPCODE_BS_SKIP_UTF8: u8 = 139;
const OPCODE_BS_GET_UTF16: u8 = 140;
const OPCODE_BS_GET_UTF32: u8 = 142;
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
/// `bs_match Fail Ctx {commands,Commands}` (genop.tab:685-696).
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
    /// (opcode 11), `gc_bif1` (opcode 124), or `gc_bif2` (opcode 125) and
    /// writes its value to a register. `bif0` has no fail label and decodes with `fail_label` 0.
    /// With fail label 0, a BIF failure raises its exception; with another
    /// fail label, it jumps to the label.
    ///
    /// Encoding: `bif0` has opcode, import index, destination register;
    /// `bif1` and `bif2` have opcode, fail label, import index, one or two
    /// arguments, destination register. `gc_bif1` and `gc_bif2` have
    /// opcode, fail label, live register count, import index, one or two
    /// arguments, destination register. The live count tells the ERTS garbage collector which X
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
    /// `proc_lib:trans_init/3` (proc_lib.erl:944) compiles to
    /// `{gc_bif,length,{f,0},3,[{tr,{x,2},...}],{x,2}}`.
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
        lambda: Lambda,
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
    /// Gives the stack trace of the exception in `x0` as a list. The
    /// compiler moves the stack trace from `x2` to `x0` first. PBeam keeps
    /// the stack trace as a list, so `x0` does not change.
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
    /// Jumps to `fail_label` when the operands are not equal in the ERTS
    /// arithmetic order (`is_eq`, opcode 41). PBeam has no float term, so the
    /// test gives the same result as exact equality.
    ///
    /// Encoding: opcode, fail label, two operands. A typed register (extended
    /// subtag 5) reads as its register. Example: `compare_probe:eq/2` in
    /// `fixtures/compare/compare_probe.beam` has `[0x29, 0x0D, 0x19, 0x03,
    /// 0x13]`, which fails to label 25 unless `x0` equals `x1`.
    ///
    /// Erlang: the guard `CtrlSync==0` of `logger_h_common:handle_load/2`
    /// (logger_h_common.erl:250) compiles to an `is_eq` test.
    /// `eq(A, B) when A == B -> id(true);` compiles to
    /// `{test,is_eq,{f,25},[{x,0},{x,1}]}`.
    TestEqual {
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
    /// Curr` of `user_drv:server/3` (user_drv.erl:542) compiles to
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
    /// `io_lib_format:field_width/3` (io_lib_format.erl:259) does, which
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
    BinaryMatchStart {
        fail_label: u32,
        source: Operand,
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
    /// Reads one UTF-8 character from the match context in `context` and
    /// writes its code point to `destination`, or jumps to `fail_label`
    /// (`bs_get_utf8`, opcode 138).
    ///
    /// Encoding: opcode, fail label, context, unsigned live count, unsigned
    /// flags, destination register. The decoder skips the live count and
    /// the flags, as the ERTS loader does (emu `ops.tab:1286`). Example:
    /// `binary_match_probe:is_usascii/1` has `[0x8A, 0x0D, 0x10, 0x57,
    /// 0x03, 0x10, 0x20, 0x00, 0x23]`, which reads a character from the
    /// typed register `x0` to `x2` or fails to label 16.
    ///
    /// Erlang: `<<Char/utf8, T/binary>>` compiles to
    /// `{test,bs_get_utf8,{f,16},[{tr,{x,0},...},2,{field_flags,[]},{x,2}]}`.
    BinaryGetUtf8 {
        fail_label: u32,
        context: Operand,
        destination: Operand,
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
    },
    /// Converts `source` to a float in a float register (`fconv`, opcode
    /// 97). ERTS raises `error:badarith` for a term that is not a number.
    /// PBeam has no float register, so an integer source returns the runtime
    /// error `unsupported BEAM opcode 97`.
    ///
    /// Encoding: opcode, source, float register (extended subtag 2). The
    /// decoder skips the float register. Example: `float_test_probe:half/1`
    /// in `fixtures/float_test/float_test_probe.beam` has `[0x61, 0x03, 0x27,
    /// 0x00]`, which converts `x0` to `fr0`.
    ///
    /// Erlang: `half(X) -> X / 2.` compiles to `{fconv,{x,0},{fr,0}}`, as
    /// the `/` operators of `string` and `erl_scan` do.
    FloatConvert { source: Operand },
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
    /// `is_function`, opcode 77. Example: `[0x4D, 0x0D, 0x65, 0x57, 0x03,
    /// 0x40]` in `fun_probe:kind/1` fails to label 101 unless the typed
    /// register `x0` holds a fun. Erlang: `kind(F) when is_function(F) ->
    /// any;` compiles to `{test,is_function,{f,101},[{tr,{x,0},...}]}`.
    Function,
    /// `is_integer`, opcode 45. Example: `[0x2D, 0xB5, 0x03]` in
    /// `compare_probe:kind/1` fails to label 11 unless `x0` is an integer.
    /// Erlang: `kind(X) when is_integer(X) -> integer;` compiles to
    /// `{test,is_integer,{f,11},[{x,0}]}`, as the guard of
    /// `io_lib_format:field_value/2` (io_lib_format.erl:271) does.
    Integer,
    /// `is_float`, opcode 46. Example: `[0x2E, 0x55, 0x03]` in
    /// `float_test_probe:kind/1` fails to label 5 unless `x0` is a float.
    /// Erlang: `kind(Term) when is_float(Term) -> float;` compiles to
    /// `{test,is_float,{f,5},[{x,0}]}`, as `lists:thing_to_list/1`
    /// (lists.erl:1283-1286) does. PBeam has no float term.
    Float,
    /// `is_boolean`, opcode 114. Example: `[0x72, 0xC5, 0x03]` in
    /// `compare_probe:kind/1` fails to label 12 unless `x0` is the atom
    /// `true` or `false`. Erlang: `kind(X) when is_boolean(X) -> boolean;`
    /// compiles to `{test,is_boolean,{f,12},[{x,0}]}`, as the guard of
    /// `standard_error:check_valid_opts/1` (standard_error.erl:236) does.
    Boolean,
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
    /// A command that PBeam does not run yet: `binary`, `integer`, `skip`,
    /// `get_tail`, or `=:=`. It holds the command name. Example: `[0x0A,
    /// 0x13, 0x10, 0x02, 0x80, 0x10, 0x13]` is `{integer,1,[],8,1,{x,1}}`
    /// when atom 19 is `integer`.
    Unsupported(Atom),
}

/// A source or destination in the supported `move` instruction.
///
/// `Value` holds a supported immediate term. `X` uses compact register tag 3,
/// and `Y` uses compact register tag 4. For example, `move x0, y0` is encoded
/// as `[0x40, 0x03, 0x04]`. `UnsupportedInteger` is an integer operand with 9
/// or more payload bytes, such as `{integer,18446744073709551616}`, encoded
/// as `[0xF9, 0x00, 0x01, 0, 0, 0, 0, 0, 0, 0, 0]`. ERTS loads it as a
/// bignum literal. PBeam has no bignum term, so the module loads and the
/// instruction fails only when it reads the operand.
#[derive(Debug, Clone)]
pub(crate) enum Operand {
    Value(Term),
    UnsupportedLiteral(usize),
    UnsupportedInteger,
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
/// let (instructions, labels) = parse_code(&code, &[], &[], &[], &[])?;
/// assert_eq!(labels, 0);
/// assert!(matches!(instructions.as_slice(), [Instruction::End]));
/// ```
pub(super) fn parse_code(
    data: &[u8],
    atoms: &[Atom],
    imports: &[(String, String, u32)],
    literals: &[Option<Term>],
    lambdas: &[(Lambda, usize)],
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
    let mut function = None;
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
                // The label directly after `func_info` is the function entry.
                if let Some((_, _, entry)) = &mut function
                    && *entry == instructions.len()
                {
                    *entry += 1;
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
            OPCODE_FCONV => {
                let source = read_move_operand(data, &mut cursor, atoms, literals)?;
                skip_operands(data, &mut cursor, 1)?;
                Instruction::FloatConvert { source }
            }
            OPCODE_BS_START_MATCH3 => {
                let fail_label = read_operand(data, &mut cursor, TAG_LABEL)? as u32;
                let source = read_move_operand(data, &mut cursor, atoms, literals)?;
                skip_live_registers(data, &mut cursor)?;
                Instruction::BinaryMatchStart {
                    fail_label,
                    source,
                    destination: read_register(data, &mut cursor)?,
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
            OPCODE_BS_GET_UTF8 => {
                let fail_label = read_operand(data, &mut cursor, TAG_LABEL)? as u32;
                let context = read_move_operand(data, &mut cursor, atoms, literals)?;
                skip_live_registers(data, &mut cursor)?;
                let _flags = read_operand(data, &mut cursor, TAG_UNSIGNED)?;
                Instruction::BinaryGetUtf8 {
                    fail_label,
                    context,
                    destination: read_register(data, &mut cursor)?,
                }
            }
            OPCODE_BS_MATCH => Instruction::BinaryMatch {
                fail_label: read_operand(data, &mut cursor, TAG_LABEL)? as u32,
                context: read_move_operand(data, &mut cursor, atoms, literals)?,
                commands: read_binary_match_commands(data, &mut cursor, atoms)?,
            },
            OPCODE_TEST_EQ => Instruction::TestEqual {
                fail_label: read_operand(data, &mut cursor, TAG_LABEL)? as u32,
                left: read_move_operand(data, &mut cursor, atoms, literals)?,
                right: read_move_operand(data, &mut cursor, atoms, literals)?,
            },
            OPCODE_TEST_EXACT_EQ => Instruction::TestExactEqual {
                fail_label: read_operand(data, &mut cursor, TAG_LABEL)? as u32,
                left: read_move_operand(data, &mut cursor, atoms, literals)?,
                right: read_move_operand(data, &mut cursor, atoms, literals)?,
            },
            OPCODE_TEST_EXACT_NE => Instruction::TestExactNotEqual {
                fail_label: read_operand(data, &mut cursor, TAG_LABEL)? as u32,
                left: read_move_operand(data, &mut cursor, atoms, literals)?,
                right: read_move_operand(data, &mut cursor, atoms, literals)?,
            },
            OPCODE_TEST_LT => Instruction::TestLess {
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
            OPCODE_BUILD_STACKTRACE => Instruction::BuildStacktrace,
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
                fail_label: read_operand(data, &mut cursor, TAG_LABEL)? as u32,
                map: read_move_operand(data, &mut cursor, atoms, literals)?,
                pairs: read_operand_pairs(data, &mut cursor, atoms, literals)?,
            },
            OPCODE_HAS_MAP_FIELDS => {
                let fail_label = read_operand(data, &mut cursor, TAG_LABEL)? as u32;
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
                let fail_label = read_operand(data, &mut cursor, TAG_LABEL)? as u32;
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
                    lambda: lambda.clone(),
                    free,
                    destination,
                }
            }
            OPCODE_CALL_FUN => {
                let arity = read_operand(data, &mut cursor, TAG_UNSIGNED)? as u32;
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
                    atom_at(atoms, tag as usize)?;
                }
                let arity = read_operand(data, &mut cursor, TAG_UNSIGNED)? as u32;
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
            | OPCODE_TEST_IS_REFERENCE
            | OPCODE_TEST_IS_TUPLE
            | OPCODE_TEST_IS_LIST
            | OPCODE_TEST_IS_NIL
            | OPCODE_TEST_IS_NONEMPTY_LIST
            | OPCODE_TEST_IS_BINARY
            | OPCODE_TEST_IS_MAP
            | OPCODE_TEST_IS_FUNCTION
            | OPCODE_TEST_IS_INTEGER
            | OPCODE_TEST_IS_FLOAT
            | OPCODE_TEST_IS_BOOLEAN => {
                let fail_label = read_operand(data, &mut cursor, TAG_LABEL)? as u32;
                let value = read_move_operand(data, &mut cursor, atoms, literals)?;
                let kind = test_kind(opcode)?;
                Instruction::Test {
                    kind,
                    fail_label,
                    value,
                }
            }
            OPCODE_TEST_IS_FUNCTION2 => Instruction::TestFunctionArity {
                fail_label: read_operand(data, &mut cursor, TAG_LABEL)? as u32,
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
                // `i_hibernate` returns to the next instruction, so a tail
                // call hibernates first and returns after that
                // (`jit/arm/ops.tab:669-673`; `emu/ops.tab:896-898`).
                if (module.as_str(), name.as_str(), arity) == ("erlang", "hibernate", 0) {
                    instructions.push(Instruction::Hibernate);
                    if opcode == OPCODE_CALL_EXT_LAST {
                        instructions.push(Instruction::Deallocate {
                            stack_size: deallocate,
                        });
                    }
                    if opcode != OPCODE_CALL_EXT {
                        instructions.push(Instruction::Return);
                    }
                    continue;
                } else {
                    Instruction::CallExternal {
                        arity,
                        module: module.clone(),
                        name: name.clone(),
                        tail: opcode != OPCODE_CALL_EXT,
                        deallocate,
                    }
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
            OPCODE_BIF1 | OPCODE_BIF2 | OPCODE_GC_BIF1 | OPCODE_GC_BIF2 => {
                let fail_label = read_operand(data, &mut cursor, TAG_LABEL)? as u32;
                if matches!(opcode, OPCODE_GC_BIF1 | OPCODE_GC_BIF2) {
                    skip_live_registers(data, &mut cursor)?;
                }
                let (module, name) = bif_import(
                    imports,
                    read_operand(data, &mut cursor, TAG_UNSIGNED)? as usize,
                )?;
                let argument_count = if matches!(opcode, OPCODE_BIF1 | OPCODE_GC_BIF1) {
                    1
                } else {
                    2
                };
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
                skip_live_registers(data, &mut cursor)?;
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
        OPCODE_TEST_IS_FUNCTION => Ok(TestKind::Function),
        OPCODE_TEST_IS_INTEGER => Ok(TestKind::Integer),
        OPCODE_TEST_IS_FLOAT => Ok(TestKind::Float),
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

/// Read the command list of `bs_match`: each command is a name atom and
/// its operands (genop.tab:685-696). The command names and operand counts
/// are those of the ERTS loader (`gen.bs_match` in emu
/// `generators.tab:1102-1430`), which fails to load a module with another
/// command name.
///
/// # Examples
///
/// ```rust,ignore
/// // {list,[ensure_at_least,0,8]} with atom 8 `ensure_at_least`
/// let commands = read_binary_match_commands(&[0x17, 0x30, 0x82, 0x00, 0x80], &mut 0, &atoms)?;
/// assert!(matches!(commands.as_slice(), [BinaryMatchCommand::EnsureAtLeast { size: 0, .. }]));
/// ```
fn read_binary_match_commands(
    data: &[u8],
    cursor: &mut usize,
    atoms: &[Atom],
) -> Result<Vec<BinaryMatchCommand>, BeamError> {
    let item_count = read_list_count(data, cursor)?;
    let mut commands = Vec::new();
    let mut items = 0;
    while items < item_count {
        let name = atom_at(atoms, read_operand(data, cursor, TAG_ATOM)? as usize)?;
        let (command, operand_count) = match name.name() {
            "ensure_at_least" => {
                let size = read_operand(data, cursor, TAG_UNSIGNED)?;
                let unit = NonZeroU64::new(read_operand(data, cursor, TAG_UNSIGNED)?)
                    .ok_or_else(|| error("BEAM bs_match ensure_at_least has unit 0"))?;
                (BinaryMatchCommand::EnsureAtLeast { size, unit }, 2)
            }
            "ensure_exactly" => {
                let size = read_operand(data, cursor, TAG_UNSIGNED)?;
                (BinaryMatchCommand::EnsureExactly { size }, 1)
            }
            other => {
                let operand_count = match other {
                    "binary" | "integer" => 5,
                    "get_tail" | "=:=" => 3,
                    "skip" => 1,
                    _ => return Err(error(format!("unsupported BEAM bs_match command {other}"))),
                };
                skip_operands(data, cursor, operand_count)?;
                (BinaryMatchCommand::Unsupported(name.clone()), operand_count)
            }
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
/// skip_unsupported_instruction(OPCODE_BS_GET_UTF16, &[0, 0, 0, 0, 0], &mut cursor)?;
/// assert_eq!(cursor, 5);
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
            | OPCODE_GET_MAP_ELEMENTS
            | OPCODE_GET_RECORD_ELEMENTS
            | OPCODE_PUT_RECORD
            | OPCODE_PUT_TUPLE2
            | OPCODE_INIT_YREGS
            | OPCODE_BS_CREATE_BIN
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
/// assert_eq!(unsupported_opcode_operand_count(OPCODE_BS_GET_UTF16), Some(5));
/// assert_eq!(unsupported_opcode_operand_count(255), None);
/// ```
fn unsupported_opcode_operand_count(opcode: u8) -> Option<usize> {
    match opcode {
        OPCODE_RAISE => Some(2),
        OPCODE_BS_GET_BINARY2 => Some(7),
        OPCODE_BS_SKIP_BITS2 => Some(5),
        // `bs_get_integer2` is in `io_lib`.
        OPCODE_BS_GET_INTEGER2 => Some(7),
        // `gc_bif3` is in `io_lib`, for `erlang:binary_part/3`.
        OPCODE_GC_BIF3 => Some(7),
        // `is_any_native_record` is in `io_lib`.
        OPCODE_IS_ANY_NATIVE_RECORD => Some(2),
        OPCODE_BS_START_MATCH4 => Some(4),
        OPCODE_BS_GET_TAIL => Some(3),
        // `bs_get_utf16` and `bs_get_utf32` are in `unicode`.
        OPCODE_BS_GET_UTF16 | OPCODE_BS_GET_UTF32 => Some(5),
        OPCODE_BS_SKIP_UTF8 => Some(4),
        OPCODE_BADRECORD => Some(1),
        OPCODE_IF_END => Some(0),
        // `raw_raise` is in `gen_statem:call/3`.
        OPCODE_RAW_RAISE => Some(0),
        OPCODE_TEST_HEAP => Some(2),
        OPCODE_BIF0 => Some(2),
        OPCODE_TEST_NE | OPCODE_TEST_EXACT_EQ => Some(3),
        OPCODE_TEST_TAGGED_TUPLE => Some(4),
        OPCODE_TEST_IS_NUMBER
        | OPCODE_TEST_IS_ATOM
        | OPCODE_TEST_IS_PID
        | OPCODE_TEST_IS_BINARY
        | OPCODE_TEST_IS_LIST
        | OPCODE_TEST_IS_MAP
        | OPCODE_TEST_IS_BITSTR => Some(2),
        // `is_port Fail Src` is in `erlang:is_port/1` and `erlang:open_port/2`.
        OPCODE_TEST_IS_PORT => Some(2),
        // `try_case_end Src` is in `erlang:processes_next/1`.
        OPCODE_BADMATCH | OPCODE_CASE_END | OPCODE_TRY_CASE_END => Some(1),
        OPCODE_PUT_MAP_ASSOC => Some(5),
        OPCODE_GET_MAP_ELEMENTS | OPCODE_GET_RECORD_ELEMENTS => Some(3),
        OPCODE_PUT_RECORD => Some(6),
        OPCODE_PUT_TUPLE2 => Some(2),
        OPCODE_INIT_YREGS => Some(1),
        OPCODE_BS_CREATE_BIN => Some(6),
        OPCODE_BS_INIT_WRITABLE => Some(0),
        // The float-register instructions are in `string` and `erl_scan`.
        // PBeam has no float term, so each one stops the run. `fconv`
        // decodes as `Instruction::FloatConvert`.
        OPCODE_FMOVE => Some(2),
        OPCODE_FADD | OPCODE_FSUB | OPCODE_FMUL | OPCODE_FDIV => Some(4),
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
        TAG_INTEGER if first == HUGE_INTEGER_FIRST_BYTE => {
            skip_operand(data, cursor)?;
            Ok(Operand::UnsupportedInteger)
        }
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
mod tests;
