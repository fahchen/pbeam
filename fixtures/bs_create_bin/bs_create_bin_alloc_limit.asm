%% BEAM assembly of bs_create_bin_alloc_limit, in the form that
%% beam_asm:module/4 takes.
%%
%% erlc -S of this source made the code:
%%
%%   wrap(X) -> B = <<"a", X/binary>>, fun() -> B end.
%%
%% Then the Alloc operand of bs_create_bin got the allocation list
%% {alloc,[{words,715827882},{floats,715827882},{funs,1}]}. Its sum is
%% 715827882 + 2 * 715827882 + 2 = 2147483648 heap words, which is more
%% than i32::MAX. erlc gives no such code, and beam_validator rejects it, so
%% beam_asm:module/4 assembles it without the validator:
%%
%%   erl -noshell -eval '{ok, [Code]} = file:consult("fixtures/bs_create_bin/bs_create_bin_alloc_limit.asm"), {ok, Beam} = beam_asm:module(Code, [], [], []), ok = file:write_file("fixtures/bs_create_bin/bs_create_bin_alloc_limit.beam", Beam), halt().'
%%
%% ERTS 29.1.1 rejects the allocation list (marshal_allocation_list,
%% erts/emulator/beam/beam_file.c:2269), so
%% code:load_binary/3 gives {error,badfile}.

{bs_create_bin_alloc_limit,
    [{wrap,1}],
    [],
    #{records => []},
    [{function,wrap,1,2,
         [{label,1},
          {func_info,{atom,bs_create_bin_alloc_limit},{atom,wrap},1},
          {label,2},
          {bs_create_bin,
              {f,0},
              {alloc,[{words,715827882},{floats,715827882},{funs,1}]},
              1,8,
              {x,0},
              {list,
                  [{atom,string},
                   0,8,nil,
                   {string,<<"a">>},
                   {integer,1},
                   {atom,binary},
                   2,8,nil,
                   {x,0},
                   {atom,all}]}},
          {make_fun3,{f,4},0,0,{x,0},{list,[{x,0}]}},
          return]},
     {function,'-wrap/1-fun-0-',1,4,
         [{label,3},
          {func_info,
              {atom,bs_create_bin_alloc_limit},
              {atom,'-wrap/1-fun-0-'},
              1},
          {label,4},
          return]}],
    5}.
