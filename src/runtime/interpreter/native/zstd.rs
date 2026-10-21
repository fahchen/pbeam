//! The load-check table of the `zstd` NIF library
//! (`erts/emulator/nifs/common/zstd_nif.c`), which `zstd:on_load/0` loads
//! (`zstd.erl:251-252`).
//!
//! PBeam has no binding of a `zstd` NIF. A call of a function of the table
//! stops the run with the runtime error of the static library
//! (`Library::unbound_fails`). The load callback of the library returns 0
//! for each load info (`zstd_nif.c:1008-1070`).

/// The functions of the ERTS `zstd` NIF library, as `(name, arity)` in the
/// order of `nif_funcs` (`zstd_nif.c:1082-1116`). `erlang:load_nif/2`
/// checks each of them in this order. The flags of
/// `load_compress_dictionary_nif/2`, `load_decompress_dictionary_nif/2`,
/// `create_cdict_nif/2`, and `create_ddict_nif/1` are
/// `ERL_NIF_DIRTY_JOB_CPU_BOUND` (`zstd_nif.c:1086-1087`, `1093-1094`,
/// `1108-1109`), and the flags of each other function are 0.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(FUNCTIONS[0], ("init_compress_nif", 0));
/// assert_eq!(FUNCTIONS[23], ("get_frame_header_nif", 1));
/// ```
pub(super) const FUNCTIONS: [(&str, u32); 24] = [
    ("init_compress_nif", 0),
    ("init_decompress_nif", 0),
    ("load_compress_dictionary_nif", 2),
    ("ref_compress_dictionary_nif", 2),
    ("set_compress_parameter_nif", 3),
    ("get_compress_parameter_nif", 2),
    ("set_pledged_src_size_nif", 2),
    ("load_decompress_dictionary_nif", 2),
    ("ref_decompress_dictionary_nif", 2),
    ("set_decompress_parameter_nif", 3),
    ("get_decompress_parameter_nif", 2),
    ("compress_stream_nif", 3),
    ("decompress_stream_nif", 3),
    ("compress_reset_nif", 1),
    ("decompress_reset_nif", 1),
    ("compress_close_nif", 1),
    ("decompress_close_nif", 1),
    ("create_cdict_nif", 2),
    ("create_ddict_nif", 1),
    ("getDictId_fromCDict_nif", 1),
    ("getDictId_fromDDict_nif", 1),
    ("getDictId_fromDict_nif", 1),
    ("getDictId_fromFrame_nif", 1),
    ("get_frame_header_nif", 1),
];
