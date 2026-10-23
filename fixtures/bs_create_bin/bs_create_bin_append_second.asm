%% BEAM assembly of bs_create_bin_append_second, in the form that
%% beam_asm:module/4 takes.
%%
%% erlc -S of `second(X) -> <<"a", X/binary>>.` made the code. Then the
%% second segment got the type `append`. erlc gives an `append` segment
%% only as the first segment, so beam_asm:module/4 assembles it without
%% the validator:
%%
%%   erl -noshell -eval '{ok, [Code]} = file:consult("fixtures/bs_create_bin/bs_create_bin_append_second.asm"), {ok, Beam} = beam_asm:module(Code, [], [], []), ok = file:write_file("fixtures/bs_create_bin/bs_create_bin_append_second.beam", Beam), halt().'

{bs_create_bin_append_second,
    [{second,1}],
    [],
    #{records => []},
    [{function,second,1,2,
         [{label,1},
          {func_info,{atom,bs_create_bin_append_second},{atom,second},1},
          {label,2},
          {bs_create_bin,
              {f,0},
              0,
              1,8,
              {x,0},
              {list,
                  [{atom,string},
                   0,8,nil,
                   {string,<<"a">>},
                   {integer,1},
                   {atom,append},
                   2,8,nil,
                   {x,0},
                   {atom,all}]}},
          return]}],
    3}.
