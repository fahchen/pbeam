-module(load_unsupported).
-export([skip/1]).
-compile([line_coverage]).

%% A module with an instruction that the PBeam loader does not decode:
%% executable_line (opcode 183). The `line_coverage` option makes the
%% compiler add it.
skip(<<"ab", Rest/binary>>) -> Rest;
skip(_) -> none.
