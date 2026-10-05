-module(fun_probe).
-export([mfas/0, id/1]).

%% erlang:apply/3 and the apply instruction, as proc_lib:init_p_do_apply/3
%% runs them. apply/3 takes at most 1023 arguments.
mfas() ->
    M = id(fun_probe), F = id(id),
    [apply(fun_probe, id, id([v])),
     apply(erlang, apply, id([fun_probe, id, [w]])),
     M:F(z),
     (id(erlang)):(id(apply))(fun_probe, id, [y]),
     tail_apply(fun_probe, id, [s]),
     try apply(id(1), id, []) catch error:R1 -> R1 end,
     try apply(fun_probe, id(1), []) catch error:R2 -> R2 end,
     try apply(id(1), id, id([])) catch error:R3 -> R3 end,
     try apply(fun_probe, id(1), id([])) catch error:R4 -> R4 end,
     try apply(fun_probe, id, id(notalist)) catch error:R5 -> R5 end,
     try apply(erlang, apply, id([fun_probe, id(1), []])) catch error:R6 -> R6 end,
     try (id(1)):F(z) catch error:R7 -> R7 end,
     try M:(id(1))(z) catch error:R8 -> R8 end,
     try apply(fun_probe, id, [x, x | list1022()]) catch error:R9 -> R9 end].

%% A list of 1022 atoms x, built without arithmetic.
list1022() ->
    L1 = id([x]), L2 = L1 ++ L1, L4 = L2 ++ L2, L8 = L4 ++ L4, L16 = L8 ++ L8,
    L32 = L16 ++ L16, L64 = L32 ++ L32, L128 = L64 ++ L64, L256 = L128 ++ L128,
    L512 = L256 ++ L256,
    L512 ++ L256 ++ L128 ++ L64 ++ L32 ++ L16 ++ L8 ++ L4 ++ L2.

tail_apply(M, F, A) -> apply(M, F, A).
id(X) -> X.
