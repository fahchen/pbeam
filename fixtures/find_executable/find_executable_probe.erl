-module(find_executable_probe).
-export([main/0, bifs/1, names/1]).

%% os:find_executable/1,2 and file:read_file_info/1 through the file server
%% file_server_2, which pbeam_boot and the OTP boot start. The test makes
%% the files of Dir (the environment variable PBEAM_FIND_EXECUTABLE_DIR):
%% bin1/tool (0644), bin1/sub/ (0755), bin2/tool (0755), bin2/sub (0755),
%% bin2/link (a symbolic link to tool), write_only (0200), no_access (0000),
%% old (a time before 1902), future (a time after 9999), and socket (a Unix
%% socket). os:verify_executable/3 accepts a regular file with an execute bit
%% (os.erl:452-471).
main() ->
    Dir = os:getenv("PBEAM_FIND_EXECUTABLE_DIR"),
    Path = Dir ++ "/bin1:" ++ Dir ++ "/bin2",
    Files = ["bin1/tool", "bin1/sub", "bin2/tool", "bin2/sub", "bin2/link", "write_only", "no_access",
             "old", "future", "socket", "missing", "bin2/tool/x"],
    {os:find_executable("sh"),
     [os:find_executable(Name, Path)
      || Name <- ["tool", "sub", "link", "missing", Dir ++ "/bin2/tool", Dir ++ "/bin1/tool",
                  "bin2/tool"]],
     os:find_executable("tool", Dir ++ "/bin1"),
     [file:read_file_info(Dir ++ "/" ++ File) || File <- Files],
     file:read_link_info(Dir ++ "/bin2/link"),
     file:read_file_info(unicode:characters_to_binary(Dir ++ "/bin2/tool")),
     file:read_file_info([Dir, "/bin2", ["/", <<"tool">>]]),
     [file:read_file_info(Name) || Name <- ["", [0], <<0>>, {}, [a], [256 | Dir]]],
     device(file:read_file_info("/dev/null"))}.

%% The type and the access of a device file. Other processes of the host can
%% change its times.
device({ok, Info}) -> {element(3, Info), element(4, Info)}.

%% The BIFs of the find_executable path, called directly, and the native
%% binding prim_file:read_info_nif/2 through prim_file:read_file_info/1,2 and
%% prim_file:read_link_info/1,2 (prim_file.erl:647-681). prim_file does not
%% export read_info_nif/2. Dir is the directory of main/0.
bifs(Dir) ->
    Tool = Dir ++ "/bin2/tool",
    Link = Dir ++ "/bin2/link",
    [file:native_name_encoding(),
     [call(erlang, dt_spread_tag, [X]) || X <- [true, false, x, 1]],
     [call(erlang, dt_restore_tag, [X]) || X <- [true, false, {0, []}, x]],
     [call(erlang, list_to_tuple, [X]) || X <- [[], [a, 1], "ab", [a | b], a, <<"a">>]],
     [call(prim_file, Function, Args)
      || {Function, Args} <- [{read_file_info, [Tool]},
                              {read_file_info, [unicode:characters_to_binary(Tool)]},
                              {read_file_info, [Tool, [{time, posix}]]},
                              {read_file_info, [Tool, [{time, universal}]]},
                              {read_link_info, [Link]}, {read_link_info, [Link, [{time, posix}]]},
                              {read_file_info, [Link]}, {read_file_info, [Tool ++ "/x"]},
                              {read_file_info, [Dir ++ "/missing"]}, {read_file_info, [[0]]},
                              {read_file_info, [<<"a", 0>>]}, {read_file_info, [x]},
                              {read_file_info, [""]}]]].

%% prim_file:internal_name2native/1 with each name form. Big is a bignum,
%% 1 bsl 60: a bignum in a literal is not a PBeam term, and the compiler
%% folds a constant bignum into the literal of the list.
names(Big) ->
    [call(prim_file, internal_name2native, [Name])
     || Name <- ["abc", ["a", [$b, []], "c"], [], <<"abc">>, <<>>, <<"a", 0>>, <<0>>, abc,
                 [0], "a" ++ [0], [233], [256], [16#10FFFF], [16#110000], [16#D7FF],
                 [16#D800], [16#DFFF], [16#E000], [-1], [a], ["a" | b], [$a | b], {}, 1,
                 [<<"a">>], [Big]]].

call(Module, Function, Args) ->
    try apply(Module, Function, Args) catch Class:Reason -> {Class, Reason} end.
