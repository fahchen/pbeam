%% The results of `erlang:decode_packet/3` for the packet types `http_bin`
%% and `httph_bin`, for a comparison with the pinned ERTS.
-module(decode_packet_probe).

-export([request_lines/0, header_lines/0, end_of_headers/0, incomplete/0,
         malformed/0, options/0, badargs/0, unsupported/0]).

decode(Type, Bin) ->
    erlang:decode_packet(Type, Bin, []).

request_lines() ->
    [decode(http_bin, <<"GET / HTTP/1.1\r\n">>),
     decode(http_bin, <<"post /x HTTP/2.0\r\n">>),
     decode(http_bin, <<"FOO / HTTP/1.1\r\n">>),
     decode(http_bin, <<"GET /\r\n">>),
     decode(http_bin, <<"OPTIONS * HTTP/1.1\r\n">>),
     decode(http_bin, <<"GET http://example.com:8080/p?q HTTP/1.1\r\n">>),
     decode(http_bin, <<"GET https://[::1]:4000/x HTTP/1.1\r\n">>),
     decode(http_bin, <<"GET http://[::1]/x HTTP/1.1\r\n">>),
     decode(http_bin, <<"GET http://example.com HTTP/1.1\r\n">>),
     decode(http_bin, <<"GET http:/x HTTP/1.1\r\n">>),
     decode(http_bin, <<"GET foo:bar HTTP/1.1\r\n">>),
     decode(http_bin, <<"GET foo HTTP/1.1\r\n">>),
     decode(http_bin, <<"GET http://example.com:abc/x HTTP/1.1\r\n">>),
     decode(http_bin, <<"GET http://[::1]:/x HTTP/1.1\r\n">>),
     decode(http_bin, <<"GET http://[::1]:abc/x HTTP/1.1\r\n">>),
     decode(http_bin, <<"HTTP/1.1 200 OK\r\n">>),
     decode(http_bin, <<"HTTP/1.0 404 \r\n">>),
     decode(http_bin, <<"GET / HTTP/1.1\r\nX">>)].

header_lines() ->
    [decode(httph_bin, <<"Host: example.com\r\n\r\n">>),
     decode(httph_bin, <<"content-length: 5\r\n\r\n">>),
     decode(httph_bin, <<"x-foo: bar\r\n\r\n">>),
     decode(httph_bin, <<"X-Long: a\r\n b\r\n\r\n">>),
     decode(httph_bin, <<"X:\r\n\r\n">>),
     decode(httph_bin, <<"X:  value  \r\n\r\n">>),
     decode(httph_bin, <<"X:\tvalue\r\n\r\n">>),
     decode(httph_bin, <<"ThisIsAVeryLongHeaderNameThatExceedsTheFiftyCharacterLimit: v\r\n\r\n">>)].

end_of_headers() ->
    [decode(httph_bin, <<"\r\n">>),
     decode(httph_bin, <<"\n">>),
     decode(httph_bin, <<"Host: x\r\n\r\n">>)].

incomplete() ->
    [decode(http_bin, <<>>),
     decode(http_bin, <<"GET /">>),
     decode(httph_bin, <<"Host: x\r\n">>),
     decode(http_bin, <<"HTTP/1.1 ">>)].

malformed() ->
    [decode(http_bin, <<"BOGUS\r\n">>),
     decode(http_bin, <<"\r\n">>),
     decode(httph_bin, <<"NoColon\r\n\r\n">>),
     decode(httph_bin, <<"Bad Header\r\n\r\n">>),
     decode(http_bin, <<"GE T / HTTP/1.1\r\n">>),
     decode(http_bin, <<"HTTP/1 200 OK\r\n">>),
     decode(http_bin, <<"HTTP/1. 200 OK\r\n">>),
     decode(http_bin, <<"HTTP/1.1\r\n">>),
     decode(http_bin, <<"HTTP/1.1 200OK\r\n">>),
     decode(http_bin, <<"GET \r\n">>),
     decode(http_bin, <<"GET / HTP/1.1\r\n">>),
     decode(http_bin, <<"GET / HTTP/1.\r\n">>),
     decode(http_bin, <<"GET / HTTP/xyz\r\n">>),
     decode(http_bin, <<"GET / HTTP/12x\r\n">>),
     decode(http_bin, <<"GET / HTTP/123\r\n">>),
     decode(http_bin, <<"GET / HTTP/1.a\r\n">>)].

options() ->
    [erlang:decode_packet(http_bin, <<"GET / HTTP/1.1\r\n">>, [{packet_size, 4}]),
     erlang:decode_packet(http_bin, <<"GET / HTTP/1.1\r\n">>, [{packet_size, 100}]),
     erlang:decode_packet(http_bin, <<"GET / HTTP/1.1\r\n">>, [{packet_size, 0}]),
     erlang:decode_packet(httph_bin, <<"Host: x\r\n\r\n">>, [{line_length, 4}]),
     erlang:decode_packet(httph_bin, <<"Host: x\r\n\r\n">>, [{line_length, 100}]),
     erlang:decode_packet(http_bin, <<"GET /">>, [{packet_size, 3}]),
     erlang:decode_packet(httph_bin, <<"Host: x\r\n">>, [{packet_size, 5}])].

%% Each argument error gives `{error, badarg, Info}` with the error
%% information of the first stack frame.
badargs() ->
    [catch_error(http_bin, x, []),
     catch_error(bogus, <<>>, []),
     catch_error(http_bin, <<"x">>, [{packet_size, -1}]),
     catch_error(http_bin, <<"x">>, [{packet_size, big()}]),
     catch_error(http_bin, <<"x">>, [{line_delimiter, 10}]),
     catch_error(http_bin, <<>>, [packet_size]),
     catch_error(http_bin, <<"x">>, [packet_size | foo]),
     catch_error(http_bin, <<"x">>, [{packet_size}]),
     catch_error(http_bin, <<"x">>, [{packet_size, foo}])].

%% A packet type that PBeam does not run stops with a runtime error.
unsupported() ->
    [erlang:decode_packet(http, <<"GET / HTTP/1.1\r\n">>, [])].

big() ->
    1 bsl 32.


catch_error(Type, Subject, Options) ->
    try erlang:decode_packet(Type, Subject, Options)
    catch error:Reason:Stack ->
            [{erlang, decode_packet, _, Info} | _] = Stack,
            {error, Reason, Info}
    end.
