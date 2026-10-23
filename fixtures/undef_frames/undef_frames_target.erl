-module(undef_frames_target).
-export([present/0]).

%% A loaded module without `missing/1`, the target of the failed calls of
%% `undef_frames`.
present() -> ok.
