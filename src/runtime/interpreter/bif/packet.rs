//! The BIF `erlang:decode_packet/3` for the packet types `http_bin` and
//! `httph_bin`: the length detection of `packet_get_length`
//! (`packet_parser.c:394-450`), the HTTP parser of `packet_parse_http`
//! (`packet_parser.c:662-862`), and the results of `decode_packet_3`
//! (`erl_bif_port.c:1476-1617`). Each other packet type gives a runtime
//! error.

use super::BifResult::Value;
use super::*;
use crate::term::{AtomTable, Binary};

/// The HTTP methods that `packet_parse_http` returns as atoms, in the order
/// of `http_meth_strings` (`packet_parser.c:125-134`).
const METHODS: [&str; 7] = ["OPTIONS", "GET", "HEAD", "POST", "PUT", "DELETE", "TRACE"];

/// The header names that `packet_parse_http` returns as atoms, in the order
/// of `http_hdr_strings` (`packet_parser.c:68-122`). The bit of a known
/// header is the position in the list plus one (`erl_bif_port.c:1391`).
const HEADERS: [&str; 52] = [
    "Cache-Control",
    "Connection",
    "Date",
    "Pragma",
    "Transfer-Encoding",
    "Upgrade",
    "Via",
    "Accept",
    "Accept-Charset",
    "Accept-Encoding",
    "Accept-Language",
    "Authorization",
    "From",
    "Host",
    "If-Modified-Since",
    "If-Match",
    "If-None-Match",
    "If-Range",
    "If-Unmodified-Since",
    "Max-Forwards",
    "Proxy-Authorization",
    "Range",
    "Referer",
    "User-Agent",
    "Age",
    "Location",
    "Proxy-Authenticate",
    "Public",
    "Retry-After",
    "Server",
    "Vary",
    "Warning",
    "Www-Authenticate",
    "Allow",
    "Content-Base",
    "Content-Encoding",
    "Content-Language",
    "Content-Length",
    "Content-Location",
    "Content-Md5",
    "Content-Range",
    "Content-Type",
    "Etag",
    "Expires",
    "Last-Modified",
    "Accept-Ranges",
    "Set-Cookie",
    "Set-Cookie2",
    "X-Forwarded-For",
    "Cookie",
    "Keep-Alive",
    "Proxy-Connection",
];

/// The largest header name that `packet_parse_http` compares with the known
/// names (`HTTP_MAX_NAME_LEN`, `packet_parser.c:64`).
const MAX_NAME_LEN: usize = 50;

/// A packet type of `decode_packet_3` (`TCP_PB_*`, `packet_parser.h`).
enum Kind {
    /// `http_bin`: a request or response line, with binary strings.
    HttpBin,
    /// `httph_bin`: a header line, with binary strings.
    HttpHBin,
    /// A known type that PBeam does not run.
    Unsupported(String),
}

/// Run `erlang:decode_packet/3`, as `decode_packet_3`
/// (`erl_bif_port.c:1476-1617`) does: the type selects the parser, the
/// options give the largest packet and the largest line, and a bitstring
/// that is not a whole number of bytes or a third argument that is not a
/// proper list raises `badarg`. An unknown type raises `badarg` with the
/// `badopt` cause. A whole packet gives `{ok, Packet, Rest}` and an
/// incomplete one gives `{more, undefined}`; a length that is too large
/// gives `{error, invalid}`.
pub(super) fn decode_packet(
    context: &mut BifContext<'_>,
    packet_type: &Term,
    subject: &Term,
    options: &Term,
) -> Result<BifResult, BeamError> {
    let Some(kind) = kind(context, packet_type) else {
        return Ok(BifResult::badopt());
    };
    let Term::Binary(binary) = subject else {
        return Ok(BifResult::badarg());
    };
    let Some(options) = options.proper_list() else {
        return Ok(BifResult::badarg());
    };
    let mut packet_size = 0;
    let mut line_length = 0;
    for option in options {
        let Term::Tuple(option) = option else {
            return Ok(BifResult::badarg());
        };
        if option.len() != 2 {
            return Ok(BifResult::badarg());
        }
        let (Term::Atom(name), Term::Integer(value)) = (&option[0], &option[1]) else {
            return Ok(BifResult::badarg());
        };
        if !(0..=i64::from(u32::MAX)).contains(value) {
            return Ok(BifResult::badarg());
        }
        let value = *value as u64;
        match context.scheduler.atoms.name_of(name) {
            "packet_size" => packet_size = value,
            "line_length" => line_length = value,
            // ERTS accepts a delimiter of at most 255 for the `line` type
            // (`erl_bif_port.c:1537-1541`).
            "line_delimiter"
                if value <= 255 && matches!(&kind, Kind::Unsupported(name) if name == "line") => {}
            _ => return Ok(BifResult::badarg()),
        }
    }
    let header = match kind {
        Kind::HttpBin => false,
        Kind::HttpHBin => true,
        Kind::Unsupported(name) => {
            if !binary.bit_size().is_multiple_of(8) {
                return Ok(BifResult::badarg());
            }
            return Err(error(format!(
                "unsupported erlang:decode_packet/3 type {name} (see \"erlang:decode_packet/3\" in docs/erts-differences.md)"
            )));
        }
    };
    let Some(bytes) = binary.bytes() else {
        return Ok(BifResult::badarg());
    };
    let length = packet_length(bytes, packet_size, line_length, header);
    if length < 0 {
        return Ok(Value(Term::tuple(vec![
            atom(context, "error")?,
            atom(context, "invalid")?,
        ])));
    }
    if length == 0 {
        return Ok(Value(Term::tuple(vec![
            atom(context, "more")?,
            atom(context, "undefined")?,
        ])));
    }
    // An HTTP packet length is at most the size of the subject, so the
    // `packet_sz > bin_sz` case of `decode_packet_3` cannot occur.
    let packet_size = usize::try_from(length).expect("an HTTP packet length fits a usize");
    let packet = parse_http(&mut context.scheduler.atoms, &bytes[..packet_size], header)?;
    // The rest is a sub-binary of the subject, as
    // `erts_make_sub_binary` gives it (`erl_bif_port.c:1602-1604`).
    let rest = Term::Binary(binary.bits(packet_size * 8, binary.bit_size()));
    Ok(Value(Term::tuple(vec![atom(context, "ok")?, packet, rest])))
}

/// Return the kind of the first argument of `decode_packet_3`
/// (`erl_bif_port.c:1492-1511`), or `None` for a type that the BIF does not
/// know.
fn kind(context: &BifContext<'_>, term: &Term) -> Option<Kind> {
    match term {
        Term::Atom(atom) => Some(match context.scheduler.atoms.name_of(atom) {
            "http_bin" => Kind::HttpBin,
            "httph_bin" => Kind::HttpHBin,
            name @ ("raw" | "asn1" | "sunrm" | "cdr" | "fcgi" | "line" | "tpkt" | "http"
            | "httph" | "ssl_tls") => Kind::Unsupported(name.to_owned()),
            _ => return None,
        }),
        Term::Integer(value) if matches!(value, 0 | 1 | 2 | 4) => {
            Some(Kind::Unsupported(value.to_string()))
        }
        _ => None,
    }
}

/// Return the size of the first HTTP packet of `bytes`, or 0 for an
/// incomplete packet and -1 for a packet that is larger than the largest
/// packet or line (`packet_get_length`, `packet_parser.c:390-450`). A
/// `header` packet continues a line that starts with a space after a
/// newline (`packet_parser.c:431-436`).
fn packet_length(bytes: &[u8], packet_size: u64, line_length: u64, header: bool) -> i64 {
    let n = bytes.len();
    let max_length = if packet_size == 0 {
        line_length
    } else {
        packet_size
    };
    if (n == 1 && bytes[0] == b'\n') || (n == 2 && bytes == b"\r\n".as_slice()) {
        return i64::try_from(n).expect("a packet of two bytes fits an i64");
    }
    let mut start = 0;
    loop {
        let Some(offset) = bytes[start..].iter().position(|byte| *byte == b'\n') else {
            if max_length != 0 && n as u64 >= max_length {
                return -1;
            }
            return 0;
        };
        let end = start + offset + 1;
        if !header {
            if max_length != 0 && end as u64 > max_length {
                return -1;
            }
            return i64::try_from(end).expect("a packet length fits an i64");
        }
        if end < n {
            let next = bytes[end];
            if (next == b' ' || next == b'\t') && end > 2 {
                start = end;
                continue;
            }
            if max_length != 0 && end as u64 > max_length {
                return -1;
            }
            return i64::try_from(end).expect("a packet length fits an i64");
        }
        if max_length != 0 && end as u64 > max_length {
            return -1;
        }
        return 0;
    }
}

/// Return the packet of an HTTP line, as `packet_parse_http`
/// (`packet_parser.c:662-862`) gives it, or the `{http_error, Line}` packet
/// of a line that it cannot parse (`packet_parse`, `packet_parser.h:155-159`).
fn parse_http(atoms: &mut AtomTable, bytes: &[u8], header: bool) -> Result<Term, BeamError> {
    match line(atoms, bytes, header)? {
        Some(packet) => Ok(packet),
        None => Ok(Term::tuple(vec![
            Term::Atom(atoms.put("http_error")?),
            binary(bytes),
        ])),
    }
}

/// Return the packet of one HTTP line, or `None` for a line that
/// `packet_parse_http` refuses (`packet_parser.c:662-862`). The line is
/// `bytes` without the trailing CRLF (`packet_parser.c:669-673`).
fn line(atoms: &mut AtomTable, bytes: &[u8], header: bool) -> Result<Option<Term>, BeamError> {
    // `packet_length` gives a packet only after a newline, so the last byte
    // is always `\n`.
    let n = if bytes.len() >= 2 && bytes[bytes.len() - 2] == b'\r' {
        bytes.len() - 2
    } else {
        bytes.len() - 1
    };
    if header {
        return header_line(atoms, bytes, n);
    }
    if n >= 5 && &bytes[..5] == b"HTTP/" {
        response_line(atoms, bytes, n)
    } else {
        request_line(atoms, bytes, n)
    }
}

/// Return the packet of a status line, or `None` for a line that the parser
/// refuses (`packet_parser.c:678-728`).
fn response_line(atoms: &mut AtomTable, bytes: &[u8], n: usize) -> Result<Option<Term>, BeamError> {
    let mut ptr = 5;
    let mut left = n - 5;
    let start = ptr;
    let mut major = 0i32;
    while left > 0 && bytes[ptr].is_ascii_digit() {
        major = major
            .wrapping_mul(10)
            .wrapping_add(i32::from(bytes[ptr] - b'0'));
        ptr += 1;
        left -= 1;
    }
    if ptr == start || left == 0 || bytes[ptr] != b'.' {
        return Ok(None);
    }
    ptr += 1;
    left -= 1;
    let start = ptr;
    let mut minor = 0i32;
    while left > 0 && bytes[ptr].is_ascii_digit() {
        minor = minor
            .wrapping_mul(10)
            .wrapping_add(i32::from(bytes[ptr] - b'0'));
        ptr += 1;
        left -= 1;
    }
    if ptr == start {
        return Ok(None);
    }
    let start = ptr;
    while left > 0 && space(bytes[ptr]) {
        ptr += 1;
        left -= 1;
    }
    if ptr == start {
        return Ok(None);
    }
    let mut status = 0i32;
    while left > 0 && bytes[ptr].is_ascii_digit() {
        status = status
            .wrapping_mul(10)
            .wrapping_add(i32::from(bytes[ptr] - b'0'));
        ptr += 1;
        left -= 1;
    }
    let start = ptr;
    while left > 0 && space(bytes[ptr]) {
        ptr += 1;
        left -= 1;
    }
    if ptr == start && left > 0 {
        return Ok(None);
    }
    Ok(Some(Term::tuple(vec![
        Term::Atom(atoms.put("http_response")?),
        Term::tuple(vec![
            Term::Integer(i64::from(major)),
            Term::Integer(i64::from(minor)),
        ]),
        Term::Integer(i64::from(status)),
        binary(&bytes[ptr..ptr + left]),
    ])))
}

/// Return the packet of a request line, or `None` for a line that the
/// parser refuses (`packet_parser.c:729-799`). A line without a version is
/// HTTP/0.9.
fn request_line(atoms: &mut AtomTable, bytes: &[u8], n: usize) -> Result<Option<Term>, BeamError> {
    let mut ptr = 0;
    let mut left = n;
    while left > 0 && !tspecial(bytes[ptr]) {
        ptr += 1;
        left -= 1;
    }
    let method_len = ptr;
    if left == 0 || method_len == 0 || !space(bytes[ptr]) {
        return Ok(None);
    }
    let method = match METHODS
        .iter()
        .find(|name| name.as_bytes() == &bytes[..method_len])
    {
        Some(name) => Term::Atom(atoms.put(name)?),
        None => binary(&bytes[..method_len]),
    };
    while left > 0 && space(bytes[ptr]) {
        ptr += 1;
        left -= 1;
    }
    let uri_start = ptr;
    while left > 0 && !space(bytes[ptr]) {
        ptr += 1;
        left -= 1;
    }
    if ptr == uri_start {
        return Ok(None);
    }
    let uri_end = ptr;
    while left > 0 && space(bytes[ptr]) {
        ptr += 1;
        left -= 1;
    }
    let (major, minor) = if left == 0 {
        (0, 9)
    } else {
        if left < 8 || &bytes[ptr..ptr + 5] != b"HTTP/" {
            return Ok(None);
        }
        ptr += 5;
        left -= 5;
        let start = ptr;
        let mut major = 0i32;
        while left > 0 && bytes[ptr].is_ascii_digit() {
            major = major
                .wrapping_mul(10)
                .wrapping_add(i32::from(bytes[ptr] - b'0'));
            ptr += 1;
            left -= 1;
        }
        if ptr == start || left == 0 || bytes[ptr] != b'.' {
            return Ok(None);
        }
        ptr += 1;
        left -= 1;
        let start = ptr;
        let mut minor = 0i32;
        while left > 0 && bytes[ptr].is_ascii_digit() {
            minor = minor
                .wrapping_mul(10)
                .wrapping_add(i32::from(bytes[ptr] - b'0'));
            ptr += 1;
            left -= 1;
        }
        if ptr == start {
            return Ok(None);
        }
        (major, minor)
    };
    Ok(Some(Term::tuple(vec![
        Term::Atom(atoms.put("http_request")?),
        method,
        uri(atoms, &bytes[uri_start..uri_end])?,
        Term::tuple(vec![
            Term::Integer(i64::from(major)),
            Term::Integer(i64::from(minor)),
        ]),
    ])))
}

/// Return the packet of a header line, or `None` for a line that the parser
/// refuses (`packet_parser.c:801-860`). The name of a known header is an
/// atom and its bit is the position in [`HEADERS`] plus one; an unknown
/// name is the name with the case of a known name.
fn header_line(atoms: &mut AtomTable, bytes: &[u8], n: usize) -> Result<Option<Term>, BeamError> {
    if n == 0 {
        return Ok(Some(Term::Atom(atoms.put("http_eoh")?)));
    }
    let mut ptr = 0;
    let mut left = n;
    let mut up = true;
    let mut name = Vec::with_capacity(MAX_NAME_LEN.min(n));
    let mut name_len = 0;
    while !tspecial(bytes[ptr]) {
        if name_len < MAX_NAME_LEN {
            let mut byte = bytes[ptr];
            if up {
                if byte.is_ascii_lowercase() {
                    byte = byte.to_ascii_uppercase();
                }
                up = false;
            } else if byte.is_ascii_uppercase() {
                byte = byte.to_ascii_lowercase();
            } else if byte == b'-' {
                up = true;
            }
            name.push(byte);
        }
        name_len += 1;
        ptr += 1;
        left -= 1;
        if left == 0 {
            return Ok(None);
        }
    }
    if bytes[ptr] != b':' {
        return Ok(None);
    }
    let known = if name_len <= MAX_NAME_LEN {
        HEADERS.iter().position(|header| header.as_bytes() == name)
    } else {
        None
    };
    ptr += 1;
    left -= 1;
    while left > 0 && space(bytes[ptr]) {
        ptr += 1;
        left -= 1;
    }
    let (bit, header) = match known {
        Some(index) => (index + 1, Term::Atom(atoms.put(HEADERS[index])?)),
        None if name_len <= MAX_NAME_LEN => (0, binary(&name)),
        None => (0, binary(&bytes[..name_len])),
    };
    Ok(Some(Term::tuple(vec![
        Term::Atom(atoms.put("http_header")?),
        Term::Integer(bit as i64),
        header,
        binary(&bytes[..name_len]),
        binary(&bytes[ptr..ptr + left]),
    ])))
}

/// Return the packet of a request URI, as `http_parse_uri` and the callback
/// `http_bld_uri` give it (`packet_parser.c:615-652`,
/// `erl_bif_port.c:1315-1356`).
fn uri(atoms: &mut AtomTable, uri: &[u8]) -> Result<Term, BeamError> {
    if uri == b"*" {
        return Ok(Term::Atom(atoms.put("*")?));
    }
    if uri.len() <= 1 || uri[0] == b'/' {
        return Ok(Term::tuple(vec![
            Term::Atom(atoms.put("abs_path")?),
            binary(uri),
        ]));
    }
    for (scheme, prefix) in [("http", &b"http://"[..]), ("https", &b"https://"[..])] {
        if uri.len() >= prefix.len() && uri[..prefix.len()].eq_ignore_ascii_case(prefix) {
            return absolute_uri(atoms, scheme, &uri[prefix.len()..]);
        }
    }
    match uri.iter().position(|byte| *byte == b':') {
        Some(index) => Ok(Term::tuple(vec![
            Term::Atom(atoms.put("scheme")?),
            binary(&uri[..index]),
            binary(&uri[index + 1..]),
        ])),
        None => Ok(binary(uri)),
    }
}

/// Return the packet of an absolute URI, as `http_parse_absoluteURI` and
/// `http_bld_uri` give it (`packet_parser.c:520-582`,
/// `erl_bif_port.c:1331-1343`).
fn absolute_uri(atoms: &mut AtomTable, scheme: &str, uri: &[u8]) -> Result<Term, BeamError> {
    let (host, path) = match uri.iter().position(|byte| *byte == b'/') {
        Some(index) => (&uri[..index], &uri[index..]),
        None => (uri, &b"/"[..]),
    };
    let (host, port) = split_port(host);
    let port = match port {
        Some(port) => Term::Integer(port),
        None => Term::Atom(atoms.put("undefined")?),
    };
    Ok(Term::tuple(vec![
        Term::Atom(atoms.put("absoluteURI")?),
        Term::Atom(atoms.put(scheme)?),
        binary(host),
        port,
        binary(path),
    ]))
}

/// Return the host and the port of the host part of an absolute URI, as
/// `http_parse_absoluteURI` gives them (`packet_parser.c:538-581`): an IPv6
/// address in brackets, the first colon of a host name, a port of digits,
/// and no port for an absent or zero port.
fn split_port(uri: &[u8]) -> (&[u8], Option<i64>) {
    let Some(colon) = uri.iter().position(|byte| *byte == b':') else {
        return (uri, None);
    };
    if uri[0] == b'['
        && let Some(close) = uri.iter().position(|byte| *byte == b']')
    {
        let host = &uri[..=close];
        let last = uri.iter().rposition(|byte| *byte == b':');
        if last == Some(close + 1) {
            return (host, digits(&uri[close + 2..]));
        }
        return (host, None);
    }
    let rest = &uri[colon + 1..];
    if !rest.iter().all(u8::is_ascii_digit) {
        return (&uri[..colon], None);
    }
    let port = rest.iter().fold(0i32, |port, byte| {
        port.wrapping_mul(10).wrapping_add(i32::from(byte - b'0'))
    });
    (&uri[..colon], (port != 0).then_some(i64::from(port)))
}

/// Return the port of the digits `bytes`, or `None` for no digits, a byte
/// that is not a digit, or the port 0 (`packet_parser.c:556-563`,
/// `574-580`).
fn digits(bytes: &[u8]) -> Option<i64> {
    if bytes.is_empty() || !bytes.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let port = bytes.iter().fold(0i32, |port, byte| {
        port.wrapping_mul(10).wrapping_add(i32::from(byte - b'0'))
    });
    (port != 0).then_some(i64::from(port))
}

/// Return the binary of `bytes`.
fn binary(bytes: &[u8]) -> Term {
    Term::Binary(Binary::from(bytes))
}

/// Return the atom `name`.
fn atom(context: &mut BifContext<'_>, name: &str) -> Result<Term, BeamError> {
    Ok(Term::Atom(context.scheduler.atoms.put(name)?))
}

/// Return whether `byte` is a space or a tab (`SP`, `packet_parser.c:144`).
fn space(byte: u8) -> bool {
    byte == b' ' || byte == b'\t'
}

/// Return whether `byte` ends a token (`is_tspecial`,
/// `packet_parser.c:66`, `145`, `186-191`): a control byte or one of
/// `()<>@,;:\"/[]?={} \t`.
fn tspecial(byte: u8) -> bool {
    if !(33..128).contains(&byte) {
        return true;
    }
    matches!(
        byte,
        b'(' | b')'
            | b'<'
            | b'>'
            | b'@'
            | b','
            | b';'
            | b':'
            | b'\\'
            | b'"'
            | b'/'
            | b'['
            | b']'
            | b'?'
            | b'='
            | b'{'
            | b'}'
    )
}
