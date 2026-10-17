-module(re_probe).
-export([main/0, options/0, errors/0, badarg/0, iodata/0, unicode_list/0,
         export/0, unsupported/1, id/1,
         run2/0, run/0, captures/0, globals/0, run_errors/0, reductions/0,
         spot/0, internal_run_first/0, yield/0, recursion/0]).

%% prim_tty:init/1 compiles the ANSI regular expression (prim_tty.erl:238).
%% The fifth element is a reference (erl_bif_re.c:607-609).
main() ->
    {ok, {re_pattern, Count, Unicode, Crlf, Ref}} =
        re:compile(prim_tty:ansi_regexp(), [unicode]),
    {ok, {re_pattern, _, _, _, Other}} = re:compile(<<"a">>, []),
    [Count, Unicode, Crlf, kind(Ref), Ref =:= Other].

%% The capture count, the unicode flag, and the newline flag of each
%% compile option (build_compile_result, erl_bif_re.c:581-615).
options() ->
    [info(re:compile(Pattern, Options))
     || {Pattern, Options} <-
            [{<<"a">>, []}, {<<"(a)(?:b)(c)">>, []}, {<<"a">>, [unicode]},
             {<<"a">>, [{newline, cr}]}, {<<"a">>, [{newline, lf}]},
             {<<"a">>, [{newline, crlf}]}, {<<"a">>, [{newline, nul}]},
             {<<"a">>, [{newline, anycrlf}]}, {<<"a">>, [{newline, any}]},
             {<<"(*CRLF)a">>, []}, {<<"(*ANY)a">>, [{newline, cr}]},
             {<<"(*UTF)a">>, []}, {<<"a">>, [{newline, crlf}, {newline, lf}]},
             {<<"(?<n>a)(?<n>b)">>, [dupnames]}, {<<"(a)">>, [no_auto_capture]},
             {<<"a">>, [anchored, caseless, dollar_endonly, dotall, extended,
                        firstline, multiline, ungreedy, ucp, no_start_optimize,
                        bsr_anycrlf, bsr_unicode, never_utf]}]].

%% The PCRE2 message and offset of each compile error
%% (build_compile_error, erl_bif_re.c:802-836).
errors() ->
    [re:compile(Pattern, Options)
     || {Pattern, Options} <-
            [{<<"(a)(b">>, [unicode]}, {<<"(a)(b">>, []}, {<<"[">>, []},
             {<<"a{2,1}">>, []}, {<<"(?<n>a)(?<n>b)">>, []},
             {<<255>>, [unicode]}, {<<"a">>, [never_utf, unicode]},
             {<<"(*UTF)a">>, [never_utf]},
             {[repeat(10001, "(?="), repeat(10001, ")")], []}]].

%% badarg for each option that parse_options rejects
%% (erl_bif_re.c:350-545), each execution option (erl_bif_re.c:914-916),
%% and each pattern that is not iodata (erl_bif_re.c:927-929).
badarg() ->
    [catch_error(fun() -> re:compile(Pattern, Options) end)
     || {Pattern, Options} <-
            [{<<"a">>, foo}, {<<"a">>, [bogus]}, {<<"a">>, [1]},
             {<<"a">>, [{newline, bogus}]}, {<<"a">>, [{bogus, 1}]},
             {<<"a">>, [{1, 2}]}, {<<"a">>, [{newline, lf, x}]},
             {<<"a">>, [global]}, {<<"a">>, [report_errors]},
             {<<"a">>, [notempty]}, {<<"a">>, [notempty_atstart]},
             {<<"a">>, [notbol]}, {<<"a">>, [noteol]},
             {<<"a">>, [{offset, 0}]}, {<<"a">>, [{offset, ?MODULE:id(-1)}]},
             {<<"a">>, [{match_limit, 1}]},
             {<<"a">>, [{match_limit_recursion, 1}]},
             {<<"a">>, [{capture, all}]}, {<<"a">>, [{capture, all, list}]},
             {<<"a">>, [unicode, global]}, {foo, []},
             {[?MODULE:id(256)], []}, {[?MODULE:id(-1)], []},
             {[<<"a">>, foo], []}, {{a}, []}, {foo, [bogus]}]].

%% Without unicode, a pattern is iodata: bytes, binaries, and nested lists
%% (get_iolist_as_bytes, erl_bif_re.c:850-889).
iodata() ->
    [info(re:compile(Pattern, []))
     || Pattern <- ["a(b)", ["a", <<"(b)">>, $c, [[<<"(d)">>]]], <<>>, []]].

%% With unicode, a pattern that is not a binary traps to re:ucompile/2
%% (erl_bif_re.c:921-923).
unicode_list() ->
    [info(re:compile([$a, ?MODULE:id(256), "(b)"], [unicode])),
     info(re:compile(["(a)", <<"(b)">>], [unicode])),
     re:compile("(", [unicode])].

%% The export option (build_compile_export, erl_bif_re.c:644-700).
%% Encoded holds the compile options and the bsr convention of the code,
%% so each compile option gives other bytes. The last bsr option applies.
export() ->
    [exported(re:compile(<<"a">>, [export])),
     exported(re:compile(["a", <<"b">>], [export, caseless])),
     exported(re:compile([$a, ?MODULE:id(256)], [unicode, export]))]
    ++ [exported(re:compile(<<"a">>, [export | Options]))
        || Options <- [[anchored], [caseless], [dollar_endonly], [dotall],
                       [dupnames], [extended], [firstline], [multiline],
                       [ungreedy], [ucp], [no_auto_capture],
                       [no_start_optimize], [bsr_anycrlf], [bsr_unicode],
                       [never_utf],
                       [bsr_anycrlf, bsr_unicode],
                       [bsr_unicode, bsr_anycrlf]]]
    ++ [re:compile(<<"(">>, [export])].

%% Calls that ERTS runs and PBeam does not support yet.
unsupported(chardata) -> re:compile(foo, [unicode]);
unsupported(names) ->
    re:run(<<"abc">>, <<"(?<x>a)(?<y>b)?">>, [{capture, all_names}]);
unsupported(name_index) ->
    re:run(<<"abc">>, <<"(?<x>a)(?<y>b)?">>, [{capture, [x], index}]);
unsupported(name_list) ->
    re:run(<<"abc">>, <<"(?<x>a)(?<y>b)?">>, [{capture, ["x"], index}]).

%% re:run/2: the option list is []. The subject and the pattern are a
%% binary or a list (re_run, erl_bif_re.c:1485-1803).
run2() ->
    [re:run(Subject, Pattern)
     || {Subject, Pattern} <-
            [{<<"abc">>, <<"b">>}, {"abc", "b"}, {<<"abc">>, <<"z">>},
             {<<>>, <<>>}, {[<<"a">>, "bc"], <<"c">>}]].

%% re:run/3 with the options that the Plug serve path uses.
run() ->
    {ok, Compiled} = re:compile(<<"(b)">>, []),
    {ok, Caseless} = re:compile(<<"b">>, [caseless]),
    {ok, CompiledUnicode} = re:compile(<<"(b)">>, [unicode]),
    {re_pattern, _, _, _, CRef} = Compiled,
    [re:run(Subject, RE, Options)
     || {Subject, RE, Options} <- run_cases()] ++
        [re:run(<<"abcabc">>, Compiled, [global]),
         re:run(<<"abcabc">>, Compiled, [{capture, [1], binary}]),
         re:run(<<"abc">>, Caseless, []),
         re:run(<<"abc">>, Compiled, [{newline, lf}]),
         re:run(<<"abc">>, Compiled, [bsr_unicode]),
         re:run("abc", CompiledUnicode, []),
         re:run(<<"abc">>, {re_pattern, -1, 0, 0, CRef}, []),
         re:run(<<>>, {re_pattern, 0, 0, 0, CRef}, [global, {offset, 1}, {newline, crlf}])].

run_cases() ->
    [{<<"abc">>, <<"b">>, []},
     {<<"abc">>, <<"z">>, []},
     {<<"abc">>, <<"(a)(b)">>, [{offset, 1}]},
     {<<"abc">>, <<"b">>, [{offset, 2}]},
     {<<"abc">>, <<"b">>, [{offset, 3}]},
     {"abc", <<"b">>, [{capture, all, list}]},
     {[$a, <<"b">>, "c"], <<"(b)">>, [{capture, all, binary}]},
     {<<"a\nb">>, <<"a.b">>, [dotall]},
     {<<"ab">>, <<"^b">>, [multiline]},
     {<<"abc">>, <<"a b c">>, [extended]},
     {<<"ABC">>, <<"abc">>, [caseless]},
     {<<"abc">>, <<"a+?">>, [ungreedy]},
     {<<"abc">>, <<"(a)(b)">>, [no_auto_capture]},
     {<<"a\r\nb">>, <<"^b">>, [{newline, anycrlf}, multiline]},
     {<<"abc">>, <<"b">>, [bsr_unicode]},
     {<<"abc">>, <<"b">>, [{match_limit, 1000}]},
     {<<"abc">>, <<"b">>, [{match_limit_recursion, 1000}]},
     {<<"abcabc">>, <<"b">>, [{capture, all, binary}]},
     {<<"\x{68}\x{e9}llo"/utf8>>, <<"\x{e9}"/utf8>>, [unicode]},
     {<<"\x{68}\x{e9}llo"/utf8>>, "\x{e9}", [unicode]},
     {<<"abc">>, <<"(b)">>, [unicode, {capture, all, binary}]},
     {<<"abc">>, <<"(b)">>, [unicode, {capture, all, list}]},
     {<<"abc">>, <<"(b)">>, [{capture, all_names}]},
     {<<"abc">>, <<"(b)">>, [notbol]},
     {<<"abc">>, <<"c$">>, [noteol]},
     {<<"abc">>, <<"b">>, [notempty, anchored, no_start_optimize]},
     {<<"abc">>, <<"z">>, [report_errors]},
     {<<"a">>, <<"a">>, [{match_limit, 1}, report_errors]},
     {<<"a">>, <<"a">>, [{match_limit, 1}]},
     {<<"abc">>, <<"(a)(b)?">>, [{capture, all_but_first}]},
     {<<"abc">>, <<"(a)(b)?">>, [{capture, [2, 5], index}]},
     {<<"abc">>, <<"(a)(b)?">>, [{capture, first}]},
     {<<"abc">>, <<"(a)(b)?">>, [{capture, none}]},
     {<<"abc">>, <<"(a)(b)?">>, [{capture, none, list}]},
     {<<"ab">>, <<"(a)(z)?(b)">>, [{capture, all}]},
     {<<"abc">>, <<"a(b">>, [report_errors]},
     {<<"aaaaaaaaaaaaaz">>, <<"(a+)*z">>,
      [{match_limit_recursion, 1}, report_errors]},
     {<<"abc">>, <<"b">>, [caseless, global]},
     {<<"abc">>, <<"(a)(b)?">>, [{capture, [], index}]}].

%% Each capture value and type, on a match with a group that did not take
%% part in the match (build_exec_return, erl_bif_re.c:1047-1256).
captures() ->
    [re:run(<<"abc">>, <<"(a)(b)?">>, [{capture, Values, Type}])
     || Values <- [all, all_but_first, first, none, [0, 1, 2, 7]],
        Type <- [index, list, binary]].

%% A global run with several matches, with an empty match, and with an
%% offset (re.erl:1796-1899).
globals() ->
    [re:run(<<"abcabc">>, <<"b">>, [global]),
     re:run(<<"aaa">>, <<"a*">>, [global]),
     re:run(<<"aba">>, <<"a">>, [global, {offset, 1}]),
     re:run(<<"abc">>, <<"z">>, [global]),
     re:run(<<"abcabc">>, <<"b">>, [global, {capture, all, binary}]),
     re:run(<<"abcabc">>, <<"b">>, [global, {capture, none}]),
     re:run(<<"abcabc">>, <<"b">>, [{capture, all, binary}, global])].

%% The badarg of re:run/2,3 (erl_bif_re.c:1501-1518, 1581-1632, 1646-1701,
%% 1756-1786). Each pair holds a name and the caught result.
run_errors() ->
    {ok, Compiled} = re:compile(<<"b">>, [caseless]),
    [{Label, catch_error(fun() -> re:run(Subject, RE, Options) end)}
     || {Label, Subject, RE, Options} <-
            [{options_type, <<"abc">>, <<"b">>, foo},
             {unknown_option, <<"abc">>, <<"b">>, [bogus]},
             {option_not_atom, <<"abc">>, <<"b">>, [1]},
             {unknown_option_after_global, <<"abc">>, <<"b">>, [global, bogus]},
             {offset_past_subject, <<"abc">>, <<"b">>, [{offset, 5}]},
             {offset_negative, <<"abc">>, <<"b">>, [{offset, ?MODULE:id(-1)}]},
             {offset_float, <<"abc">>, <<"b">>, [{offset, 1.5}]},
             {offset_too_large, <<"abc">>, <<"b">>, [{offset, 2147483648}]},
             {match_limit_negative, <<"abc">>, <<"b">>,
              [{match_limit, ?MODULE:id(-1)}]},
             {capture_type_unknown, <<"abc">>, <<"b">>, [{capture, [1], bogus}]},
             {capture_none_type, <<"abc">>, <<"b">>, [{capture, [1], none}]},
             {capture_values_tuple, <<"abc">>, <<"b">>, [{capture, {a}}]},
             {capture_index_float, <<"abc">>, <<"b">>,
              [{capture, [1.5], index}]},
             {capture_index_negative, <<"abc">>, <<"b">>,
              [{capture, [?MODULE:id(-1)], index}]},
             {export, <<"abc">>, <<"b">>, [export]},
             {global_export, <<"abc">>, <<"b">>, [global, export]},
             {pattern_error, <<"abc">>, <<"a(b">>, []},
             {pattern_not_text, <<"abc">>, foo, []},
             {pattern_not_iodata, <<"abc">>, [foo], []},
             {subject_not_iodata, foo, <<"b">>, []},
             {subject_negative_byte, [?MODULE:id(-1)], <<"b">>, []},
             {subject_tuple, {a}, <<"b">>, []},
             {subject_not_utf8, <<255>>, <<"a">>, [unicode]},
             {pattern_not_utf8, <<"abc">>, <<255>>, [unicode]},
             {compile_option_with_pattern, <<"abc">>, Compiled, [caseless]},
             {newline_mismatch, <<"abc">>, Compiled, [{newline, crlf}]},
             {bsr_mismatch, <<"abc">>, Compiled, [bsr_anycrlf]},
             {foreign_pattern, <<"abc">>, {re_pattern, 1, 0, 0, make_ref()}, []},
             {pattern_tag, <<"abc">>, {pattern, 0, 0, 0, make_ref()}, []},
             {pattern_magic_not_ref, <<"abc">>, {re_pattern, 1, 0, 0, foo}, []},
             {pattern_count_not_small, <<"abc">>,
              {re_pattern, 1 bsl 60, 0, 0, ?MODULE:id(make_ref())}, []}]].

%% One case of each shape of a match, with the value of each one.
spot() ->
    {ok, Compiled} = re:compile(<<"(b)">>, []),
    [re:run(<<"abc">>, <<"b">>, []),
     re:run(<<"abc">>, <<"(a)(b)">>, [{capture, all}]),
     re:run(<<"abc">>, <<"(a)(b)?">>, [{capture, all_but_first}]),
     re:run(<<"abc">>, <<"(a)(b)?">>, [{capture, first}]),
     re:run(<<"abc">>, <<"(a)(b)?">>, [{capture, [1], list}]),
     re:run(<<"abc">>, <<"(a)(b)?">>, [{capture, all, binary}]),
     re:run(<<"abc">>, <<"(a)(b)?">>, [{capture, none}]),
     re:run(<<"abc">>, <<"z">>, []),
     re:run(<<"abc">>, <<"b">>, [{offset, 2}]),
     re:run(<<"abc">>, <<"b">>, [{offset, 3}]),
     re:run(<<"abc">>, <<"(b)">>, [{capture, all_names}]),
     re:run(<<"a">>, <<"a">>, [{match_limit, 1}, report_errors]),
     re:run(<<"abc">>, Compiled, [{capture, [1], binary}]),
     re:run(<<"abc">>, <<"(a)(b)?">>, [{capture, [4], index}])].

%% The reductions that a re:run call charges, as erl_bif_re.c:1718 and 1796
%% read them. erl +S 1 runs the probe on one scheduler.
reductions() ->
    {ok, Compiled} = re:compile(<<"b">>, []),
    [charge(Subject, RE, Options)
     || {Subject, RE, Options} <-
            [{<<"abc">>, <<"b">>, []},
             {<<"abc">>, <<"z">>, []},
             {<<"abc">>, <<"(a)(b)">>, [{capture, all, binary}]},
             {<<"abc">>, Compiled, []},
             {dup(1000, $a), <<"b">>, []},
             {dup(10000, $a), <<"a*b">>, []}]].

charge(Subject, RE, Options) ->
    {reductions, Before} = process_info(self(), reductions),
    Result = re:run(Subject, RE, Options),
    {reductions, After} = process_info(self(), reductions),
    {After - Before, Result}.

info({ok, {re_pattern, Count, Unicode, Crlf, Ref}}) when is_reference(Ref) ->
    {Count, Unicode, Crlf}.

exported({ok, {re_exported_pattern, Header, Source, Options, Encoded}}) ->
    [Header, Source, Options, Encoded].

kind(Ref) when is_reference(Ref) -> reference;
kind(_) -> other.

%% Part N times as iodata. Each step takes a block of Size parts and
%% doubles it, so the iodata has few list cells.
repeat(N, Part) -> repeat(N, Part, 1, Part, []).

repeat(0, _, _, _, Acc) -> Acc;
repeat(N, Part, Size, Block, Acc) when Size =< N ->
    repeat(N - Size, Part, Size + Size, [Block, Block], [Block | Acc]);
repeat(N, Part, _, _, Acc) -> repeat(N, Part, 1, Part, Acc).

id(X) -> X.

%% A pattern that recurses without a match (PCRE2_ERROR_RECURSELOOP,
%% pcre2.h:418) charges one turn of reductions and gives nomatch
%% (erl_bif_re.c:1751-1754). Only the value is compared.
recursion() ->
    [re:run(<<"x">>, <<"(?R)">>, []), re:run(<<"xx">>, <<"a(?R)">>, [])].

%% A match that uses the loop limit of one turn continues with a fresh
%% budget (`erl_bif_re.c:1730-1748`, `1852-1866`). The reduction total of
%% this call is the ERTS one, but ERTS switches the process out between the
%% turns and PBeam runs them in one call, so only the value is compared.
yield() ->
    [re:run(dup(600000, $a) ++ "b", <<"a*b">>), re:run(dup(600000, $a), <<"a*b">>)].

%% The fourth argument of re:internal_run/4 must be true or false
%% (erl_bif_re.c:1805-1816). ERTS does not export the function, so only
%% PBeam runs this probe.
internal_run_first() ->
    [catch_error(fun() -> re:internal_run(<<"abc">>, <<"b">>, [], First) end)
     || First <- [true, false, foo, 1]].

%% A list of N copies of C.
dup(N, C) -> dup(N, C, []).
dup(0, _, Acc) -> Acc;
dup(N, C, Acc) -> dup(N - 1, C, [C | Acc]).

catch_error(Fun) ->
    try Fun() catch error:Reason -> {error, Reason} end.
