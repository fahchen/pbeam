%% Run the prim_file NIFs of the file_server write and metadata paths on a
%% scratch directory, and return a digest of each result.
%%
%% The caller gives one directory that exists when the run starts. The probe
%% makes each other path in the directory, and it leaves the directory empty.
%% The digest keeps the results short and compares each term exactly.
-module(prim_extras_probe).
-export([main/1]).

-include_lib("kernel/include/file.hrl").

main([Dir]) ->
    {ok, Original} = prim_file:get_cwd(),
    File = join(Dir, "file"),
    Moved = join(Dir, "moved"),
    Hard = join(Dir, "hard"),
    Link = join(Dir, "link"),
    Dangling = join(Dir, "dangling"),
    Sub = join(Dir, "sub"),
    Nested = join(Sub, "nested"),
    NestedFile = join(Nested, "nested-file"),
    Cwd = join(Dir, "cwd"),
    CwdChild = join(Cwd, "child"),
    Results = [
        prim_file:make_dir(Sub),
        prim_file:make_dir(Sub),
        prim_file:write_file(File, <<"abc">>),
        prim_file:read_file(File),
        info(File),
        prim_file:write_file_info(
            File,
            #file_info{mode = 8#600, atime = 1000, mtime = 2000, ctime = 3000},
            [{time, posix}]
        ),
        info_posix(File),
        prim_file:altname(File),
        prim_file:rename(File, Moved),
        prim_file:read_file(Moved),
        prim_file:make_link(Moved, Hard),
        prim_file:make_link(join(Dir, "missing"), Hard),
        prim_file:make_symlink("moved", Link),
        prim_file:make_symlink("missing", Dangling),
        prim_file:read_link(Link),
        link_type(Link),
        prim_file:delete(Hard),
        prim_file:delete(Moved),
        prim_file:delete(Moved),
        prim_file:delete(Link),
        prim_file:delete(Dangling),
        prim_file:delete(Sub),
        prim_file:make_dir(Nested),
        prim_file:write_file(NestedFile, <<>>),
        prim_file:del_dir(Sub),
        prim_file:delete(NestedFile),
        prim_file:del_dir(Nested),
        prim_file:del_dir(Sub),
        prim_file:write_file(File, <<>>),
        prim_file:write_file_info(File, #file_info{mode = -1, atime = 1000, mtime = 2000, ctime = 3000}, [{time, posix}]),
        prim_file:write_file_info(File, #file_info{uid = 1 bsl 40, atime = 1000, mtime = 2000, ctime = 3000}, [{time, posix}]),
        prim_file:write_file_info(File, #file_info{atime = 1 bsl 70, mtime = 2000, ctime = 3000}, [{time, posix}]),
        prim_file:delete(File),
        prim_file:altname("/"),
        prim_file:make_dir(Cwd),
        prim_file:write_file(CwdChild, <<>>),
        cwd_of(Original, Cwd),
        prim_file:delete(CwdChild),
        prim_file:del_dir(Cwd)
    ],
    erlang:phash2(Results).

%% Return the path of the entry `Name` in the directory `Dir`.
join(Dir, Name) ->
    Dir ++ "/" ++ Name.

%% Return the fields of the information of `Name` that do not change from
%% run to run.
info(Name) ->
    {ok, Info} = prim_file:read_file_info(Name),
    {Info#file_info.type, Info#file_info.size, Info#file_info.access}.

%% Return the permission bits and the access and modification times of
%% `Name`, which `write_file_info/3` with the `posix` time sets.
info_posix(Name) ->
    {ok, Info} = prim_file:read_file_info(Name, [{time, posix}]),
    {Info#file_info.mode band 8#777, Info#file_info.atime, Info#file_info.mtime}.

%% Return the type of the link `Name` itself.
link_type(Name) ->
    {ok, Info} = prim_file:read_link_info(Name),
    Info#file_info.type.

%% Set the current directory to the non-empty directory `Cwd`, so that
%% `efile_del_dir` gives `einval` for the current directory, and then set
%% the current directory back to the directory `Original` of the run.
cwd_of(Original, Cwd) ->
    ok = prim_file:set_cwd(Cwd),
    Current = prim_file:del_dir(Cwd),
    {ok, Got} = prim_file:get_cwd(),
    ok = prim_file:set_cwd(Original),
    {Current, is_list(Got) andalso Got =/= []}.
