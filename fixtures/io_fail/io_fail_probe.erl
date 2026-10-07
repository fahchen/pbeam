-module(io_fail_probe).
-export([main/0]).

%% Write to standard_io, whose writer fails, and catch the class and the
%% reason of the exception. Then wait until the default handler has written
%% the report of user_drv (logger_std_h:filesync/1), and write the result
%% to standard_error.
main() ->
    Result = try io:put_chars(standard_io, [<<"Hello, world!">>, 10]) of
                 Value -> {ok, Value}
             catch
                 Class:Reason -> {Class, Reason}
             end,
    ok = logger_std_h:filesync(default),
    io:format(standard_error, "result ~p~n", [Result]).
