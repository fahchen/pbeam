-module(crypto_probe).
-export([main/0, info_lib/0]).

%% Write the results of crypto:strong_rand_bytes/1, one term on each line.
%% The first call loads crypto on demand, and its -on_load function loads
%% the NIF library (crypto.erl:4432-4510). The NIF strong_rand_bytes_nif/1
%% gives a binary of the size, <<>> for 0, and badarg for a term that is not
%% an unsigned C int or that is above INT_MAX (rand.c:26-50).
main() ->
    Sizes = [byte_size(crypto:strong_rand_bytes(Size)) || Size <- [0, 1, 16, 1024]],
    Binaries = [is_binary(crypto:strong_rand_bytes(Size)) || Size <- [1, 16]],
    Bad = [bad(Size) || Size <- [-1, 2147483648, 1 bsl 64, 1.0, a, <<1>>]],
    io:format("~p~n", [[erlang:system_info(build_type), crypto:strong_rand_bytes(0),
                        Sizes, Binaries, Bad]]).

%% Call crypto:info_lib/0, whose NIF info_lib/0 PBeam does not link.
info_lib() ->
    io:format("~p~n", [crypto:info_lib()]).

bad(Size) ->
    try crypto:strong_rand_bytes(Size) catch Class:Reason:Stack -> {Class, Reason, hd(Stack)} end.
