-module(zstd).
-export([load/2]).
-export([init_compress_nif/0, init_decompress_nif/0, load_compress_dictionary_nif/2, ref_compress_dictionary_nif/2, set_compress_parameter_nif/3, get_compress_parameter_nif/2, set_pledged_src_size_nif/2, load_decompress_dictionary_nif/2, ref_decompress_dictionary_nif/2, set_decompress_parameter_nif/3, get_decompress_parameter_nif/2, compress_stream_nif/3, decompress_stream_nif/3, compress_reset_nif/1, decompress_reset_nif/1, compress_close_nif/1, decompress_close_nif/1, create_cdict_nif/2, create_ddict_nif/1, getDictId_fromCDict_nif/1, getDictId_fromDDict_nif/1, getDictId_fromDict_nif/1, getDictId_fromFrame_nif/1, get_frame_header_nif/1]).
-nifs([init_compress_nif/0, init_decompress_nif/0, load_compress_dictionary_nif/2, ref_compress_dictionary_nif/2, set_compress_parameter_nif/3, get_compress_parameter_nif/2, set_pledged_src_size_nif/2, load_decompress_dictionary_nif/2, ref_decompress_dictionary_nif/2, set_decompress_parameter_nif/3, get_decompress_parameter_nif/2, compress_stream_nif/3, decompress_stream_nif/3, compress_reset_nif/1, decompress_reset_nif/1, compress_close_nif/1, decompress_close_nif/1, create_cdict_nif/2, create_ddict_nif/1, getDictId_fromCDict_nif/1, getDictId_fromDDict_nif/1, getDictId_fromDict_nif/1, getDictId_fromFrame_nif/1, get_frame_header_nif/1]).

%% A stand-in for the zstd module with each function of the nif_funcs
%% table of its NIF library (zstd_nif.c:1082-1116). load/2 returns the result of
%% erlang:load_nif/2 for each load info, in order. An error does not load
%% the library, so the next load info runs the checks again.
load(Lib, Infos) ->
    [erlang:load_nif(Lib, Info) || Info <- Infos].

init_compress_nif() -> erlang:nif_error(undef).
init_decompress_nif() -> erlang:nif_error(undef).
load_compress_dictionary_nif(_, _) -> erlang:nif_error(undef).
ref_compress_dictionary_nif(_, _) -> erlang:nif_error(undef).
set_compress_parameter_nif(_, _, _) -> erlang:nif_error(undef).
get_compress_parameter_nif(_, _) -> erlang:nif_error(undef).
set_pledged_src_size_nif(_, _) -> erlang:nif_error(undef).
load_decompress_dictionary_nif(_, _) -> erlang:nif_error(undef).
ref_decompress_dictionary_nif(_, _) -> erlang:nif_error(undef).
set_decompress_parameter_nif(_, _, _) -> erlang:nif_error(undef).
get_decompress_parameter_nif(_, _) -> erlang:nif_error(undef).
compress_stream_nif(_, _, _) -> erlang:nif_error(undef).
decompress_stream_nif(_, _, _) -> erlang:nif_error(undef).
compress_reset_nif(_) -> erlang:nif_error(undef).
decompress_reset_nif(_) -> erlang:nif_error(undef).
compress_close_nif(_) -> erlang:nif_error(undef).
decompress_close_nif(_) -> erlang:nif_error(undef).
create_cdict_nif(_, _) -> erlang:nif_error(undef).
create_ddict_nif(_) -> erlang:nif_error(undef).
getDictId_fromCDict_nif(_) -> erlang:nif_error(undef).
getDictId_fromDDict_nif(_) -> erlang:nif_error(undef).
getDictId_fromDict_nif(_) -> erlang:nif_error(undef).
getDictId_fromFrame_nif(_) -> erlang:nif_error(undef).
get_frame_header_nif(_) -> erlang:nif_error(undef).
