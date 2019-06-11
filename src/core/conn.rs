#![allow(clippy::borrowed_box)]
#![allow(dead_code)]

use std::io::{prelude::*, BufWriter};
use std::net::Shutdown;
use std::str;
use std::time::Duration;

use crate::channel;
use crate::core::config::ConnMetadata;
use crate::core::http::{
    Request, RequestWriter, Response, ResponseManager, ResponseStates, ResponseWriter,
};
use crate::core::router::{Route, RouteSeeker, REST, RouteHandler};
use crate::core::stream::Stream;
use crate::hashbrown::HashMap;
use crate::support::{
    common::flush_buffer, common::write_to_buff, common::MapUpdates, debug, debug::InfoLevel,
    shared_pool, TaskType,
};

static HEADER_END: [u8; 2] = [13, 10];
static FLUSH_RETRY: u8 = 4;

type ExecCode = u8;
type BaseLine = Option<channel::Receiver<(RouteHandler, HashMap<String, String>)>>;

//TODO/P3: in lib, launch workers that will handle the request parsing, and write to the stream.
//         implementation shall be built in this module

#[derive(PartialEq, Eq, Clone, Copy)]
enum ConnError {
    HeartBeat,
    EmptyRequest,
    ReadStreamFailure,
    AccessDenied,
    ServiceUnavailable,
}

pub(crate) fn handle_connection(mut stream: Stream) -> ExecCode {
    //TODO/P2: refactor this code, such that we send request to a worker who will generate the resp
    //         and we will write them to this stream in sequence.
    let (callback, request) = match recv_request(&mut stream) {
        Err(err) => {
            let status: u16 = match err {
                ConnError::EmptyRequest => 400,
                ConnError::AccessDenied => 401,
                ConnError::ServiceUnavailable => 404,
                ConnError::ReadStreamFailure | ConnError::HeartBeat => {
                    // connection is sour, shutdown now
                    if let Err(err) = stream.shutdown(Shutdown::Both) {
                        return 1;
                    }

                    return 0;
                },
            };

            debug::print(
                &format!("Error on parsing request: {}", status),
                InfoLevel::Error,
            );

            return write_to_stream(stream, &mut build_err_response(status));
        },
        Ok(cb) => cb,
    };

    let is_tls = stream.is_tls();
    send_response(stream, request, callback, is_tls)
}

#[inline]
pub(crate) fn send_err_resp(stream: Stream, err_code: u16) -> ExecCode {
    write_to_stream(stream, &mut build_err_response(err_code))
}

fn send_response(
    stream: Stream,
    request: Box<Request>,
    mut callback: RouteHandler,
    is_tls: bool,
) -> ExecCode
{
    let mut response = initialize_response(is_tls);
    match request.header("connection") {
        Some(ref val) if val.eq(&String::from("close")) => response.can_keep_alive(false),
        _ => response.can_keep_alive(true),
    };

    if request.method.eq(&REST::OTHER(String::from("HEAD"))) {
        response.header_only(true);
    }

    // callback function will decide what to be written into the response
    callback.execute(&request, &mut response);

    response.redirect_handling();
    response.validate_and_update();

    write_to_stream(stream, &mut response)
}

fn initialize_response(is_tls: bool) -> Box<Response> {
    let header = ConnMetadata::get_default_header();
    let mut resp = match header {
        None => Box::new(Response::new()),
        Some(h) => Box::new(Response::new_with_default_header(h)),
    };

    if is_tls {
        resp.can_keep_alive(false);
    }

    resp
}

fn write_to_stream(mut stream: Stream, response: &mut Response) -> ExecCode {
    let s_clone = if response.to_keep_alive() {
        match  stream.try_clone() {
            Ok(s) => Some(s),
            _ => None,
        }
    } else {
        None
    };

    let mut writer = BufWriter::new(&mut stream);

    // Serialize the header to the stream
    response.write_header(&mut writer);

    // Blank line to indicate the end of the response header
    write_to_buff(&mut writer, &HEADER_END);

    // If header only, we're done
    if response.is_header_only() {
        return flush_buffer(&mut writer);
    }

    if let Some(s) = s_clone {
        // serialize_trunked_body will block until all the keep-alive i/o are done
        response.keep_long_conn(s, &mut writer);
    } else {
        // else, write the body to the stream
        response.write_body(&mut writer);

        // flush the buffer and shutdown the connection: we're done; no need for explicit shutdown
        // the stream as it's dropped automatically on out-of-the-scope.
        let mut retry: u8 = 0;
        while retry < FLUSH_RETRY {
            retry += 1;
            if flush_buffer(&mut writer) == 0 {
                break;
            }
        }
    }

    // regardless of buffer being flushed, close the stream now.
    stream_shutdown(writer.get_mut())
}

fn stream_shutdown(stream: &mut Stream) -> u8 {
    if let Err(err) = stream.shutdown(Shutdown::Both) {
        debug::print(
            &format!(
                "Encountered errors while shutting down the trunked body stream: {}",
                err
            ),
            InfoLevel::Warning,
        );
        return 1;
    }

    0
}

fn recv_request(stream: &mut Stream) -> Result<(RouteHandler, Box<Request>), ConnError> {
    //TODO/P1: continuous read from this stream until we shall close it up.
    //      use channel to request
    let raw = read_content(stream)?;
    let trimmed = raw.trim_matches(|c| c == '\r' || c == '\n' || c == '\u{0}');

    if trimmed.is_empty() {
        return Err(ConnError::EmptyRequest);
    }

    let mut request = Box::new(Request::new());
    let result = parse_request(trimmed, &mut request);

    if result.is_none() {
        return Err(ConnError::ServiceUnavailable)
    }

    if let Ok(client) = stream.peer_addr() {
        request.set_client(client);
    }

    if let Some(auth) = Route::get_auth_func() {
        if !auth(&request, &request.uri) {
            return Err(ConnError::AccessDenied);
        }
    }

    Ok((result, request))
}

fn read_content(stream: &mut Stream) -> Result<String, ConnError> {
    let mut buffer = [0u8; 512];
    let mut raw_req = String::with_capacity(512);

    loop {
        match stream.read(&mut buffer) {
            Ok(len) => {
                if len == 0 && raw_req.is_empty() {
                    // if the request is a mere handshake with no request data, we return
                    return Err(ConnError::HeartBeat);
                }

                if let Ok(req_slice) = str::from_utf8(&buffer[..len]) {
                    if len < 512 {
                        // trim end if we're at the end of the request stream
                        raw_req.push_str(
                            req_slice.trim_end_matches(|c| c == '\r' || c == '\n' || c == '\u{0}')
                        );

                        return Ok(raw_req);
                    } else {
                        // if there are more to read, don't trim and continue
                        raw_req.push_str(req_slice);
                    }
                } else {
                    debug::print("Failed to parse the request stream", InfoLevel::Warning);
                    return Err(ConnError::ReadStreamFailure);
                }
            },
            Err(e) => {
                debug::print(
                    &format!("Reading stream disconnected -- {}", e),
                    InfoLevel::Warning,
                );

                return Err(ConnError::ReadStreamFailure);
            },
        };
    }
}

fn parse_request(request: &str, store: &mut Box<Request>) -> RouteHandler {
    if request.is_empty() {
        return RouteHandler::default();
    }

    let mut res = RouteHandler::default();
    let mut baseline_chan = None;
    let mut remainder_chan = None;

    for (index, info) in request.trim().splitn(2, "\r\n").enumerate() {
        match index {
            0 => baseline_chan = parse_baseline(&info, store),
            1 => {
                let remainder: String = info.to_owned();
                if !remainder.is_empty() {
                    let (tx_remainder, rx_remainder) = channel::bounded(1);

                    let mut header: HashMap<String, String> = HashMap::new();
                    let mut cookie: HashMap<String, String> = HashMap::new();
                    let mut body: Vec<String> = Vec::with_capacity(64);

                    shared_pool::run(
                        move || {
                            let mut is_body = false;

                            for line in remainder.lines() {
                                if line.is_empty() && !is_body {
                                    // meeting the empty line dividing header and body
                                    is_body = true;
                                    continue;
                                }

                                parse_headers(
                                    line,
                                    is_body,
                                    &mut header,
                                    &mut cookie,
                                    &mut body,
                                );
                            }

                            if tx_remainder.send((header, cookie, body)).is_err() {
                                debug::print(
                                    "Unable to construct the remainder of the request.",
                                    InfoLevel::Error,
                                );
                            }
                        },
                        TaskType::Request,
                    );

                    remainder_chan = Some(rx_remainder)
                }
            }
            _ => break,
        }
    }

    if let Some(rx) = baseline_chan {
        if let Ok(result) = rx.recv_timeout(Duration::from_millis(128)) {
            res = result.0;
            if res.is_some() {
                store.create_param(result.1);
            }
        }

        if let Some(chan) = remainder_chan {
            if let Ok((header, cookie, body)) = chan.recv_timeout(Duration::from_secs(8)) {
                store.set_headers(header);
                store.set_cookies(cookie);
                store.set_bodies(body);
            }
        }
    }

    res
}

fn parse_baseline(source: &str, req: &mut Box<Request>) -> BaseLine {
    let mut raw_scheme = String::new();
    let mut raw_fragment = String::new();

    for (index, info) in source.split_whitespace().enumerate() {
        if index < 2 && info.is_empty() {
            return None;
        }

        match index {
            0 => {
                let base_method = match &info.to_uppercase()[..] {
                    "GET" => REST::GET,
                    "PUT" => REST::PUT,
                    "POST" => REST::POST,
                    "DELETE" => REST::DELETE,
                    "OPTIONS" => REST::OPTIONS,
                    _ => REST::OTHER(info.to_uppercase())
                };

                req.method = base_method;
            }
            1 => split_path(info, &mut req.uri, &mut raw_scheme, &mut raw_fragment),
            2 => req.write_header("HTTP_VERSION", info, true),
            _ => {
                break;
            }
        };
    }

    if !req.uri.is_empty() {
        let uri = req.uri.to_owned();
        let req_method = req.method.clone();

        let (tx, rx) = channel::bounded(1);
        shared_pool::run(
            move || Route::seek(&req_method, &uri, tx),
            TaskType::Request,
        );

        // now do more work on non-essential parsing
        if !raw_fragment.is_empty() {
            req.set_fragment(raw_fragment);
        }

        if !raw_scheme.is_empty() {
            req.create_scheme(parse_scheme(raw_scheme));
        }

        return Some(rx);
    }

    None
}

fn parse_headers(
    line: &str,
    is_body: bool,
    header: &mut HashMap<String, String>,
    cookie: &mut HashMap<String, String>,
    body: &mut Vec<String>,
)
{
    if !is_body {
        let mut header_key: &str = "";
        let mut is_cookie = false;

        for (idx, info) in line.trim().splitn(2, ':').enumerate() {
            match idx {
                0 => {
                    header_key = &info.trim()[..];
                    is_cookie = header_key.eq("cookie");
                }
                1 => {
                    if is_cookie {
                        parse_cookie(info.trim(), cookie);
                    } else if !header_key.is_empty() {
                        header.add(header_key, info.trim().to_owned(), true);
                    }
                }
                _ => break,
            }
        }
    } else {
        body.push(line.to_owned());
    }
}

fn split_path(source: &str, path: &mut String, scheme: &mut String, frag: &mut String) {
    let uri = source.trim().trim_end_matches('/');
    if uri.is_empty() {
        path.push('/');
        return;
    }

    let mut uri_parts: Vec<&str> = uri.rsplitn(2, '/').collect();

    // parse fragment out
    if let Some(pos) = uri_parts[0].find('#') {
        let (remains, raw_frag) = uri_parts[0].split_at(pos);
        uri_parts[0] = remains;

        if !raw_frag.is_empty() {
            frag.push_str(raw_frag);
        }
    }

    // parse scheme out
    if let Some(pos) = uri_parts[0].find('?') {
        let (remains, raw_scheme) = uri_parts[0].split_at(pos);
        uri_parts[0] = remains;

        if uri_parts[1].is_empty() {
            path.push('/');
            path.push_str(uri_parts[0]);
        } else {
            path.push_str(uri_parts[1]);
            path.push('/');
            path.push_str(uri_parts[0]);
        };

        scheme.push_str(raw_scheme.trim());
    } else {
        let uri_len = uri.len();
        if uri_len > 1 && uri.ends_with('/') {
            path.push_str(&uri[..uri_len - 1]);
        } else {
            path.push_str(uri)
        };
    }
}

/// Cookie parser will parse the request header's cookie field into a hash-map, where the
/// field is the key of the map, which map to a single value of the key from the Cookie
/// header field. Assuming no duplicate cookie keys, or the first cookie key-value pair
/// will be stored.
fn parse_cookie(raw: &str, cookie: &mut HashMap<String, String>) {
    if raw.is_empty() {
        return;
    }

    for set in raw.trim().split(';') {
        let pair: Vec<&str> = set.trim().splitn(2, '=').collect();
        if pair.len() == 2 {
            cookie.add(pair[0].trim(), pair[1].trim().to_owned(), false);
        } else if !pair.is_empty() {
            cookie.add(pair[0].trim(), String::new(), false);
        }
    }
}

fn parse_scheme(scheme: String) -> HashMap<String, Vec<String>> {
    let mut scheme_result: HashMap<String, Vec<String>> = HashMap::new();
    for (_, kv_pair) in scheme.trim().split('&').enumerate() {
        let store: Vec<&str> = kv_pair.trim().splitn(2, '=').collect();

        if !store.is_empty() {
            let key = store[0].trim();
            let val = if store.len() == 2 {
                store[1].trim().to_owned()
            } else {
                String::new()
            };

            if scheme_result.contains_key(key) {
                if let Some(val_vec) = scheme_result.get_mut(key) {
                    val_vec.push(val);
                }
            } else {
                scheme_result.insert(key.to_owned(), vec![val]);
            }
        }
    }

    scheme_result
}

fn build_err_response(err_status: u16) -> Response {
    let mut resp = Response::new();

    resp.status(err_status);
    resp.validate_and_update();
    resp.keep_alive(false);

    if resp.get_content_type().is_empty() {
        resp.set_content_type("text/html");
    }

    resp
}
