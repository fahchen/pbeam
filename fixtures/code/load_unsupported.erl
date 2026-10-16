-module(load_unsupported).
-export([skip/1]).

%% A module with an instruction that the PBeam loader does not decode:
%% bs_skip_utf16 (opcode 141).
skip(<<_/utf16, Rest/binary>>) -> Rest;
skip(_) -> none.
