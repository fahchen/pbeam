-module(bs_bignum_probe).
-export([put128/1, put100/1, put75/1, put_sized/2, put_little/2, put_odd/2,
         get128/1, get100/1, get_wide/1, get_wide_signed/1, get_wide_sized/2, get_sized/2, get_little/2, get_odd/1]).

%% bs_create_bin integer segments of more than 64 bits. A pad segment
%% makes each result a binary.
put128(X) -> <<X:128>>.
put100(X) -> <<X:100/little-signed, 0:4>>.
put75(X) -> <<X:75, 0:5>>.
put_sized(X, N) -> <<X:N>>.
put_little(X, N) -> <<X:N/little>>.
put_odd(A, X) -> <<A:3, X:70/little, 0:7>>.

%% bs_match integer commands of more than 64 bits.
get128(<<X:128>>) -> X.
get100(<<X:100/little-signed, _/bits>>) -> X.
get_odd(<<_:3, X:70/little-signed, _/bits>>) -> X.

%% bs_get_integer2 with the size in a register.
get_sized(B, N) ->
    <<X:N/signed, _/bits>> = B,
    X.
get_little(B, N) ->
    <<X:N/little, _/bits>> = B,
    X.

%% A read of more than BIG_ARITY_MAX digits fails the match; a value with
%% fewer digits matches.
get_wide(<<X:4194304>>) -> X band 1;
get_wide(_) -> nomatch.
get_wide_signed(<<X:4194304/signed>>) -> X band 1;
get_wide_signed(_) -> nomatch.
get_wide_sized(B, N) ->
    case B of
        <<X:N, _/bits>> -> X band 1;
        _ -> nomatch
    end.
