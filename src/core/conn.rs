#![allow(clippy::borrowed_box)]

use std::io::prelude::*;
use std::io::BufWriter;
use std::net::{Shutdown, TcpStream};
use std::str;
use std::time::Duration;

use crate::channel;
use crate::core::config::ConnMetadata;
use crate::core::http::{
    Request, RequestWriter, Response, ResponseManager, ResponseStates, ResponseWriter,
};
use crate::core::router::{Route, RouteSeeker, REST, RouteHandler};
use crate::hashbrown::HashMap;
use crate::support::{
    common::flush_buffer, common::write_to_buff, common::MapUpdates, debug, debug::InfoLevel,
    shared_pool, TaskType,
};

static HEADER_END: [u8; 2] = [13, 10];
static FLUSH_RETRY: u8 = 4;

type ExecCode = u8;
type BaseLine = Option<channel::Receiver<(RouteHandler, HashMap<String, String>)>>;

#[derive(PartialEq, Eq, Clone, Copy)]
enum ConnError {
    EmptyRequest,
    ReadStreamFailure,
    AccessDenied,
    ServiceUnavailable,
}

pub(crate) fn handle_connection(stream: TcpStream) -> ExecCode {
    let mut request = Box::new(Request::new());

    let callback = match parse_request(&stream, &mut request) {
        Err(err) => {
            let status: u16 = match err {
                ConnError::EmptyRequest => 400,
                ConnError::AccessDenied => 401,
                ConnError::ServiceUnavailable => 404,
                ConnError::ReadStreamFailure => {
                    // connection is sour, shutdown now
                    if let Err(err) = stream.shutdown(Shutdown::Both) {
                        return 1;
                    }

                    return 0;
                }
            };

            debug::print(
                &format!("Error on parsing request: {}", status),
                InfoLevel::Error,
            );

            return write_to_stream(&stream, &mut build_err_response(status));
        }
        Ok(cb) => cb,
    };

    handle_response(stream, callback, request, initialize_response())
}

#[inline]
pub(crate) fn send_err_resp(stream: TcpStream, err_code: u16) -> ExecCode {
    write_to_stream(&stream, &mut build_err_response(err_code))
}

fn handle_response(
    stream: TcpStream,
    mut callback: RouteHandler,
    request: Box<Request>,
    mut response: Box<Response>,
) -> ExecCode
{
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

    write_to_stream(&stream, &mut response)
}

fn initialize_response() -> Box<Response> {
    let header = ConnMetadata::get_default_header();
    match header {
        None => Box::new(Response::new()),
        Some(h) => Box::new(Response::new_with_default_header(h)),
    }
}

fn write_to_stream(stream: &TcpStream, response: &mut Response) -> ExecCode {
    let mut writer = BufWriter::new(stream);

    // Serialize the header to the stream
    response.write_header(&mut writer);

    // Blank line to indicate the end of the response header
    write_to_buff(&mut writer, &HEADER_END);

    // If header only, we're done
    if response.is_header_only() {
        return flush_buffer(&mut writer);
    }

    if !response.to_keep_alive() {
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

        // regardless of buffer being flushed, close the stream now.
        return stream_shutdown(&stream);
    }

    if let Ok(clone) = stream.try_clone() {
        // serialize_trunked_body will block until all the keep-alive i/o are done
        response.keep_long_conn(clone, &mut writer);
    }

    // trunked keep-alive i/o is done, shut down the stream for good since copies
    // can be listening on read/write
    stream_shutdown(&stream)
}

fn stream_shutdown(stream: &TcpStream) -> u8 {
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

fn parse_request(stream: &TcpStream, request: &mut Box<Request>) -> Result<RouteHandler, ConnError> {
    let mut raw = String::with_capacity(512); // 512 -- default buffer size
    if let Some(e) = read_stream(stream, &mut raw) {
        return Err(e);
    };

    if raw.trim_matches(|c| c == '\r' || c == '\n' || c == '\u{0}').is_empty() {
        return Err(ConnError::EmptyRequest);
    }

    let result = deserialize(raw, request);
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

    Ok(result)
}

fn read_stream(mut stream: &TcpStream, raw_req: &mut String) -> Option<ConnError> {
    let mut buffer = [0u8; 512];

    loop {
        match stream.read(&mut buffer) {
            Ok(len) => {
                if let Ok(req_slice) = str::from_utf8(&buffer[..len]) {
                    raw_req.push_str(req_slice);
                } else {
                    debug::print("Failed to parse the request stream", InfoLevel::Warning);
                    return Some(ConnError::ReadStreamFailure);
                }

                if len < 512 {
                    break;
                } else {
                    // possibly to have more to read, clear the buffer and load it again
                    buffer.iter_mut().for_each(|val| {
                        *val = 0;
                    });
                }
            },
            Err(e) => {
                debug::print(
                    &format!("Reading stream disconnected -- {}", e),
                    InfoLevel::Warning,
                );

                return Some(ConnError::ReadStreamFailure);
            },
        };
    }

    None
}

fn deserialize(request: String, store: &mut Box<Request>) -> RouteHandler {
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

pub(crate) fn parse_baseline(source: &str, req: &mut Box<Request>) -> BaseLine {
    let mut header_only = false;
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
                    _ => {
                        let others = info.to_uppercase();
                        if others.eq(&String::from("HEADER")) {
                            header_only = true;
                        }

                        REST::OTHER(others)
                    }
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
            move || {
                Route::seek(&req_method, &uri, header_only, tx);
            },
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
