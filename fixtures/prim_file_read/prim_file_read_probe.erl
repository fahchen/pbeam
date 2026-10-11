-module(prim_file_read_probe).
-export([main/1]).

%% prim_file:read_file/1, list_dir/1, get_cwd/0, and read_link/1 of the pinned
%% prim_file module, which call the NIFs read_file_nif/1, list_dir_nif/1,
%% get_cwd_nif/0, and read_link_nif/1, and prim_file:internal_native2name/1
%% (prim_file.erl:564-646, 787-793, 894-897). Dir is the directory of the test
%% files, as a list of bytes. The test makes: file ("hello"), big (70000
%% bytes), dir/ with a and b, empty/, caf<e9>/ (the current directory of the
%% run, in UTF-8), link (to file), dangling (to missing), long (to a name of
%% 300 bytes), and raw_link (to <<"target",255>>). When the file system of the
%% host accepts names that are not UTF-8, it also makes raw/ with the file
%% <<"name",255>> and the directory <<"dir",255>>/ with the file inner.
main(Dir) ->
    D = Dir ++ "/",
    Raw = list_to_binary(D ++ "raw/"),
    Paths = [D, D ++ "file", D ++ "missing", D ++ "dir", D ++ "empty", D ++ "link", D ++ "dangling",
             D ++ "long", D ++ "raw_link", D ++ "file/x", D ++ [99, 97, 102, 233],
             list_to_binary(D ++ "file"), [D, ["fi", <<"le">>]], <<Raw/binary, "name", 255>>,
             <<Raw/binary, "dir", 255>>, "", <<>>, <<"a", 0>>, [0], a, [256], [16#10FFFF], {}],
    {[{Function, [prim_file:Function(Path) || Path <- Paths]}
      || Function <- [read_file, list_dir, read_link]],
     big(prim_file:read_file(D ++ "big")),
     prim_file:read_file("/dev/null"),
     prim_file:get_cwd(),
     prim_file:list_dir_all(Raw),
     prim_file:read_link_all(D ++ "raw_link"),
     [call(prim_file, internal_native2name, [Name])
      || Name <- [<<>>, <<"abc">>, <<"caf", 195, 169>>, <<"a", 255>>, <<"a", 0, "b">>,
                  <<240, 159, 152, 128>>, <<192, 128>>, <<237, 160, 128>>, <<244, 144, 128, 128>>,
                  <<"a", 195>>, "abc", a, 1]]}.

%% The size and the first and last 4 bytes of the contents of big.
big({ok, Bytes}) ->
    {byte_size(Bytes), binary_part(Bytes, 0, 4), binary_part(Bytes, byte_size(Bytes), -4)}.

call(Module, Function, Args) ->
    try apply(Module, Function, Args) catch Class:Reason -> {Class, Reason} end.
