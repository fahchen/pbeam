use std::process::{Command, Output, Stdio};

#[path = "../common/boot.rs"]
mod boot;
#[expect(
    dead_code,
    reason = "this test crate gives BEAM paths, not bytes, to the binary"
)]
#[path = "../common/mod.rs"]
mod common;

use boot::{BOOT_MODULES, boot_beam, otp_root};
use common::pinned_beam_path;

/// The probe that writes the code server results.
const CODE_SERVER_PROBE: &str = "fixtures/code_server/code_server_probe";

/// The probe of the modules with an `-on_load` function.
const ON_LOAD_PROBE: &str = "fixtures/code_server/on_load_server_probe";

/// The probe of the fun entries of the code of a failed `-on_load`
/// function.
const FUN_ENTRY_PROBE: &str = "fixtures/code_server/fun_entry_probe";

/// The output of `crypto_engine_probe:main/0` on PBeam and on the pinned
/// macOS ERTS (OpenSSL 3.5.8), which has no engine in its list before
/// the first engine lookup.
const ENGINE_RESULTS: &str =
    "[[engine_method_rsa,engine_method_dsa,engine_method_dh,engine_method_rand,
  engine_method_ciphers,engine_method_digests,engine_method_pkey_meths,
  engine_method_pkey_asn1_meths,engine_method_ec],
 [],
 {error,badarg},
 {error,badarg},
 {error,badarg},
 {error,badarg},
 {error,badarg},
 {error,badarg},
 {error,badarg},
 {error,badarg},
 {error,badarg},
 {error,badarg},
 {error,badarg},
 {error,badarg}]
";

#[test]
fn runs_the_engine_functions_as_pinned_erts() {
    // engine.c:353-1045: PBeam has no engine, so each call that needs no
    // loaded engine gives the ERTS result. The Linux CI ERTS links another
    // OpenSSL, whose engine list can differ, so only macOS compares.
    let pbeam = run(&mut pbeam_probe(
        CRYPTO_ENGINE_PROBE,
        "crypto_engine_probe",
        "main",
        &[],
    ));
    assert_eq!(pbeam, ENGINE_RESULTS);
    if cfg!(target_os = "macos") {
        let erts = run(&mut erts_probe(
            CRYPTO_ENGINE_PROBE,
            "crypto_engine_probe",
            "main",
            &[],
        ));
        assert_eq!(erts, ENGINE_RESULTS);
    }
}

#[test]
fn reports_an_engine_load_as_unsupported() {
    // The safe openssl crate has no engine API (engine.c:375, 477, 959).
    for (function, expected) in [
        (
            "by_id",
            "crypto:engine_by_id_nif/1 NIF is not supported for an engine lookup",
        ),
        (
            "load",
            "crypto:engine_load_dynamic_nif/0 NIF is not supported for the engine dynamic",
        ),
        (
            "ensure",
            "crypto:ensure_engine_loaded_nif/2 NIF is not supported for an engine load",
        ),
    ] {
        let output = pbeam_probe(CRYPTO_ENGINE_PROBE, "crypto_engine_probe", function, &[])
            .env("TERM", "dumb")
            .stdin(Stdio::null())
            .output()
            .unwrap();
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains(expected), "{function}: {stderr}");
    }
}

#[test]
fn computes_mod_pow_and_srp_as_pinned_erts() {
    // mod_exp_nif/4 (bn.c:101-165) and the SRP NIFs (srp.c:26-300): the
    // bytes of each result with fixed inputs, `badarg`, and `error`.
    let erts = run(&mut erts_probe(
        CRYPTO_BN_PROBE,
        "crypto_bn_probe",
        "main",
        &[],
    ));
    assert!(erts.contains("badarg"), "{erts}");
    assert_eq!(
        run(&mut pbeam_probe(
            CRYPTO_BN_PROBE,
            "crypto_bn_probe",
            "main",
            &[]
        )),
        erts
    );
}

/// The output of `crypto_rand_probe:main/0` on the pinned ERTS: the
/// shapes, the ranges, and the errors do not depend on the random bytes.
const CRYPTO_RAND_OUTPUT: &str = "[[{0,1,true},
  {0,10,true},
  {5,6,true},
  {-5,5,true},
  {-10,-9,true},
  {18446744073709551616,18446744073709551619,true},
  {0,1606938044258990275541962092341162602522202993782792835301376,true},
  {255,256,true},
  {10,10,{error,badarg}},
  {10,5,{error,badarg}}],
 7,
 [{true,2,true},{true,0,true},{true,0,true}],
 [{error,badarg},
  {error,badarg},
  {error,badarg},
  {error,badarg},
  {error,badarg},
  {error,badarg}],
 [ok,ok,{error,function_clause}],
 [{true,true,5},{true,true,5},{true,true,5},{error,function_clause}],
 [true,9]]
";

#[test]
fn runs_the_random_functions_as_pinned_erts() {
    // rand_uniform_nif/2 and strong_rand_bytes_nif/1 (rand.c:26-137)
    // through crypto:rand_uniform/2 and the crypto plugins of rand
    // (crypto.erl:2256-2282).
    assert_eq!(
        run(&mut erts_probe(
            CRYPTO_RAND_PROBE,
            "crypto_rand_probe",
            "main",
            &[]
        )),
        CRYPTO_RAND_OUTPUT
    );
    assert_eq!(
        run(&mut pbeam_probe(
            CRYPTO_RAND_PROBE,
            "crypto_rand_probe",
            "main",
            &[]
        )),
        CRYPTO_RAND_OUTPUT
    );
}

/// The output of `crypto_bn_probe:kem_errors/0` on the pinned macOS ERTS
/// (OpenSSL 3.5.8). The Linux CI ERTS links OpenSSL 3.0.x, which has no
/// ML-KEM (`evp.c:32`, `73-75`).
const KEM_ERRORS_HOST: &str = r#"[{error,{error,#{c_function_arg_num => -1,c_file_name => "pkey.c",
                 c_file_line_num => 649},
               "Can't create PKEY fromdata"}},
 {error,{error,#{c_function_arg_num => -1,c_file_name => "pkey.c",
                 c_file_line_num => 649},
               "Can't create PKEY fromdata"}},
 {error,{error,#{c_function_arg_num => 1,c_file_name => "pkey.c",
                 c_file_line_num => 621},
               "Invalid public key"}},
 {error,{error,#{c_function_arg_num => 0,c_file_name => "pkey.c",
                 c_file_line_num => 612},
               "Invalid key type"}},
 {error,{error,#{c_function_arg_num => 0,c_file_name => "pkey.c",
                 c_file_line_num => 612},
               "Invalid key type"}},
 {error,{error,#{c_function_arg_num => -1,c_file_name => "pkey.c",
                 c_file_line_num => 616},
               "Can't create PKEY_CTX from name"}},
 {error,{error,#{c_function_arg_num => 1,c_file_name => "pkey.c",
                 c_file_line_num => 621},
               "Invalid public key"}},
 {error,{error,#{c_function_arg_num => 1,c_file_name => "pkey.c",
                 c_file_line_num => 621},
               "Invalid public key"}},
 {error,{error,#{c_function_arg_num => 2,c_file_name => "evp.c",
                 c_file_line_num => 92},
               "Invalid encapsulated secret"}},
 {error,{error,#{c_function_arg_num => -1,c_file_name => "pkey.c",
                 c_file_line_num => 649},
               "Can't create PKEY fromdata"}},
 {error,{error,#{c_function_arg_num => -1,c_file_name => "pkey.c",
                 c_file_line_num => 616},
               "Can't create PKEY_CTX from name"}}]
"#;

#[test]
fn checks_the_kem_arguments_as_pinned_erts() {
    // encapsulate_key_nif/2 and decapsulate_key_nif/3 (evp.c:30-129,
    // pkey.c:587-665): the error tuple of each failed check.
    assert_eq!(
        run(&mut pbeam_probe(
            CRYPTO_BN_PROBE,
            "crypto_bn_probe",
            "kem_errors",
            &[]
        )),
        KEM_ERRORS_HOST
    );
    if cfg!(target_os = "macos") {
        let erts = run(&mut erts_probe(
            CRYPTO_BN_PROBE,
            "crypto_bn_probe",
            "kem_errors",
            &[],
        ));
        assert_eq!(erts, KEM_ERRORS_HOST);
    }
}

#[test]
fn reports_ml_kem_encapsulation_as_unsupported() {
    // The safe openssl crate 0.10.75 has no EVP_PKEY_encapsulate and no
    // EVP_PKEY_decapsulate (evp.c:50-60, 104-113).
    for (function, expected) in [
        (
            "encapsulate",
            "crypto:encapsulate_key_nif/2 NIF is not supported for an ML-KEM key: the safe \
             openssl crate has no EVP_PKEY_encapsulate",
        ),
        (
            "decapsulate",
            "crypto:decapsulate_key_nif/3 NIF is not supported for an ML-KEM key: the safe \
             openssl crate has no EVP_PKEY_decapsulate",
        ),
        (
            "other_key_type",
            "crypto:encapsulate_key_nif/2 NIF is not supported for the algorithm rsa",
        ),
    ] {
        let output = pbeam_probe(CRYPTO_BN_PROBE, "crypto_bn_probe", function, &[])
            .env("TERM", "dumb")
            .stdin(Stdio::null())
            .output()
            .unwrap();
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(
            !output.status.success() && stderr.contains(expected),
            "{function}: {stderr}"
        );
    }
}

/// The probe of `crypto:strong_rand_bytes/1`.
const CRYPTO_PROBE: &str = "fixtures/code_server/crypto_probe";
const CRYPTO_SIGN_PROBE: &str = "fixtures/code_server/crypto_sign_probe";
const CRYPTO_CRYPT_PROBE: &str = "fixtures/code_server/crypto_crypt_probe";
const CRYPTO_KEYGEN_PROBE: &str = "fixtures/code_server/crypto_keygen_probe";
const CRYPTO_BN_PROBE: &str = "fixtures/code_server/crypto_bn_probe";
const CRYPTO_ENGINE_PROBE: &str = "fixtures/code_server/crypto_engine_probe";

/// The probe of the random functions of `crypto` and its `rand` plugins.
const CRYPTO_RAND_PROBE: &str = "fixtures/code_server/crypto_rand_probe";
const CRYPTO_COMPUTE_KEY_PROBE: &str = "fixtures/code_server/crypto_compute_key_probe";

/// The stand-in `crypto` module that loads the `crypto` NIF library.
const CRYPTO_STAND_IN: &str = "fixtures/crypto/crypto";

/// The probe that loads the stand-in of each module of an OTP NIF library
/// without bindings in `fixtures/nif_libraries`.
const NIF_LIBRARY_PROBE: &str = "fixtures/nif_libraries/nif_library_probe";

/// The probe of the `zstd` contexts, which loads the pinned `zstd`.
const ZSTD_PROBE: &str = "fixtures/zstd/zstd_probe";

/// The probe of the node table of native record definitions.
const RECORD_RELOAD_PROBE: &str = "fixtures/native_record/nr_reload";

/// The output of `nr_reload:main/0` on the pinned ERTS (OTP 29.1.1).
const RECORD_RELOAD_OUTPUT: &str =
    "[{badrecord,{nr_def,pub}},1,{badrecord,{nr_def,pub}},[{badfield,{{nr_def,pub},a}},3],1]\n";

#[test]
fn reads_remote_record_definitions_of_the_current_code_as_pinned_erts() {
    // A remote record reads the definition of the current code
    // (`erl_record.c:344-379`): none before the load, the definition that
    // the load puts (`asm_load.c:1250-1256`), and none after
    // `code:delete/1` (`beam_bif_load.c:2451`, `erl_record.c:194-202`), and
    // the definitions of the new version after `erlang:load_module/2`. A
    // record value keeps its own definition.
    let flags = ["--pa", "fixtures/native_record"];
    let erts = run(&mut erts_probe(
        RECORD_RELOAD_PROBE,
        "nr_reload",
        "main",
        &flags,
    ));
    assert_eq!(erts, RECORD_RELOAD_OUTPUT);
    assert_eq!(
        run(&mut pbeam_probe(
            RECORD_RELOAD_PROBE,
            "nr_reload",
            "main",
            &flags
        )),
        RECORD_RELOAD_OUTPUT
    );
}

/// The code path directory of the runs. It has `code_server_target.beam`,
/// which no run loads at its start.
const CODE_PATH: &str = "fixtures/code_server";

/// The pinned OTP modules that the run needs in addition to `BOOT_MODULES`:
/// `code` and `code_server`, the modules that `code:do_start/0` loads
/// before it starts `code_server` and that `BOOT_MODULES` does not have
/// (code.erl:1699-1712), and the modules that `io:format/2` with `~p` calls.
const CODE_SERVER_MODULES: [&str; 12] = [
    "code",
    "code_server",
    "beam_lib",
    "binary",
    "ets",
    "filename",
    "gb_sets",
    "gb_trees",
    "io_lib_pretty",
    "erl_scan",
    "erl_features",
    "epp",
];

/// The pinned OTP modules that the kernel start of `pbeam_boot` calls in
/// addition to `BOOT_MODULES` and `CODE_SERVER_MODULES`: the
/// `application_master` of `kernel`, the `kernel_sup` children and the
/// modules that they call, `c` of `c:erlangrc/0`, and `prim_inet`, a
/// preloaded module of ERTS. On Linux, `inet_config:init/0` of the
/// `inet_db` start calls `inet:gethostbyname/1` for the host name when the
/// domain is empty (inet_config.erl:112-134), and the `native` lookup calls
/// `inet_gethost_native` (inet.erl:3813). In embedded mode, `code_server`
/// loads no module on demand (code.erl:601-602), so the run must hold them,
/// as the `primLoad` commands of `start_clean.boot` load them in ERTS
/// (init.erl:1335-1346).
const KERNEL_START_MODULES: [&str; 21] = [
    "application_master",
    "c",
    "erl_distribution",
    "erpc",
    "file",
    "global",
    "global_group",
    "inet",
    "inet_config",
    "inet_db",
    "inet_gethost_native",
    "inet_parse",
    "inet_udp",
    "kernel",
    "kernel_config",
    "kernel_refc",
    "net_kernel",
    "peer",
    "prim_inet",
    "rand",
    "rpc",
];

#[test]
fn starts_code_server_and_loads_a_module_from_the_code_path_as_pinned_erts() {
    // With `--root`, the kernel start of `pbeam_boot` runs
    // `code:start_link/0` as the first `kernel_sup` child
    // (kernel.erl:110-115), and the boot first sets the code path of the
    // last `path` command of `start_clean.boot` (init.erl:1323-1332).
    // `code_server` makes the interactive path of `Root/lib`
    // (code_server.erl:91-125, 513-535). `code:ensure_loaded/1` of a module
    // that is not loaded asks `code_server` for its object code, prepares
    // it, and lets `code_server` finish the load (code.erl:581-613;
    // code_server.erl:1112-1143, 1303-1384). A module that no path
    // directory has gives `{error,nofile}`. `code:which/1` gives the file of
    // a module that `code_server` loaded, `preloaded` for a module of
    // `erlang:pre_loaded/0`, and else the first file on the path
    // (code.erl:1782-1789). The ERTS run gives `-boot start_clean`, so that
    // its boot path is that of `pbeam_boot`, and loads the probe with
    // `code:load_abs/1`, so that `code_server` has no other module of the
    // code path.
    let erts = run(&mut erts(&[]));
    assert!(
        erts.starts_with(
            "[interactive,\n [false,\n  {module,code_server_target},\n  true,\n  \
             {module,code_server_target},\n  {error,nofile}],\n code_server_target_value,\n"
        ),
        "{erts}"
    );
    assert_eq!(run(&mut pbeam(&[])), erts);
}

#[test]
fn loads_a_module_with_an_on_load_function_through_code_server_as_pinned_erts() {
    // `code:ensure_loaded/1` loads `on_load_ok`, and a call loads
    // `on_load_call` on demand through `error_handler`
    // (error_handler.erl:85-98; code.erl:581-600). `erlang:finish_loading/1`
    // gives `{on_load, [Module]}`, `code_server` runs the function in a new
    // process, and `erts_code_purger` makes the code current
    // (code_server.erl:1348-1365, 1386-1403; erts_code_purger.erl:166-170).
    let erts = run(&mut erts_probe(
        ON_LOAD_PROBE,
        "on_load_server_probe",
        "main",
        &[],
    ));
    assert_eq!(
        erts,
        "[[false,{module,on_load_ok},true,on_load_ok_value,{module,on_load_ok}],\n \
         [false,on_load_call_value,true]]\n"
    );
    assert_eq!(
        run(&mut pbeam_probe(
            ON_LOAD_PROBE,
            "on_load_server_probe",
            "main",
            &[]
        )),
        erts
    );
}

#[test]
fn loads_crypto_and_runs_strong_rand_bytes_as_pinned_erts() {
    // The first call loads `crypto` on demand. Its `-on_load` function
    // checks `erlang:system_info(build_type)` and loads the NIF library with
    // the load info `{302, LibBin, false}` (crypto.erl:4432-4510).
    // `strong_rand_bytes_nif/1` gives a binary of the size, and `badarg`
    // with the NIF frame for a size that is not an unsigned C int or that
    // is above `INT_MAX` (rand.c:26-50).
    let erts = run(&mut erts_probe(CRYPTO_PROBE, "crypto_probe", "main", &[]));
    assert_eq!(
        erts,
        "[opt,<<>>,\n \
         [0,1,16,1024],\n \
         [true,true],\n \
         [{error,badarg,{crypto,strong_rand_bytes_nif,[-1],[]}},\n  \
         {error,badarg,{crypto,strong_rand_bytes_nif,[2147483648],[]}},\n  \
         {error,badarg,{crypto,strong_rand_bytes_nif,[18446744073709551616],[]}},\n  \
         {error,badarg,{crypto,strong_rand_bytes_nif,[1.0],[]}},\n  \
         {error,badarg,{crypto,strong_rand_bytes_nif,[a],[]}},\n  \
         {error,badarg,{crypto,strong_rand_bytes_nif,[<<1>>],[]}}]]\n"
    );
    assert_eq!(
        run(&mut pbeam_probe(CRYPTO_PROBE, "crypto_probe", "main", &[])),
        erts
    );
}

#[test]
fn checks_the_crypto_load_info_as_pinned_erts() {
    // The stand-in `crypto` module defines each function of the `crypto`
    // NIF library and loads it with each load info. A failed check of the
    // load callback gives its line in crypto.c (crypto.c:204-218, 286-288;
    // erl_nif.c:4980-4986). The pinned ERTS has no FIPS support, so
    // `FipsMode` `true` gives the veto 287 (fips.c:59-60).
    let veto = |line| format!("{{error,{{load,\"Library load-call unsuccessful ({line}).\"}}}}");
    let vetoes: Vec<_> = [205, 208, 211, 214, 217, 287].map(veto).into();
    let expected = format!("[{}]\n", vetoes.join(",\n "));
    let fips = format!("[{}]\n", veto(287));
    for (function, expected) in [
        ("load_info", expected.as_str()),
        ("load_ok", "[ok]\n"),
        ("load_fips", fips.as_str()),
    ] {
        let erts = run(&mut erts_probe(CRYPTO_STAND_IN, "crypto", function, &[]));
        assert_eq!(erts, expected);
        let pbeam = run(&mut pbeam_probe(CRYPTO_STAND_IN, "crypto", function, &[]));
        assert_eq!(pbeam, erts);
    }
}

#[test]
fn checks_the_load_info_of_the_libraries_without_bindings_as_pinned_erts() {
    // Each stand-in defines the functions of the `nif_funcs` table of its
    // library and loads the library with each load info. The load callback
    // of `prim_socket` needs a map with a local pid under `registry`
    // (prim_socket_nif.c:18862-18867), and the load callback of `asn1rt_nif`
    // needs the integer 1 (asn1_erl_nif.c:1320-1329). A table entry that the
    // stand-in does not define gives `Function not found` on one side.
    let reload =
        "{error,{reload,\"NIF library already loaded (reload disallowed since OTP 20).\"}}";
    let veto = |veto| format!("{{error,{{load,\"Library load-call unsuccessful ({veto}).\"}}}}");
    let socket = [
        veto(1),
        veto(1),
        veto(1),
        "ok".to_owned(),
        reload.to_owned(),
    ];
    let asn1 = [
        &[veto(-1), veto(-1), veto(-1), veto(-1), veto(-1)][..],
        &["ok".to_owned(), reload.to_owned()],
    ]
    .concat();
    let expected = format!(
        "{{zstd,[ok,{reload}]}}\n{{prim_socket,[{}]}}\n{{prim_net,[ok,{reload}]}}\n\
         {{erl_tracer,[ok,{reload}]}}\n{{asn1rt_nif,[{}]}}\n",
        socket.join(","),
        asn1.join(","),
    );
    let flat = |text: String| text.split_whitespace().collect::<Vec<_>>().join("");
    let erts = run(&mut erts_probe(
        NIF_LIBRARY_PROBE,
        "nif_library_probe",
        "main",
        &[],
    ));
    assert_eq!(flat(erts.clone()), flat(expected));
    let pbeam = run(&mut pbeam_probe(
        NIF_LIBRARY_PROBE,
        "nif_library_probe",
        "main",
        &[],
    ));
    assert_eq!(pbeam, erts);
}

#[test]
fn loads_the_pinned_zstd_as_pinned_erts() {
    // The `on_load` function of the pinned `zstd` loads its library and
    // matches `ok` on the result (zstd.erl:251-252).
    let erts = run(&mut erts_probe(
        NIF_LIBRARY_PROBE,
        "nif_library_probe",
        "pinned",
        &[],
    ));
    assert_eq!(erts, "{module,zstd}\n");
    let pbeam = run(&mut pbeam_probe(
        NIF_LIBRARY_PROBE,
        "nif_library_probe",
        "pinned",
        &[],
    ));
    assert_eq!(pbeam, erts);
}

#[test]
fn stops_with_a_runtime_error_for_a_nif_of_the_pinned_zstd() {
    // `zstd:get_frame_header/1` calls `get_frame_header_nif/1`
    // (zstd.erl:433-435), which has no binding.
    let output = pbeam_probe(NIF_LIBRARY_PROBE, "nif_library_probe", "frame_header", &[])
        .env("TERM", "dumb")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        !output.status.success()
            && stderr.contains("zstd:get_frame_header_nif/1 NIF is not supported"),
        "{stderr}"
    );
}

#[test]
fn runs_the_zstd_contexts_as_pinned_erts() {
    // Each line is one case of the probe: its label and the hex text of the
    // external term format of its result. The results hold the compressed
    // and decompressed bytes, the iodata shape of each output, and each
    // exception class and reason (zstd_nif.c:100-827). libzstd 1.5.7 of
    // `zstd-sys` is the version that pinned ERTS vendors, so the compressed
    // bytes are the same.
    let erts = run(&mut erts_probe(ZSTD_PROBE, "zstd_probe", "main", &[]));
    let pbeam = run(&mut pbeam_probe(ZSTD_PROBE, "zstd_probe", "main", &[]));
    assert_eq!(pbeam.lines().count(), erts.lines().count());
    for (pbeam, erts) in pbeam.lines().zip(erts.lines()) {
        let label = erts.rsplit_once(' ').unwrap().0;
        assert!(pbeam == erts, "the case {label} differs");
    }
}

#[test]
fn stops_with_a_runtime_error_for_each_unsupported_zstd_call() {
    // The dictionary functions and the parameter reads have no binding, and
    // the safe API of libzstd cannot set the `default` strategy again.
    for (function, expected) in [
        (
            "unsupported_dictionary",
            "zstd:load_compress_dictionary_nif/2 NIF is not supported",
        ),
        (
            "unsupported_get_parameter",
            "zstd:get_compress_parameter_nif/2 NIF is not supported",
        ),
        (
            "unsupported_default_strategy",
            "zstd:set_compress_parameter_nif/3 cannot set the default strategy after another \
             strategy",
        ),
    ] {
        let output = pbeam_probe(ZSTD_PROBE, "zstd_probe", function, &[])
            .env("TERM", "dumb")
            .stdin(Stdio::null())
            .output()
            .unwrap();
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(
            !output.status.success() && stderr.contains(expected),
            "{function}: {stderr}"
        );
    }
}

#[test]
fn runs_the_linked_crypto_nifs_as_pinned_erts() {
    // PBeam links hash_nif/2, pbkdf2_hmac_nif/5, strong_rand_range_nif/1,
    // info_nif/0, and info_lib/0 (hash.c:101-164, pbkdf2_hmac.c:70-100,
    // rand.c:54-85, info.c:110-204). The probe prints only the shape of a
    // random value, and it prints the pinned OpenSSL version that the
    // pinned ERTS reports.
    let erts = run(&mut erts_probe(CRYPTO_PROBE, "crypto_probe", "nifs", &[]));
    assert_eq!(
        run(&mut pbeam_probe(CRYPTO_PROBE, "crypto_probe", "nifs", &[])),
        erts
    );
    // The pinned ERTS links a different libcrypto on macOS and Linux, so the
    // version fields and the link type stay out of this comparison. The unit
    // test checks the full pinned values of a PBeam build.
    let erts = run(&mut erts_probe(CRYPTO_PROBE, "crypto_probe", "info", &[]));
    assert_eq!(
        run(&mut pbeam_probe(CRYPTO_PROBE, "crypto_probe", "info", &[])),
        erts
    );
}

#[test]
fn runs_the_one_shot_crypto_hashes_and_macs_as_pinned_erts() {
    // hash_info/1, hash_nif/2 for each digest, mac_nif/4 for hmac, cmac,
    // and poly1305, and hash_equals_nif/2 (hash.c:74-164, mac.c:188-505,
    // hash_equals.c:27-53), with the error tuple of each failed check.
    let erts = run(&mut erts_probe(
        CRYPTO_PROBE,
        "crypto_probe",
        "oneshot",
        &[],
    ));
    assert!(erts.contains("Couldn't get mac"), "{erts}");
    // md4 and cmac with a cipher that is not CBC depend on the libcrypto of
    // the host (digest.c:186-188, mac.c:344-347, 400-405). PBeam links a
    // vendored OpenSSL 3.5.5 without the legacy provider, so its output does
    // not depend on the host. The literal is the output of the pinned macOS
    // ERTS (OpenSSL 3.5.8); the Linux CI ERTS links a libcrypto with md4,
    // the legacy ciphers, and cmac for each mode.
    let pbeam = run(&mut pbeam_probe(
        CRYPTO_PROBE,
        "crypto_probe",
        "oneshot_host",
        &[],
    ));
    assert_eq!(pbeam, ONESHOT_HOST);
    if cfg!(target_os = "macos") {
        let erts = run(&mut erts_probe(
            CRYPTO_PROBE,
            "crypto_probe",
            "oneshot_host",
            &[],
        ));
        assert_eq!(erts, ONESHOT_HOST);
    }
    assert_eq!(
        run(&mut pbeam_probe(
            CRYPTO_PROBE,
            "crypto_probe",
            "oneshot",
            &[]
        )),
        erts
    );
    // A fetched SHAKE md gives 16/32 on OpenSSL 3.0-3.3 and 0 from 3.4
    // (hash.c:90-94, evp_lib.c:1041 in 3.5.5). The pinned macOS ERTS gives
    // 0; the Linux CI ERTS links OpenSSL 3.0.x.
    let erts = run(&mut erts_probe(
        CRYPTO_PROBE,
        "crypto_probe",
        "xof_sizes",
        &[],
    ));
    assert!(matches!(erts.as_str(), "[0,0]\n" | "[16,32]\n"), "{erts}");
    assert_eq!(
        run(&mut pbeam_probe(
            CRYPTO_PROBE,
            "crypto_probe",
            "xof_sizes",
            &[]
        )),
        "[0,0]\n"
    );
}

#[test]
fn runs_the_one_shot_crypto_ciphers_as_pinned_erts() {
    // cipher_info_nif/1, ng_crypto_one_time_nif/5 with each padding,
    // aead_cipher_nif/7, and do_exor/2 for each cipher that the OpenSSL 3
    // default provider has (cipher.c:266-371, api_ng.c:91-864,
    // aead.c:150-377, math.c:25-54), with the error tuple of each failed
    // check.
    let erts = run(&mut erts_probe(
        CRYPTO_PROBE,
        "crypto_probe",
        "ciphers",
        &[],
    ));
    assert!(erts.contains("Bad tag in option"), "{erts}");
    // cipher_algorithms/0 lists the ciphers in the order of their atom
    // indexes (cipher.c:239, 395-416), which is the same on each run.
    let order = run(&mut erts_probe(
        CRYPTO_PROBE,
        "crypto_probe",
        "cipher_order",
        &[],
    ));
    assert_eq!(
        run(&mut pbeam_probe(
            CRYPTO_PROBE,
            "crypto_probe",
            "cipher_order",
            &[]
        )),
        order
    );
    assert_eq!(
        run(&mut pbeam_probe(
            CRYPTO_PROBE,
            "crypto_probe",
            "ciphers",
            &[]
        )),
        erts
    );
    // The ERTS of the Linux CI host links a libcrypto with the legacy
    // ciphers, so only the pinned macOS ERTS (OpenSSL 3.5.8) is the
    // reference for rc2, rc4, des, and blowfish (cipher.c:216-218). It also has
    // no sm4_gcm and sm4_ccm.
    let pbeam = run(&mut pbeam_probe(
        CRYPTO_PROBE,
        "crypto_probe",
        "ciphers_host",
        &[],
    ));
    assert_eq!(pbeam, CIPHERS_HOST);
    if cfg!(target_os = "macos") {
        let erts = run(&mut erts_probe(
            CRYPTO_PROBE,
            "crypto_probe",
            "ciphers_host",
            &[],
        ));
        assert_eq!(erts, CIPHERS_HOST);
    }
}

#[test]
fn gives_the_crypto_algorithm_lists_and_fips_status_as_pinned_erts() {
    // The pinned ERTS lists each algorithm of its OpenSSL: 187 on macOS
    // and 177 with the libcrypto of the Linux CI host (algorithms.c:86-224,
    // digest.c:226-242, crypto.erl:843-895). PBeam lists only the 18 digests
    // and 3 MACs that it computes, in the ERTS order, so the items that
    // PBeam can run are the same.
    let erts = run(&mut erts_probe(
        CRYPTO_PROBE,
        "crypto_probe",
        "supports",
        &[],
    ));
    let pbeam = run(&mut pbeam_probe(
        CRYPTO_PROBE,
        "crypto_probe",
        "supports",
        &[],
    ));
    let items = "[{hashs,[blake2s,blake2b,sm3,shake256,shake128,sha3_512,sha3_384,sha3_256,\n          \
                 sha3_224,sha512_256,sha512_224,sha512,sha384,sha256,sha224,sha,\n          \
                 ripemd160,md5]},\n  \
                 {ciphers,[aes_128_cbc,aes_128_ccm,aes_128_cfb128,aes_128_cfb8,aes_128_ctr,\n            \
                 aes_128_ecb,aes_128_gcm,aes_128_ofb,aes_192_cbc,aes_192_ccm,\n            \
                 aes_192_cfb128,aes_192_cfb8,aes_192_ctr,aes_192_ecb,aes_192_gcm,\n            \
                 aes_192_ofb,aes_256_cbc,aes_256_ccm,aes_256_cfb128,aes_256_cfb8,\n            \
                 aes_256_ctr,aes_256_ecb,aes_256_gcm,aes_256_ofb,aes_cbc,aes_ccm,\n            \
                 aes_cfb128,aes_cfb8,aes_ctr,aes_ecb,aes_gcm,chacha20,\n            \
                 chacha20_poly1305,des_ede3_cbc,des_ede3_cfb,sm4_cbc,sm4_cfb,\n            \
                 sm4_ctr,sm4_ecb,sm4_ofb]},\n  \
                 {kems,[]},\n  \
                 {public_keys,[]},\n  \
                 {macs,[cmac,hmac,poly1305]},\n  \
                 {curves,[]},\n  \
                 {rsa_opts,[]}]";
    let rest = format!(",\n {items},\n false,true]\n");
    let count = erts
        .strip_suffix(rest.as_str())
        .and_then(|head| head.strip_prefix('['));
    assert!(matches!(count, Some("187" | "177")), "{erts}");
    assert_eq!(pbeam, format!("[63,\n {items},\n true,true]\n"));
    // A build without FIPS support (fips.c:27-62).
    let erts = run(&mut erts_probe(CRYPTO_PROBE, "crypto_probe", "fips", &[]));
    assert_eq!(erts, "[not_supported,false,true,{error,badarg}]\n");
    assert_eq!(
        run(&mut pbeam_probe(CRYPTO_PROBE, "crypto_probe", "fips", &[])),
        erts
    );
}

#[test]
fn runs_the_crypto_hash_contexts_as_pinned_erts() {
    // hash_init_nif/1, hash_update_nif/2, hash_final_nif/1, and
    // hash_final_xof_nif/2 (hash.c:168-273, 561-589): a context stays usable
    // after an update and after a final call, and crypto:hash/2 sends more
    // than 20000 bytes through the contexts (crypto.erl:4535-4540).
    let erts = run(&mut erts_probe(
        CRYPTO_PROBE,
        "crypto_probe",
        "contexts",
        &[],
    ));
    assert!(erts.contains("EVP_DigestFinalXOF failed"), "{erts}");
    assert_eq!(
        run(&mut pbeam_probe(
            CRYPTO_PROBE,
            "crypto_probe",
            "contexts",
            &[]
        )),
        erts
    );
}

#[test]
fn runs_the_cipher_context_nifs_as_pinned_erts() {
    // ng_crypto_init_nif/4, ng_crypto_update_nif/2, ng_crypto_final_nif/1,
    // ng_crypto_get_data_nif/1 (api_ng.c:91-892), aead_cipher_init_nif/4,
    // and aead_cipher_nif/4 (aead.c:69-377), with the error tuple of each
    // failed check.
    let erts = run(&mut erts_probe(
        CRYPTO_PROBE,
        "crypto_probe",
        "cipher_contexts",
        &[],
    ));
    assert!(erts.contains("Expected cipher name atom"), "{erts}");
    let pbeam = run(&mut pbeam_probe(
        CRYPTO_PROBE,
        "crypto_probe",
        "cipher_contexts",
        &[],
    ));
    assert_eq!(pbeam, erts);
}

#[test]
fn runs_the_mac_context_nifs_as_pinned_erts() {
    let erts = run(&mut erts_probe(
        CRYPTO_PROBE,
        "crypto_probe",
        "mac_contexts",
        &[],
    ));
    assert!(erts.contains("Can't initialize EVP_MAC_CTX"), "{erts}");
    // The end of an HMAC context, md4, des_cbc, and CMAC with a cipher that
    // is not CBC depend on the libcrypto of the host. PBeam links a vendored
    // OpenSSL 3.5.5, so its output does not depend on the host; the Linux CI
    // ERTS links a libcrypto that takes each of them.
    let host = run(&mut pbeam_probe(
        CRYPTO_PROBE,
        "crypto_probe",
        "mac_contexts_host",
        &[],
    ));
    assert_eq!(host, MAC_CONTEXTS_HOST);
    if cfg!(target_os = "macos") {
        let erts_host = run(&mut erts_probe(
            CRYPTO_PROBE,
            "crypto_probe",
            "mac_contexts_host",
            &[],
        ));
        assert_eq!(erts_host, MAC_CONTEXTS_HOST);
    }
    assert_eq!(
        run(&mut pbeam_probe(
            CRYPTO_PROBE,
            "crypto_probe",
            "mac_contexts",
            &[]
        )),
        erts
    );
}

#[test]
fn stops_with_a_runtime_error_for_a_poly1305_context_after_its_final() {
    // The pinned ERTS stops: `Poly1305_Final` clears the context.
    for (function, name) in [
        ("mac_poly1305_final", "mac_final_nif/1"),
        ("mac_poly1305_update", "mac_update_nif/2"),
    ] {
        let output = pbeam_probe(CRYPTO_PROBE, "crypto_probe", function, &[])
            .env("TERM", "dumb")
            .stdin(Stdio::null())
            .output()
            .unwrap();
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(
            !output.status.success()
                && stderr.contains(&format!(
                    "crypto:{name} on a Poly1305 context after its final is not supported"
                )),
            "{stderr}"
        );
    }
}

#[test]
fn stops_with_a_runtime_error_for_an_unsupported_crypto_nif() {
    // PBeam does not link `privkey_to_pubkey_nif/2`, which
    // `crypto:privkey_to_pubkey/2` calls.
    let output = pbeam_probe(CRYPTO_PROBE, "crypto_probe", "unsupported", &[])
        .env("TERM", "dumb")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        !output.status.success()
            && stderr.contains("crypto:privkey_to_pubkey_nif/2 NIF is not supported"),
        "{stderr}"
    );
}

#[test]
fn loads_an_on_load_module_through_a_hibernated_erts_code_purger_as_pinned_erts() {
    // After 60 seconds with no request, `erts_code_purger` calls
    // `erlang:hibernate(erts_code_purger, wait_for_request, [])`
    // (erts_code_purger.erl:40-45). The probe does not wait 60 seconds: it
    // starts a process that makes the same call, lets it run to the call with
    // one scheduler, and registers it as `erts_code_purger`. `code_server` then
    // sends `finish_after_on_load` to it, and the process wakes and replies
    // (erts_code_purger.erl:55-59, 166-170).
    let erts = run(&mut erts_probe(
        ON_LOAD_PROBE,
        "on_load_server_probe",
        "hibernated",
        &[],
    ));
    assert_eq!(erts, "[{module,on_load_ok},on_load_ok_value,true]\n");
    assert_eq!(
        run(&mut pbeam_probe(
            ON_LOAD_PROBE,
            "on_load_server_probe",
            "hibernated",
            &[]
        )),
        erts
    );
}

#[test]
fn purges_the_code_of_a_failed_on_load_function_as_pinned_erts() {
    // `erts_code_purger` purges the code of a failed `-on_load` function with
    // `erts_internal:purge_module/2`, and the load gives
    // `{error, on_load_failure}` (code_server.erl:1360-1368;
    // erts_code_purger.erl:166-182). A later call loads the module again,
    // and its function fails again. The crash of the on_load process can
    // add an error report after the result on ERTS.
    let expected = "[{error,on_load_failure},false,{error,undef},false]\n";
    let erts = run(&mut erts_probe(
        ON_LOAD_PROBE,
        "on_load_server_probe",
        "failure",
        &[],
    ));
    assert!(erts.starts_with(expected), "{erts}");
    assert_eq!(
        run(&mut pbeam_probe(
            ON_LOAD_PROBE,
            "on_load_server_probe",
            "failure",
            &[]
        )),
        expected
    );
}

#[test]
fn kills_a_process_in_the_code_of_a_failed_on_load_function_as_pinned_erts() {
    // The `check_process_code` request finds the process in the code, and
    // `erts_code_purger` kills it before it purges the code
    // (erts_code_purger.erl:166-182, 249-265; beam_bif_load.c:1149-1185).
    let erts = run(&mut erts_probe(
        ON_LOAD_PROBE,
        "on_load_server_probe",
        "killed",
        &[],
    ));
    assert!(
        erts.starts_with("[{error,on_load_failure},false,false]\n"),
        "{erts}"
    );
    assert_eq!(
        run(&mut pbeam_probe(
            ON_LOAD_PROBE,
            "on_load_server_probe",
            "killed",
            &[]
        )),
        "[{error,on_load_failure},false,false]\n"
    );
}

#[test]
fn keeps_a_process_that_left_the_code_of_a_failed_on_load_function_as_pinned_erts() {
    // The on_load function of `on_load_survive` starts a process that leaves
    // the code of the module at once, so no frame of the process is in the
    // code. The `check_process_code` request answers `false` for the
    // process, and `erts_code_purger` does not kill it
    // (erts_code_purger.erl:166-182, 249-265). The process runs on after the
    // purge removed the code.
    let erts = run(&mut erts_probe(
        ON_LOAD_PROBE,
        "on_load_server_probe",
        "survived",
        &[],
    ));
    assert_eq!(erts, "[{error,on_load_failure},true,[3,2,1]]\n");
    assert_eq!(
        run(&mut pbeam_probe(
            ON_LOAD_PROBE,
            "on_load_server_probe",
            "survived",
            &[]
        )),
        erts
    );
}

#[test]
fn raises_badfun_for_the_funs_of_removed_code_as_pinned_erts() {
    // The -on_load function of the first version of fun_entry_send sends two
    // funs of its code to the probe and fails, so erts_code_purger removes
    // the code (code_server.erl:1360-1368). A load of the second version of
    // the same module name gives the entry of the first fun another old
    // unique value and leaves no entry for the index of the second fun, so
    // the call of each fun enters no code and raises error:{badfun, Fun}
    // (beam_common.c:1626-1630, 1697-1706). The name of a fun of code that
    // is not loaded is `[]` (`erl_bif_info.c:3998-4001`), also after the
    // code_server purged the module and before a load of another version.
    // `erlang:fun_info_mfa/1` gives the module, `[]`, and the arity of such
    // a fun (`erl_bif_info.c:4027-4035`).
    let expected = "[{error,{badfun,fun_term}},{error,{badfun,fun_term}},\
                    [{name,'-make_first/1-fun-0-'},\
                     {name,'-make_second/1-fun-0-'}],\
                    [{name,[]},{name,[]}],[{name,[]},{name,[]}],\
                    [{fun_entry_send,[],0},{fun_entry_send,[],0}]]\n";
    let erts = run(&mut erts_probe(
        FUN_ENTRY_PROBE,
        "fun_entry_probe",
        "stale",
        &[],
    ));
    assert!(erts.starts_with(expected), "{erts}");
    assert_eq!(
        run(&mut pbeam_probe(
            FUN_ENTRY_PROBE,
            "fun_entry_probe",
            "stale",
            &[]
        )),
        expected
    );
}

#[test]
fn reads_the_name_of_the_funs_of_deleted_code_as_pinned_erts() {
    // `erlang:delete_module/1` moves the code of the module to old code and
    // keeps its fun entries until the purge (`beam_bif_load.c:2412-2455`),
    // so `erlang:fun_info/2` gives the name of the function of a fun of that
    // code, as on ERTS (`erl_fun.c:37-46`).
    let expected = "[{name,'-make_rev/1-fun-0-'},{name,'-make_rev/1-fun-0-'},\
                    {name,'-make_rev/1-fun-0-'}]\n";
    let erts = run(&mut erts_probe(
        FUN_ENTRY_PROBE,
        "fun_entry_probe",
        "deleted",
        &[],
    ));
    assert!(erts.starts_with(expected), "{erts}");
    assert_eq!(
        run(&mut pbeam_probe(
            FUN_ENTRY_PROBE,
            "fun_entry_probe",
            "deleted",
            &[]
        )),
        expected
    );
}

#[test]
fn calls_the_funs_of_identical_reloaded_code_as_pinned_erts() {
    // A load of the code that made the funs, while the probe holds no
    // name, makes the code current again. The entry of each fun is then in
    // the loaded code, so the call enters the function of its lambda.
    let expected = "[{version_one,first},{version_one,second}]\n";
    let erts = run(&mut erts_probe(
        FUN_ENTRY_PROBE,
        "fun_entry_probe",
        "identical",
        &[],
    ));
    assert!(erts.starts_with(expected), "{erts}");
    assert_eq!(
        run(&mut pbeam_probe(
            FUN_ENTRY_PROBE,
            "fun_entry_probe",
            "identical",
            &[]
        )),
        expected
    );
}

#[test]
fn checks_the_arguments_of_the_bifs_of_erts_code_purger_as_pinned_erts() {
    // The argument checks and results of `erts_internal:purge_module/2`
    // from a process that is not `erts_code_purger`,
    // `erts_internal:request_system_task/3`, `erts_internal:processes_next/1`,
    // and `erlang:system_info(outstanding_system_requests_limit)`
    // (beam_bif_load.c:2090-2092; erl_process.c:11309-11492;
    // erl_ptab.c:1472-1534; erl_init.c:1354).
    let erts = run(&mut erts_probe(
        ON_LOAD_PROBE,
        "on_load_server_probe",
        "system_tasks",
        &[],
    ));
    assert!(erts.contains("[none,true,2],\n {error,notsup}]"), "{erts}");
    assert_eq!(
        run(&mut pbeam_probe(
            ON_LOAD_PROBE,
            "on_load_server_probe",
            "system_tasks",
            &[]
        )),
        erts
    );
}

#[test]
fn checks_the_arguments_of_erts_internal_purge_module_from_erts_code_purger() {
    // PBeam only: PBeam lets the process registered as `erts_code_purger`
    // call the BIF, and the probe takes the name. The results follow
    // `erts_internal_purge_module_2` (beam_bif_load.c:2094-2098, 2108-2110,
    // 2108-2110, 2122-2126, 2231-2233, 2328-2336). The last load runs the on_load
    // function through the real `erts_code_purger` again.
    assert_eq!(
        run(&mut pbeam_probe(
            ON_LOAD_PROBE,
            "on_load_server_probe",
            "fake_purger",
            &[]
        )),
        "[[{on_load,[on_load_error]},\n  true,\n  {error,badarg},\n  {error,badarg},\n  \
         {error,badarg},\n  false,true,\n  {error,badarg},\n  {error,badarg},\n  {error,badarg},\n  true,\n  \
         {error,badarg},\n  false],\n {error,on_load_failure}]\n"
    );
}

#[test]
fn stops_for_the_purge_operations_that_pbeam_does_not_support() {
    for (function, message) in [
        (
            "purge_loaded",
            "pbeam: erts_internal:purge_module/2 found no on_load code of lists\n",
        ),
        (
            "purge_prepare",
            "pbeam: unsupported erts_internal:purge_module/2 operation prepare\n",
        ),
        (
            "gc_task",
            "pbeam: unsupported erts_internal:request_system_task/3 request garbage_collect\n",
        ),
    ] {
        let output = pbeam_probe(ON_LOAD_PROBE, "on_load_server_probe", function, &[])
            .env("TERM", "dumb")
            .stdin(Stdio::null())
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success() && stderr == message, "{output:?}");
    }
}

#[test]
fn starts_code_server_in_embedded_mode_as_pinned_erts() {
    // With `--mode embedded`, `code:ensure_loaded/1` of a module that is not
    // loaded returns `{error,embedded}` (code.erl:601-602), and the path is
    // the `erl_prim_loader` path (code_server.erl:112-114, 519-520).
    let flags = ["--mode", "embedded"];
    let erts = run(&mut erts(&flags));
    assert!(
        erts.starts_with(
            "[embedded,\n [false,{error,embedded},false,{error,embedded},{error,embedded}],\n \
             not_loaded,\n"
        ),
        "{erts}"
    );
    assert_eq!(run(&mut pbeam(&flags)), erts);
}

#[test]
fn stops_when_the_start_modules_do_not_hold_the_atom_of_a_boot_flag() {
    // The binary makes the boot flag list from the atoms of the loaded
    // modules, so a flag name that no loaded module holds stops the run
    // before it starts (row "Run arguments" of docs/erts-differences.md).
    // The pinned `init` module holds the atom `mode` (init.erl:938, 1206).
    // Without it, `--mode` stops the run. A start set of this test has only
    // `pbeam_boot` and the probe, because the other pins of the boot would
    // put the atom in the table.
    let output = Command::new(env!("CARGO_BIN_EXE_pbeam"))
        .args([
            "pbeam_boot",
            "start",
            "code_server_probe",
            "main",
            "--root",
            "/pbeam-test-root-that-does-not-exist",
            "--mode",
            "embedded",
            "--",
        ])
        .arg(format!("{CODE_SERVER_PROBE}.beam"))
        .arg(boot_beam())
        .env("TERM", "dumb")
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "pbeam: boot flag mode is not an atom of the loaded modules\n"
    );
}

/// Return the command of the probe on the pinned ERTS with the code path
/// directory and the `erl` flag of each `--mode` flag.
///
/// # Examples
///
/// ```rust,ignore
/// let output = run(&mut erts(&["--mode", "embedded"]));
/// ```
fn erts(flags: &[&str]) -> Command {
    erts_probe(CODE_SERVER_PROBE, "code_server_probe", "main", flags)
}

/// The output of `crypto_probe:mac_contexts_host/0` on PBeam and on the
/// pinned macOS ERTS (OpenSSL 3.5.8).
const MAC_CONTEXTS_HOST: &str = "[[{error,{error,{\"mac.c\",840},\"mac update\"}},
  {error,{error,{\"mac.c\",879},\"Signing\"}},
  {error,{error,{\"mac.c\",879},\"Signing\"}}],
 true,
 [{error,{error,{\"mac.c\",750},\"Can't initialize EVP_MAC_CTX\"}},
  {error,{notsup,{\"mac.c\",688},\"Unsupported cipher algorithm\"}},
  {error,{error,{\"mac.c\",750},\"Can't initialize EVP_MAC_CTX\"}},
  {error,{error,{\"mac.c\",750},\"Can't initialize EVP_MAC_CTX\"}}]]
";

/// Return the command of `module:function()` of the probe file `probe` on
/// the pinned ERTS, as [`erts`] gives it.
///
/// # Examples
///
/// ```rust,ignore
/// let output = run(&mut erts_probe(ON_LOAD_PROBE, "on_load_server_probe", "main", &[]));
/// ```
/// The output of `crypto_probe:oneshot_host/0` on the pinned macOS ERTS.
#[test]
fn signs_and_verifies_as_pinned_erts() {
    // pkey_sign_nif/5 and pkey_verify_nif/6 (pkey.c:212-1122) with RSA,
    // DSA, ECDSA, and EdDSA keys, and the error tuple of each failed check.
    let erts = run(&mut erts_probe(
        CRYPTO_SIGN_PROBE,
        "crypto_sign_probe",
        "main",
        &[],
    ));
    assert!(erts.contains("Only RSA supports Options"), "{erts}");
    assert_eq!(
        run(&mut pbeam_probe(
            CRYPTO_SIGN_PROBE,
            "crypto_sign_probe",
            "main",
            &[]
        )),
        erts
    );
    // md4 is in the legacy provider of OpenSSL 3, and EVP_DigestFinal_ex
    // fails for a SHAKE digest from OpenSSL 3.4 (digest.c:186-188,
    // pkey.c:283-284, 366-367). PBeam links a vendored OpenSSL 3.5.5
    // without the legacy provider; the literal is the output of the pinned
    // macOS ERTS (OpenSSL 3.5.8). The Linux CI ERTS links OpenSSL 3.0.x.
    let host = "[{error,{badarg,{\"pkey.c\",284},\"Digest type not supported\"}},\n \
                {error,{error,{\"pkey.c\",367},\"Can't create EVP_DigestFinal_ex\"}}]\n";
    assert_eq!(
        run(&mut pbeam_probe(
            CRYPTO_SIGN_PROBE,
            "crypto_sign_probe",
            "host",
            &[]
        )),
        host
    );
    if cfg!(target_os = "macos") {
        let erts = run(&mut erts_probe(
            CRYPTO_SIGN_PROBE,
            "crypto_sign_probe",
            "host",
            &[],
        ));
        assert_eq!(erts, host);
    }
}

/// The output of `crypto_sign_probe:post_quantum/0` on the pinned macOS
/// ERTS (OpenSSL 3.5.8). The Linux CI ERTS links OpenSSL 3.0.x, which has no
/// ML-DSA or SLH-DSA (`pkey.c:90-172`).
const POST_QUANTUM_HOST: &str = r#"{[{mldsa44,2420,true,true,false},
  {mldsa65,3309,true,true,false},
  {mldsa87,4627,true,true,false}],
 [{slh_dsa_shake_128s,7856,true,true,false},
  {slh_dsa_shake_128s,7856,true,true,false},
  {slh_dsa_shake_128f,17088,true,true,false},
  {slh_dsa_shake_128f,17088,true,true,false},
  {slh_dsa_sha2_128s,7856,true,true,false},
  {slh_dsa_sha2_128s,7856,true,true,false},
  {slh_dsa_sha2_128f,17088,true,true,false},
  {slh_dsa_sha2_128f,17088,true,true,false},
  {slh_dsa_shake_192s,16224,true,true,false},
  {slh_dsa_shake_192s,16224,true,true,false},
  {slh_dsa_shake_192f,35664,true,true,false},
  {slh_dsa_shake_192f,35664,true,true,false},
  {slh_dsa_sha2_192s,16224,true,true,false},
  {slh_dsa_sha2_192s,16224,true,true,false},
  {slh_dsa_sha2_192f,35664,true,true,false},
  {slh_dsa_sha2_192f,35664,true,true,false},
  {slh_dsa_shake_256s,29792,true,true,false},
  {slh_dsa_shake_256s,29792,true,true,false},
  {slh_dsa_shake_256f,49856,true,true,false},
  {slh_dsa_shake_256f,49856,true,true,false},
  {slh_dsa_sha2_256s,29792,true,true,false},
  {slh_dsa_sha2_256s,29792,true,true,false},
  {slh_dsa_sha2_256f,49856,true,true,false},
  {slh_dsa_sha2_256f,49856,true,true,false}],
 [{badarg,{"pkey.c",552},"Seed key not allowed"},
  {badarg,{"pkey.c",556},"Invalid key tuple format"},
  {error,{"pkey.c",649},"Can't create PKEY fromdata"},
  {error,{"pkey.c",621},"Invalid public key"},
  {error,{"pkey.c",649},"Can't create PKEY fromdata"},
  {error,{"pkey.c",621},"Invalid public key"},
  {error,{"pkey.c",649},"Can't create PKEY fromdata"},
  {error,{"pkey.c",621},"Invalid public key"},
  false,
  {badarg,{"pkey.c",417},"Only RSA supports Options"}],
 [true,true,true]}
"#;

#[test]
fn signs_and_verifies_with_ml_dsa_and_slh_dsa() {
    assert_eq!(
        run(&mut pbeam_probe(
            CRYPTO_SIGN_PROBE,
            "crypto_sign_probe",
            "post_quantum",
            &[]
        )),
        POST_QUANTUM_HOST
    );
    if cfg!(target_os = "macos") {
        let erts = run(&mut erts_probe(
            CRYPTO_SIGN_PROBE,
            "crypto_sign_probe",
            "post_quantum",
            &[],
        ));
        assert_eq!(erts, POST_QUANTUM_HOST);
    }
}

#[test]
fn encrypts_and_decrypts_as_pinned_erts() {
    // pkey_crypt_nif/6 (pkey.c:1173-1581) with RSA keys, and the error
    // tuple of each failed check.
    let erts = run(&mut erts_probe(
        CRYPTO_CRYPT_PROBE,
        "crypto_crypt_probe",
        "main",
        &[],
    ));
    assert!(
        erts.contains("Expect only two-tuples in the list"),
        "{erts}"
    );
    assert_eq!(
        run(&mut pbeam_probe(
            CRYPTO_CRYPT_PROBE,
            "crypto_crypt_probe",
            "main",
            &[]
        )),
        erts
    );
    // The provider of the host libcrypto decides these results: X9.31
    // padding for a sign, the implicit rejection of a bad PKCS #1 v1.5
    // cipher text (OpenSSL 3.2 and later), and the operations of the DSA,
    // ECDSA, and EdDSA keys. PBeam links a vendored OpenSSL 3.5.5; the
    // literal is the output of the pinned macOS ERTS (OpenSSL 3.5.8). The
    // Linux CI ERTS links OpenSSL 3.0.x.
    assert_eq!(
        run(&mut pbeam_probe(
            CRYPTO_CRYPT_PROBE,
            "crypto_crypt_probe",
            "host",
            &[]
        )),
        CRYPT_HOST
    );
    if cfg!(target_os = "macos") {
        let erts = run(&mut erts_probe(
            CRYPTO_CRYPT_PROBE,
            "crypto_crypt_probe",
            "host",
            &[],
        ));
        assert_eq!(erts, CRYPT_HOST);
    }
}

/// The output of `crypto_crypt_probe:host/0` on the pinned macOS ERTS.
const CRYPT_HOST: &str = r#"[{error,{error,{"pkey.c",1410},"Couldn't set rsa padding"}},
 <<246,255,136,76,65,131,10,181,77,171>>,
 {error,{notsup,{"pkey.c",1381},"Can't initiate encrypt/decrypt"}},
 {error,{notsup,{"pkey.c",1381},"Can't initiate encrypt/decrypt"}},
 {error,{notsup,{"pkey.c",1381},"Can't initiate encrypt/decrypt"}},
 true,
 {error,{error,{"pkey.c",1474},"Couldn't get the result"}},
 {error,{notsup,{"pkey.c",1381},"Can't initiate encrypt/decrypt"}},
 {error,{notsup,{"pkey.c",1381},"Can't initiate encrypt/decrypt"}},
 {error,{notsup,{"pkey.c",1381},"Can't initiate encrypt/decrypt"}},
 true]
"#;

#[test]
fn stops_with_a_runtime_error_for_each_unsupported_signature() {
    for (function, expected) in [
        (
            "unsupported_name",
            "crypto:pkey_sign_nif/5 NIF is not supported for the algorithm dh",
        ),
        (
            "unsupported_curve",
            "crypto:pkey_sign_nif/5 NIF is not supported for an ECDSA curve over a \
             characteristic-two field",
        ),
    ] {
        let output = pbeam_probe(CRYPTO_SIGN_PROBE, "crypto_sign_probe", function, &[])
            .env("TERM", "dumb")
            .stdin(Stdio::null())
            .output()
            .unwrap();
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(
            !output.status.success() && stderr.contains(expected),
            "{function}: {stderr}"
        );
    }
}

const ONESHOT_HOST: &str = r#"[[{error,notsup},
  {error,{notsup,{"hash.c",115},
                 "Digest type not supported in this cryptolib"}},
  {error,{notsup,{"hash.c",180},"Unsupported digest type"}},
  {error,{error,{"mac.c",405},"Couldn't get mac"}}],
 [{rc2_cbc,{error,{notsup,{"mac.c",346},"Unsupported cipher algorithm"}}},
  {rc4,{error,{notsup,{"mac.c",346},"Unsupported cipher algorithm"}}},
  {des_cbc,{error,{notsup,{"mac.c",346},"Unsupported cipher algorithm"}}},
  {des_cfb,{error,{notsup,{"mac.c",346},"Unsupported cipher algorithm"}}},
  {des_ecb,{error,{notsup,{"mac.c",346},"Unsupported cipher algorithm"}}},
  {des_ede3_cbc,{error,{error,{"mac.c",405},"Couldn't get mac"}}},
  {des_ede3_cfb,{error,{error,{"mac.c",405},"Couldn't get mac"}}},
  {blowfish_cbc,{error,{notsup,{"mac.c",346},"Unsupported cipher algorithm"}}},
  {blowfish_cfb64,{error,{notsup,{"mac.c",346},
                                 "Unsupported cipher algorithm"}}},
  {blowfish_ofb64,{error,{notsup,{"mac.c",346},
                                 "Unsupported cipher algorithm"}}},
  {blowfish_ecb,{error,{notsup,{"mac.c",346},"Unsupported cipher algorithm"}}},
  {sm4_ecb,{error,{error,{"mac.c",405},"Couldn't get mac"}}},
  {sm4_cfb,{error,{error,{"mac.c",405},"Couldn't get mac"}}},
  {sm4_ofb,{error,{error,{"mac.c",405},"Couldn't get mac"}}},
  {sm4_ctr,{error,{error,{"mac.c",405},"Couldn't get mac"}}},
  {sm4_gcm,{error,{error,{"mac.c",405},"Couldn't get mac"}}},
  {sm4_ccm,{error,{error,{"mac.c",405},"Couldn't get mac"}}},
  {aes_128_ofb,{error,{error,{"mac.c",405},"Couldn't get mac"}}},
  {aes_128_cfb8,{error,{error,{"mac.c",405},"Couldn't get mac"}}},
  {aes_128_cfb128,{error,{error,{"mac.c",405},"Couldn't get mac"}}},
  {aes_128_ecb,{error,{error,{"mac.c",405},"Couldn't get mac"}}},
  {aes_128_ctr,{error,{error,{"mac.c",405},"Couldn't get mac"}}},
  {chacha20,{error,{error,{"mac.c",405},"Couldn't get mac"}}},
  {chacha20_poly1305,{error,{error,{"mac.c",405},"Couldn't get mac"}}},
  {aes_128_gcm,{error,{error,{"mac.c",405},"Couldn't get mac"}}},
  {aes_256_ccm,{error,{error,{"mac.c",405},"Couldn't get mac"}}},
  {aes_ecb,{error,{error,{"mac.c",405},"Couldn't get mac"}}}]]
"#;

fn erts_probe(probe: &str, module: &str, function: &str, flags: &[&str]) -> Command {
    let mut command = Command::new("erl");
    command.args([
        "+S",
        "1",
        "-noshell",
        "-boot",
        "start_clean",
        "-pa",
        CODE_PATH,
    ]);
    command.args(
        flags
            .iter()
            .map(|flag| flag.strip_prefix('-').unwrap_or(flag)),
    );
    command.args([
        "-eval",
        &format!(
            "{{module, _}} = code:load_abs(\"{probe}\"), \
             {module}:{function}(), halt()."
        ),
    ]);
    command
}

/// Return the command of the probe on the `pbeam` binary with `--root`,
/// the code path directory, and `flags`.
///
/// # Examples
///
/// ```rust,ignore
/// let output = run(&mut pbeam(&["--mode", "embedded"]));
/// ```
fn pbeam(flags: &[&str]) -> Command {
    pbeam_probe(CODE_SERVER_PROBE, "code_server_probe", "main", flags)
}

/// Return the command of `module:function()` of the probe file `probe` on
/// the `pbeam` binary, as [`pbeam`] gives it.
///
/// # Examples
///
/// ```rust,ignore
/// let output = run(&mut pbeam_probe(ON_LOAD_PROBE, "on_load_server_probe", "main", &[]));
/// ```
fn pbeam_probe(probe: &str, module: &str, function: &str, flags: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_pbeam"));
    command
        .args(["pbeam_boot", "start", module, function, "--root"])
        .arg(otp_root())
        .args(["--pa", CODE_PATH])
        .args(flags)
        .arg("--")
        .arg(boot_beam())
        .args(
            BOOT_MODULES
                .iter()
                .chain(&CODE_SERVER_MODULES)
                .chain(&KERNEL_START_MODULES)
                .map(|module| pinned_beam_path(module)),
        )
        // ERTS loads the probe on demand after the boot modules, so the
        // probe comes last and its atoms take the higher indexes.
        .arg(format!("{probe}.beam"));
    command
}

/// Run a command and return its stdout. The run must exit with status 0 and
/// write nothing to stderr.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(run(Command::new("echo").arg("a")), "a\n");
/// ```
fn run(command: &mut Command) -> String {
    let output: Output = command
        .env("TERM", "dumb")
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(
        output.status.success() && output.stderr.is_empty(),
        "{command:?}: {output:?}"
    );
    String::from_utf8(output.stdout).unwrap()
}

/// The output of `crypto_probe:ciphers_host/0` on PBeam and on the pinned
/// macOS ERTS: `notsup` for each legacy cipher (cipher.c:282-283,
/// api_ng.c:284-287), the AEAD results of sm4_gcm and sm4_ccm, and the type of each cipher.
const CIPHERS_HOST: &str = r#"[[{error,notsup},
  {error,notsup},
  {error,notsup},
  {error,notsup},
  {error,notsup},
  {error,notsup},
  {error,notsup},
  {error,notsup},
  {error,notsup}],
 [{error,{notsup,{"api_ng.c",286},
                 "Cipher not supported in this libcrypto version"}},
  {error,{notsup,{"api_ng.c",286},
                 "Cipher not supported in this libcrypto version"}},
  {error,{notsup,{"api_ng.c",286},
                 "Cipher not supported in this libcrypto version"}},
  {error,{notsup,{"api_ng.c",286},
                 "Cipher not supported in this libcrypto version"}},
  {error,{notsup,{"api_ng.c",286},
                 "Cipher not supported in this libcrypto version"}},
  {error,{notsup,{"api_ng.c",286},
                 "Cipher not supported in this libcrypto version"}},
  {error,{notsup,{"api_ng.c",286},
                 "Cipher not supported in this libcrypto version"}},
  {error,{notsup,{"api_ng.c",286},
                 "Cipher not supported in this libcrypto version"}},
  {error,{notsup,{"api_ng.c",286},
                 "Cipher not supported in this libcrypto version"}}],
 [{sm4_gcm,{<<203,180,174,141,189,248,170,187,99,31>>,
            <<163,50,144,65,156,39,236,69,248,146,217,89,39,62,24,144>>},
           {<<203,180,174,141,189,248,170,187,99,31>>,
            <<163,50,144,65,156,39,236,69,248,146,217,89>>},
           {<<207,189,183,144>>,
            <<52,228,94,5,194,200,243,208,128,223,76,173,240,182,70,141>>},
           [<<"plain text">>,error,<<"plain text">>]},
  {sm4_ccm,{<<85,149,167,141,192,12,141,183,179,255>>,
            <<198,214,173,145,194,170,12,155,203,131,206,1,194,77,161,88>>},
           {<<85,149,167,141,192,12,141,183,179,255>>,
            <<156,133,7,167,239,92,51,235,150,61,225,73>>},
           {<<81,156,190,144>>,
            <<238,134,79,88,94,103,180,117,75,217,119,49,223,226,20,43>>},
           [<<"plain text">>,error,error]}],
 [44,61,1134,1133,1137,1135,1139,419,423,427,420,424,428,421,425,429,421,425,
  429,418,422,426,undefined,undefined,undefined,undefined,undefined,895,898,
  901,896,899,902]]
"#;

#[test]
fn generates_keys_as_pinned_erts() {
    // rsa_generate_key_nif/2, ec_generate_key_nif/2, and
    // evp_generate_key_nif/2 (rsa.c:453-543, ec.c:353-517, evp.c:221-300):
    // the sizes of each new key and a sign and verify with it, the bytes of
    // each key from a given private key, and the error tuple of each
    // failed check.
    let erts = run(&mut erts_probe(
        CRYPTO_KEYGEN_PROBE,
        "crypto_keygen_probe",
        "main",
        &[],
    ));
    assert!(erts.contains("Can't generate RSA key-pair"), "{erts}");
    assert_eq!(
        run(&mut pbeam_probe(
            CRYPTO_KEYGEN_PROBE,
            "crypto_keygen_probe",
            "main",
            &[]
        )),
        erts
    );
}

/// The output of `crypto_keygen_probe:post_quantum/0` on the pinned macOS
/// ERTS (OpenSSL 3.5.8). The Linux CI ERTS links OpenSSL 3.0.x, which has no
/// ML-DSA, ML-KEM, or SLH-DSA (`evp.c:232-257`).
const KEYGEN_POST_QUANTUM_HOST: &str = r#"{[{mldsa44,1312,2560,true,true},
  {mldsa65,1952,4032,true,true},
  {mldsa87,2592,4896,true,true},
  {slh_dsa_shake_128f,32,64,true,true},
  {slh_dsa_sha2_128f,32,64,true,true}],
 [{mlkem512,800,1632,true},
  {mlkem768,1184,2400,true},
  {mlkem1024,1568,3168,true}],
 [{slh_dsa_shake_192f,48,96},{slh_dsa_sha2_256f,64,128}],
 [{error,{"evp.c",273},"Can't EVP_PKEY_new_raw_private_key"},
  {error,{"evp.c",273},"Can't EVP_PKEY_new_raw_private_key"}]}
"#;

#[test]
fn generates_ml_dsa_ml_kem_and_slh_dsa_keys() {
    assert_eq!(
        run(&mut pbeam_probe(
            CRYPTO_KEYGEN_PROBE,
            "crypto_keygen_probe",
            "post_quantum",
            &[]
        )),
        KEYGEN_POST_QUANTUM_HOST
    );
    if cfg!(target_os = "macos") {
        let erts = run(&mut erts_probe(
            CRYPTO_KEYGEN_PROBE,
            "crypto_keygen_probe",
            "post_quantum",
            &[],
        ));
        assert_eq!(erts, KEYGEN_POST_QUANTUM_HOST);
    }
}

#[test]
fn stops_on_an_ec_curve_that_key_generation_does_not_support() {
    for (function, expected) in [
        (
            "unsupported_field",
            "crypto:ec_generate_key_nif/2 NIF is not supported for an ECDSA curve over a \
             characteristic-two field",
        ),
        (
            "unsupported_unnamed",
            "crypto:ec_generate_key_nif/2 NIF is not supported for a private key on an EC \
             curve without a name",
        ),
    ] {
        let output = pbeam_probe(CRYPTO_KEYGEN_PROBE, "crypto_keygen_probe", function, &[])
            .env("TERM", "dumb")
            .stdin(Stdio::null())
            .output()
            .unwrap();
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(
            !output.status.success() && stderr.contains(expected),
            "{function}: {stderr}"
        );
    }
}

#[test]
fn computes_shared_secrets_as_pinned_erts() {
    // dh_generate_key_nif/4, dh_compute_key_nif/3, ecdh_compute_key_nif/3,
    // and evp_compute_key_nif/3 (dh.c:44-271, ecdh.c:38-141,
    // evp.c:131-218): the sizes of each new DH key, the bytes of each shared
    // secret of given keys, and the reason and error_info of each failed
    // check.
    let erts = run(&mut erts_probe(
        CRYPTO_COMPUTE_KEY_PROBE,
        "crypto_compute_key_probe",
        "main",
        &[],
    ));
    assert!(erts.contains("Can't EVP_PKEY_derive"), "{erts}");
    assert_eq!(
        run(&mut pbeam_probe(
            CRYPTO_COMPUTE_KEY_PROBE,
            "crypto_compute_key_probe",
            "main",
            &[]
        )),
        erts
    );
}

#[test]
fn stops_on_a_key_agreement_that_pbeam_does_not_support() {
    for (function, expected) in [
        (
            "unsupported_length",
            "crypto:dh_generate_key_nif/4 NIF is not supported for a DH private key length",
        ),
        (
            "unsupported_field",
            "crypto:ecdh_compute_key_nif/3 NIF is not supported for an ECDSA curve over a \
             characteristic-two field",
        ),
    ] {
        let output = pbeam_probe(
            CRYPTO_COMPUTE_KEY_PROBE,
            "crypto_compute_key_probe",
            function,
            &[],
        )
        .env("TERM", "dumb")
        .stdin(Stdio::null())
        .output()
        .unwrap();
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(
            !output.status.success() && stderr.contains(expected),
            "{function}: {stderr}"
        );
    }
}
