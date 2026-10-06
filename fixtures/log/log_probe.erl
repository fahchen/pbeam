-module(log_probe).
-export([main/0]).
-include_lib("kernel/include/logger.hrl").

%% Log one error event through the default handler. Then wait until the
%% file control process of the handler has written it to user
%% (logger_std_h:filesync/1, logger_std_h.erl:208-209, 619-623).
main() ->
    ok = ?LOG_ERROR("Writer crashed (~p)", [epipe]),
    ok = logger_std_h:filesync(default).
