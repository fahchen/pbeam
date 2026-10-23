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

/// The probe of `crypto:strong_rand_bytes/1`.
const CRYPTO_PROBE: &str = "fixtures/code_server/crypto_probe";

/// The stand-in `crypto` module that loads the `crypto` NIF library.
const CRYPTO_STAND_IN: &str = "fixtures/crypto/crypto";

/// The probe that loads the stand-in of each module of an OTP NIF library
/// without bindings in `fixtures/nif_libraries`.
const NIF_LIBRARY_PROBE: &str = "fixtures/nif_libraries/nif_library_probe";

/// The probe of the `zstd` contexts, which loads the pinned `zstd`.
const ZSTD_PROBE: &str = "fixtures/zstd/zstd_probe";

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
fn gives_the_crypto_algorithm_lists_and_fips_status_as_pinned_erts() {
    // The pinned ERTS lists each algorithm of its OpenSSL: 187 on macOS
    // and 177 with the libcrypto of the Linux CI host (algorithms.c:86-224,
    // digest.c:226-242, crypto.erl:843-895). PBeam lists only the 7 digests
    // that it computes, in the ERTS order, so the items that PBeam can run
    // are the same.
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
    let items = "[{hashs,[sha512_256,sha512_224,sha512,sha384,sha256,sha224,sha]},\n  \
                 {ciphers,[]},\n  \
                 {kems,[]},\n  \
                 {public_keys,[]},\n  \
                 {macs,[]},\n  \
                 {curves,[]},\n  \
                 {rsa_opts,[]}]";
    let rest = format!(",\n {items},\n false,true]\n");
    let count = erts
        .strip_suffix(rest.as_str())
        .and_then(|head| head.strip_prefix('['));
    assert!(matches!(count, Some("187" | "177")), "{erts}");
    assert_eq!(pbeam, format!("[7,\n {items},\n true,true]\n"));
    // A build without FIPS support (fips.c:27-62).
    let erts = run(&mut erts_probe(CRYPTO_PROBE, "crypto_probe", "fips", &[]));
    assert_eq!(erts, "[not_supported,false,true,{error,badarg}]\n");
    assert_eq!(
        run(&mut pbeam_probe(CRYPTO_PROBE, "crypto_probe", "fips", &[])),
        erts
    );
}

#[test]
fn stops_with_a_runtime_error_for_an_unsupported_crypto_nif() {
    // PBeam does not link `hash_init_nif/1`, which `crypto:hash_init/1`
    // calls (crypto.erl:1160). `crypto:hash/2` sends an input of more than
    // 20000 bytes through the same NIF (crypto.erl:4535-4540).
    for function in ["hash_init", "hash_large"] {
        let output = pbeam_probe(CRYPTO_PROBE, "crypto_probe", function, &[])
            .env("TERM", "dumb")
            .stdin(Stdio::null())
            .output()
            .unwrap();
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(
            !output.status.success()
                && stderr.contains("crypto:hash_init_nif/1 NIF is not supported"),
            "{function}: {stderr}"
        );
    }
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
    let expected = "[{error,{badfun,fun_term}},{error,{badfun,fun_term}},\
                    [{name,'-make_first/1-fun-0-'},\
                     {name,'-make_second/1-fun-0-'}],\
                    [{name,[]},{name,[]}],[{name,[]},{name,[]}]]\n";
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

/// Return the command of `module:function()` of the probe file `probe` on
/// the pinned ERTS, as [`erts`] gives it.
///
/// # Examples
///
/// ```rust,ignore
/// let output = run(&mut erts_probe(ON_LOAD_PROBE, "on_load_server_probe", "main", &[]));
/// ```
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
        .arg(format!("{probe}.beam"))
        .arg(boot_beam())
        .args(
            BOOT_MODULES
                .iter()
                .chain(&CODE_SERVER_MODULES)
                .chain(&KERNEL_START_MODULES)
                .map(|module| pinned_beam_path(module)),
        );
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
