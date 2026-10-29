//! Runs `decode_packet` for HTTP lines and headers on PBeam and compares the results with the
//! pinned ERTS.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers report a failure by a panic"
)]

use std::process::Command;

use pbeam_runtime::{BeamProgram, HostTerm, Map, Term};

const PROBE: &[u8] = include_bytes!("../../fixtures/decode_packet/decode_packet_probe.beam");

/// Run `decode_packet_probe:Function()` on PBeam and return its host term.
fn run(function: &str) -> HostTerm {
    let program = BeamProgram::load(&[PROBE]).unwrap();
    program
        .execute("decode_packet_probe", function, &[])
        .unwrap()
}

/// Run `decode_packet_probe:Function()` under the pinned ERTS and return its
/// `~w` output.
fn reference(function: &str) -> Vec<u8> {
    let output = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/decode_packet",
            "-eval",
            &format!("io:format(\"~w~n\", [decode_packet_probe:{function}()]), halt()."),
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(output.stderr, [] as [u8; 0]);
    output.stdout
}

fn atom(result: &HostTerm, name: &str) -> Term {
    Term::Atom(result.atom(name).expect("the result holds the atom"))
}

fn binary(bytes: &[u8]) -> Term {
    Term::binary(bytes.to_vec())
}

fn int(value: i64) -> Term {
    Term::Integer(value)
}

fn tuple(items: Vec<Term>) -> Term {
    Term::tuple(items)
}

fn list(items: Vec<Term>) -> Term {
    Term::list_from(items)
}

fn ok(result: &HostTerm, packet: Term, rest: Term) -> Term {
    tuple(vec![atom(result, "ok"), packet, rest])
}

fn request(result: &HostTerm, method: Term, uri: Term, version: (i64, i64)) -> Term {
    tuple(vec![
        atom(result, "http_request"),
        method,
        uri,
        tuple(vec![int(version.0), int(version.1)]),
    ])
}

fn response(result: &HostTerm, version: (i64, i64), status: i64, phrase: &[u8]) -> Term {
    tuple(vec![
        atom(result, "http_response"),
        tuple(vec![int(version.0), int(version.1)]),
        int(status),
        binary(phrase),
    ])
}

fn header(result: &HostTerm, bit: i64, name: Term, oname: &[u8], value: &[u8]) -> Term {
    tuple(vec![
        atom(result, "http_header"),
        int(bit),
        name,
        binary(oname),
        binary(value),
    ])
}

fn abs_path(result: &HostTerm, path: &[u8]) -> Term {
    tuple(vec![atom(result, "abs_path"), binary(path)])
}

fn absolute(result: &HostTerm, scheme: &str, host: &[u8], port: Option<i64>, path: &[u8]) -> Term {
    let port = match port {
        Some(port) => int(port),
        None => atom(result, "undefined"),
    };
    tuple(vec![
        atom(result, "absoluteURI"),
        atom(result, scheme),
        binary(host),
        port,
        binary(path),
    ])
}

fn scheme(result: &HostTerm, scheme: &[u8], rest: &[u8]) -> Term {
    tuple(vec![atom(result, "scheme"), binary(scheme), binary(rest)])
}

fn error_info(result: &HostTerm, pairs: Vec<(Term, Term)>) -> Term {
    let map = Map::from_pairs(pairs, result.atoms()).unwrap();
    list(vec![tuple(vec![
        atom(result, "error_info"),
        Term::Map(map),
    ])])
}

const REQUEST_LINES: &[u8] = br"[{ok,{http_request,'GET',{abs_path,<<47>>},{1,1}},<<>>},{ok,{http_request,<<112,111,115,116>>,{abs_path,<<47,120>>},{2,0}},<<>>},{ok,{http_request,<<70,79,79>>,{abs_path,<<47>>},{1,1}},<<>>},{ok,{http_request,'GET',{abs_path,<<47>>},{0,9}},<<>>},{ok,{http_request,'OPTIONS','*',{1,1}},<<>>},{ok,{http_request,'GET',{absoluteURI,http,<<101,120,97,109,112,108,101,46,99,111,109>>,8080,<<47,112,63,113>>},{1,1}},<<>>},{ok,{http_request,'GET',{absoluteURI,https,<<91,58,58,49,93>>,4000,<<47,120>>},{1,1}},<<>>},{ok,{http_request,'GET',{absoluteURI,http,<<91,58,58,49,93>>,undefined,<<47,120>>},{1,1}},<<>>},{ok,{http_request,'GET',{absoluteURI,http,<<101,120,97,109,112,108,101,46,99,111,109>>,undefined,<<47>>},{1,1}},<<>>},{ok,{http_request,'GET',{scheme,<<104,116,116,112>>,<<47,120>>},{1,1}},<<>>},{ok,{http_request,'GET',{scheme,<<102,111,111>>,<<98,97,114>>},{1,1}},<<>>},{ok,{http_request,'GET',<<102,111,111>>,{1,1}},<<>>},{ok,{http_request,'GET',{absoluteURI,http,<<101,120,97,109,112,108,101,46,99,111,109>>,undefined,<<47,120>>},{1,1}},<<>>},{ok,{http_request,'GET',{absoluteURI,http,<<91,58,58,49,93>>,undefined,<<47,120>>},{1,1}},<<>>},{ok,{http_request,'GET',{absoluteURI,http,<<91,58,58,49,93>>,undefined,<<47,120>>},{1,1}},<<>>},{ok,{http_response,{1,1},200,<<79,75>>},<<>>},{ok,{http_response,{1,0},404,<<>>},<<>>},{ok,{http_request,'GET',{abs_path,<<47>>},{1,1}},<<88>>}]
";
const HEADER_LINES: &[u8] = br"[{ok,{http_header,14,'Host',<<72,111,115,116>>,<<101,120,97,109,112,108,101,46,99,111,109>>},<<13,10>>},{ok,{http_header,38,'Content-Length',<<99,111,110,116,101,110,116,45,108,101,110,103,116,104>>,<<53>>},<<13,10>>},{ok,{http_header,0,<<88,45,70,111,111>>,<<120,45,102,111,111>>,<<98,97,114>>},<<13,10>>},{ok,{http_header,0,<<88,45,76,111,110,103>>,<<88,45,76,111,110,103>>,<<97,13,10,32,98>>},<<13,10>>},{ok,{http_header,0,<<88>>,<<88>>,<<>>},<<13,10>>},{ok,{http_header,0,<<88>>,<<88>>,<<118,97,108,117,101,32,32>>},<<13,10>>},{ok,{http_header,0,<<88>>,<<88>>,<<118,97,108,117,101>>},<<13,10>>},{ok,{http_header,0,<<84,104,105,115,73,115,65,86,101,114,121,76,111,110,103,72,101,97,100,101,114,78,97,109,101,84,104,97,116,69,120,99,101,101,100,115,84,104,101,70,105,102,116,121,67,104,97,114,97,99,116,101,114,76,105,109,105,116>>,<<84,104,105,115,73,115,65,86,101,114,121,76,111,110,103,72,101,97,100,101,114,78,97,109,101,84,104,97,116,69,120,99,101,101,100,115,84,104,101,70,105,102,116,121,67,104,97,114,97,99,116,101,114,76,105,109,105,116>>,<<118>>},<<13,10>>}]
";
const END_OF_HEADERS: &[u8] = br"[{ok,http_eoh,<<>>},{ok,http_eoh,<<>>},{ok,{http_header,14,'Host',<<72,111,115,116>>,<<120>>},<<13,10>>}]
";
const INCOMPLETE: &[u8] = br"[{more,undefined},{more,undefined},{more,undefined},{more,undefined}]
";
const MALFORMED: &[u8] = br"[{ok,{http_error,<<66,79,71,85,83,13,10>>},<<>>},{ok,{http_error,<<13,10>>},<<>>},{ok,{http_error,<<78,111,67,111,108,111,110,13,10>>},<<13,10>>},{ok,{http_error,<<66,97,100,32,72,101,97,100,101,114,13,10>>},<<13,10>>},{ok,{http_error,<<71,69,32,84,32,47,32,72,84,84,80,47,49,46,49,13,10>>},<<>>},{ok,{http_error,<<72,84,84,80,47,49,32,50,48,48,32,79,75,13,10>>},<<>>},{ok,{http_error,<<72,84,84,80,47,49,46,32,50,48,48,32,79,75,13,10>>},<<>>},{ok,{http_error,<<72,84,84,80,47,49,46,49,13,10>>},<<>>},{ok,{http_error,<<72,84,84,80,47,49,46,49,32,50,48,48,79,75,13,10>>},<<>>},{ok,{http_error,<<71,69,84,32,13,10>>},<<>>},{ok,{http_error,<<71,69,84,32,47,32,72,84,80,47,49,46,49,13,10>>},<<>>},{ok,{http_error,<<71,69,84,32,47,32,72,84,84,80,47,49,46,13,10>>},<<>>},{ok,{http_error,<<71,69,84,32,47,32,72,84,84,80,47,120,121,122,13,10>>},<<>>},{ok,{http_error,<<71,69,84,32,47,32,72,84,84,80,47,49,50,120,13,10>>},<<>>},{ok,{http_error,<<71,69,84,32,47,32,72,84,84,80,47,49,50,51,13,10>>},<<>>},{ok,{http_error,<<71,69,84,32,47,32,72,84,84,80,47,49,46,97,13,10>>},<<>>}]
";
const OPTIONS: &[u8] = br"[{error,invalid},{ok,{http_request,'GET',{abs_path,<<47>>},{1,1}},<<>>},{ok,{http_request,'GET',{abs_path,<<47>>},{1,1}},<<>>},{error,invalid},{ok,{http_header,14,'Host',<<72,111,115,116>>,<<120>>},<<13,10>>},{error,invalid},{error,invalid}]
";
const BADARGS: &[u8] = br"[{error,badarg,[{error_info,#{module => erl_erts_errors}}]},{error,badarg,[{error_info,#{cause => badopt,module => erl_erts_errors}}]},{error,badarg,[{error_info,#{module => erl_erts_errors}}]},{error,badarg,[{error_info,#{module => erl_erts_errors}}]},{error,badarg,[{error_info,#{module => erl_erts_errors}}]},{error,badarg,[{error_info,#{module => erl_erts_errors}}]},{error,badarg,[{error_info,#{module => erl_erts_errors}}]},{error,badarg,[{error_info,#{module => erl_erts_errors}}]},{error,badarg,[{error_info,#{module => erl_erts_errors}}]}]
";

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "case table: one expected result for each request line"
)]
fn serves_http_request_lines_as_pinned_erts() {
    let result = run("request_lines");
    let get = atom(&result, "GET");
    let expected = list(vec![
        ok(
            &result,
            request(&result, get.clone(), abs_path(&result, b"/"), (1, 1)),
            binary(b""),
        ),
        ok(
            &result,
            request(&result, binary(b"post"), abs_path(&result, b"/x"), (2, 0)),
            binary(b""),
        ),
        ok(
            &result,
            request(&result, binary(b"FOO"), abs_path(&result, b"/"), (1, 1)),
            binary(b""),
        ),
        ok(
            &result,
            request(&result, get.clone(), abs_path(&result, b"/"), (0, 9)),
            binary(b""),
        ),
        ok(
            &result,
            request(
                &result,
                atom(&result, "OPTIONS"),
                atom(&result, "*"),
                (1, 1),
            ),
            binary(b""),
        ),
        ok(
            &result,
            request(
                &result,
                get.clone(),
                absolute(&result, "http", b"example.com", Some(8080), b"/p?q"),
                (1, 1),
            ),
            binary(b""),
        ),
        ok(
            &result,
            request(
                &result,
                get.clone(),
                absolute(&result, "https", b"[::1]", Some(4000), b"/x"),
                (1, 1),
            ),
            binary(b""),
        ),
        ok(
            &result,
            request(
                &result,
                get.clone(),
                absolute(&result, "http", b"[::1]", None, b"/x"),
                (1, 1),
            ),
            binary(b""),
        ),
        ok(
            &result,
            request(
                &result,
                get.clone(),
                absolute(&result, "http", b"example.com", None, b"/"),
                (1, 1),
            ),
            binary(b""),
        ),
        ok(
            &result,
            request(
                &result,
                get.clone(),
                scheme(&result, b"http", b"/x"),
                (1, 1),
            ),
            binary(b""),
        ),
        ok(
            &result,
            request(
                &result,
                get.clone(),
                scheme(&result, b"foo", b"bar"),
                (1, 1),
            ),
            binary(b""),
        ),
        ok(
            &result,
            request(&result, get.clone(), binary(b"foo"), (1, 1)),
            binary(b""),
        ),
        ok(
            &result,
            request(
                &result,
                get.clone(),
                absolute(&result, "http", b"example.com", None, b"/x"),
                (1, 1),
            ),
            binary(b""),
        ),
        ok(
            &result,
            request(
                &result,
                get.clone(),
                absolute(&result, "http", b"[::1]", None, b"/x"),
                (1, 1),
            ),
            binary(b""),
        ),
        ok(
            &result,
            request(
                &result,
                get.clone(),
                absolute(&result, "http", b"[::1]", None, b"/x"),
                (1, 1),
            ),
            binary(b""),
        ),
        ok(&result, response(&result, (1, 1), 200, b"OK"), binary(b"")),
        ok(&result, response(&result, (1, 0), 404, b""), binary(b"")),
        ok(
            &result,
            request(&result, get, abs_path(&result, b"/"), (1, 1)),
            binary(b"X"),
        ),
    ]);
    assert_eq!(result.term, expected);
    assert_eq!(reference("request_lines"), REQUEST_LINES);
}

#[test]
fn serves_http_header_lines_as_pinned_erts() {
    let result = run("header_lines");
    let rest = binary(b"\r\n");
    let expected = list(vec![
        ok(
            &result,
            header(&result, 14, atom(&result, "Host"), b"Host", b"example.com"),
            rest.clone(),
        ),
        ok(
            &result,
            header(
                &result,
                38,
                atom(&result, "Content-Length"),
                b"content-length",
                b"5",
            ),
            rest.clone(),
        ),
        ok(
            &result,
            header(&result, 0, binary(b"X-Foo"), b"x-foo", b"bar"),
            rest.clone(),
        ),
        ok(
            &result,
            header(&result, 0, binary(b"X-Long"), b"X-Long", b"a\r\n b"),
            rest.clone(),
        ),
        ok(
            &result,
            header(&result, 0, binary(b"X"), b"X", b""),
            rest.clone(),
        ),
        ok(
            &result,
            header(&result, 0, binary(b"X"), b"X", b"value  "),
            rest.clone(),
        ),
        ok(
            &result,
            header(&result, 0, binary(b"X"), b"X", b"value"),
            rest.clone(),
        ),
        ok(
            &result,
            header(
                &result,
                0,
                binary(b"ThisIsAVeryLongHeaderNameThatExceedsTheFiftyCharacterLimit"),
                b"ThisIsAVeryLongHeaderNameThatExceedsTheFiftyCharacterLimit",
                b"v",
            ),
            rest,
        ),
    ]);
    assert_eq!(result.term, expected);
    assert_eq!(reference("header_lines"), HEADER_LINES);
}

#[test]
fn serves_the_end_of_http_headers_as_pinned_erts() {
    let result = run("end_of_headers");
    let rest = binary(b"\r\n");
    let expected = list(vec![
        ok(&result, atom(&result, "http_eoh"), binary(b"")),
        ok(&result, atom(&result, "http_eoh"), binary(b"")),
        ok(
            &result,
            header(&result, 14, atom(&result, "Host"), b"Host", b"x"),
            rest,
        ),
    ]);
    assert_eq!(result.term, expected);
    assert_eq!(reference("end_of_headers"), END_OF_HEADERS);
}

#[test]
fn waits_for_more_http_data_as_pinned_erts() {
    let result = run("incomplete");
    let more = tuple(vec![atom(&result, "more"), atom(&result, "undefined")]);
    let expected = list(vec![more.clone(), more.clone(), more.clone(), more]);
    assert_eq!(result.term, expected);
    assert_eq!(reference("incomplete"), INCOMPLETE);
}

#[test]
fn reports_a_malformed_http_line_as_pinned_erts() {
    let result = run("malformed");
    let error = |line: &[u8], rest: &[u8]| {
        ok(
            &result,
            tuple(vec![atom(&result, "http_error"), binary(line)]),
            binary(rest),
        )
    };
    let expected = list(vec![
        error(b"BOGUS\r\n", b""),
        error(b"\r\n", b""),
        error(b"NoColon\r\n", b"\r\n"),
        error(b"Bad Header\r\n", b"\r\n"),
        error(b"GE T / HTTP/1.1\r\n", b""),
        error(b"HTTP/1 200 OK\r\n", b""),
        error(b"HTTP/1. 200 OK\r\n", b""),
        error(b"HTTP/1.1\r\n", b""),
        error(b"HTTP/1.1 200OK\r\n", b""),
        error(b"GET \r\n", b""),
        error(b"GET / HTP/1.1\r\n", b""),
        error(b"GET / HTTP/1.\r\n", b""),
        error(b"GET / HTTP/xyz\r\n", b""),
        error(b"GET / HTTP/12x\r\n", b""),
        error(b"GET / HTTP/123\r\n", b""),
        error(b"GET / HTTP/1.a\r\n", b""),
    ]);
    assert_eq!(result.term, expected);
    assert_eq!(reference("malformed"), MALFORMED);
}

#[test]
fn limits_an_http_line_by_its_options_as_pinned_erts() {
    let result = run("options");
    let invalid = tuple(vec![atom(&result, "error"), atom(&result, "invalid")]);
    let get = atom(&result, "GET");
    let request_line = |()| {
        ok(
            &result,
            request(&result, get.clone(), abs_path(&result, b"/"), (1, 1)),
            binary(b""),
        )
    };
    let expected = list(vec![
        invalid.clone(),
        request_line(()),
        request_line(()),
        invalid.clone(),
        ok(
            &result,
            header(&result, 14, atom(&result, "Host"), b"Host", b"x"),
            binary(b"\r\n"),
        ),
        invalid.clone(),
        invalid,
    ]);
    assert_eq!(result.term, expected);
    assert_eq!(reference("options"), OPTIONS);
}

#[test]
fn raises_the_decode_packet_badargs_as_pinned_erts() {
    let result = run("badargs");
    let plain = || {
        tuple(vec![
            atom(&result, "error"),
            atom(&result, "badarg"),
            error_info(
                &result,
                vec![(atom(&result, "module"), atom(&result, "erl_erts_errors"))],
            ),
        ])
    };
    let cause = || {
        tuple(vec![
            atom(&result, "error"),
            atom(&result, "badarg"),
            error_info(
                &result,
                vec![
                    (atom(&result, "cause"), atom(&result, "badopt")),
                    (atom(&result, "module"), atom(&result, "erl_erts_errors")),
                ],
            ),
        ])
    };
    let expected = list(vec![
        plain(),
        cause(),
        plain(),
        plain(),
        plain(),
        plain(),
        plain(),
        plain(),
        plain(),
    ]);
    assert_eq!(result.term, expected);
    assert_eq!(reference("badargs"), BADARGS);
}

#[test]
fn stops_at_a_packet_type_that_pbeam_does_not_run() {
    let program = BeamProgram::load(&[PROBE]).unwrap();
    let error = program
        .execute("decode_packet_probe", "unsupported", &[])
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("unsupported erlang:decode_packet/3 type http"),
        "{error}"
    );
}
