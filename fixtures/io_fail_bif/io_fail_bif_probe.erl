-module(io_fail_bif_probe).
-export([unicode_io/0, done/0]).

%% net_kernel:dflag_unicode_io/1, as io:bc_req/3 (io.erl:1734-1735) runs it
%% for a pid of an io:format/3 call.
unicode_io() ->
    Dead = spawn(?MODULE, done, []),
    Ref = monitor(process, Dead),
    receive {'DOWN', Ref, process, Dead, _} -> ok end,
    [try net_kernel:dflag_unicode_io(T) catch error:R -> {error, R} end
     || T <- [self(), Dead, foo, make_ref()]].

done() -> ok.
