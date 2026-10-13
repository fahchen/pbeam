%% The scene of the live-byte harness of task #587. It builds one screen of
%% the Terra Counter target: a list of lines, each line a list of spans, and
%% each span a pair of a text binary and a style map. It also builds a flat
%% style map, a hash map of styles with more than 32 keys, a local fun with
%% the style map as a free variable, a process dictionary entry, a mailbox
%% message, and a Y register that lives across a call. The harness runs
%% `scene/0` and records the live bytes of the guest containers.
%%
%% The scene uses only core BIFs and local functions, because the harness
%% loads the fixture alone: a call of `lists:seq/2` or another function of a
%% library module needs that module.
-module(term_live_bytes_probe).
-export([scene/0]).

%% Build the scene and return the terms that the harness holds: the render
%% fun, the screen, and the hash map of styles. The held tuple keeps the fun
%% and its hash map live across the yields, so a sample includes them.
scene() ->
    Styles = styles(),
    Hash = hash_styles(),
    Screen = screen(Styles),
    Render = fun(Line) -> render(Line, Hash) end,
    Held = hold(Screen, Render, Hash),
    put(counter, length(Screen)),
    put(styles, Styles),
    self() ! {screen, Screen},
    %% Yield with the screen, the fun, the hash map, the dictionary entries,
    %% and the mailbox message live, so a sample includes those containers.
    ok = pump(3),
    Held.

%% The flat style map of the scene.
styles() ->
    #{fg => <<"black">>, bg => <<"white">>, bold => true, cells => 2}.

%% The hash map of styles: 40 keys, so the map is a HAMT.
hash_styles() ->
    maps:from_list([{Key, <<"style">>} || Key <- seq(1, 40)]).

%% The screen: one counter line and 23 body lines.
screen(Styles) ->
    [
        [
            {<<"count: ">>, Styles},
            {<<"3">>, Styles}
        ]
        | [[{<<"body ">>, Styles}, {<<"line">>, Styles}] || _ <- seq(1, 23)]
    ].

%% Render one line and count its spans. The length gives the harness a
%% small integer result.
render(Line, Hash) ->
    maps:size(Hash) + length(Line).

%% Keep the screen, the render fun, and the hash map in the held result
%% across the yields of `pump/1`, so the process holds their containers at a
%% sampling point.
hold(Screen, Render, Hash) ->
    Count = total(Screen, 0, Render),
    Safe = Screen,
    More = seq(1, 3),
    {Safe, More, Count, Render, Hash}.

%% Yield three times, so the harness samples the live bytes at more than one
%% scheduling step while the caller holds the scene.
pump(0) ->
    ok;
pump(N) ->
    erlang:yield(),
    pump(N - 1).

%% Sum the render length of each line, as `lists:foldl/3` does.
total([], Total, _Render) ->
    Total;
total([Line | Rest], Total, Render) ->
    total(Rest, Render(Line) + Total, Render).

%% The integers from `From` to `To`, as `lists:seq/2` does.
seq(To, To) ->
    [To];
seq(From, To) ->
    [From | seq(From + 1, To)].
