-module(prim_tty).
-export([load/1, isatty/1, tty_create/1, tty_init/2, setlocale/1, tty_select/2,
         tty_window_size/1, tty_encoding/1, tty_is_open/2, write_nif/2, read_nif/3,
         isprint/1, wcwidth/1, wcswidth/1, sizeof_wchar/0, setupterm_nif/0,
         tigetnum_nif/1, tigetflag_nif/1, tigetstr_nif/1, tinfo_nif/0, tputs_nif/2]).

load(Path) -> erlang:load_nif(Path, #{}).

isatty(_) -> erlang:nif_error(undef).
tty_create(_) -> erlang:nif_error(undef).
tty_init(_, _) -> erlang:nif_error(undef).
setlocale(_) -> erlang:nif_error(undef).
tty_select(_, _) -> erlang:nif_error(undef).
tty_window_size(_) -> erlang:nif_error(undef).
tty_encoding(_) -> erlang:nif_error(undef).
tty_is_open(_, _) -> erlang:nif_error(undef).
write_nif(_, _) -> erlang:nif_error(undef).
read_nif(_, _, _) -> erlang:nif_error(undef).
isprint(_) -> erlang:nif_error(undef).
wcwidth(_) -> erlang:nif_error(undef).
wcswidth(_) -> erlang:nif_error(undef).
sizeof_wchar() -> erlang:nif_error(undef).
setupterm_nif() -> erlang:nif_error(undef).
tigetnum_nif(_) -> erlang:nif_error(undef).
tigetflag_nif(_) -> erlang:nif_error(undef).
tigetstr_nif(_) -> erlang:nif_error(undef).
tinfo_nif() -> erlang:nif_error(undef).
tputs_nif(_, _) -> erlang:nif_error(undef).
