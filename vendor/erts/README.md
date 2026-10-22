# ERTS NIF headers

These files are the NIF headers of OTP 29.1.1, the pinned ERTS of `mise.toml`: `erl_nif.h`, `erl_nif_api_funcs.h`, `erl_drv_nif.h`, and `erl_int_sizes_config.h`. They are copies of the `usr/include` files of the pinned Erlang install, with no change.

`erl_int_sizes_config.h` is the configure output of an LP64 build, the data model of the macOS and Linux targets of PBeam.

The tests compile `fixtures/nif/nif_add.c` and the layout probe of `src/runtime/nif/abi/layout.rs` with these headers.
