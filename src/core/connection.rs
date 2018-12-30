use std::collections::HashMap;
use std::io::prelude::*;
use std::io::BufWriter;
use std::net::{Shutdown, TcpStream};
use std::sync::mpsc;
use std::time::Duration;
use super::config::ConnMetadata;
use super::router::{Callback, Route, RouteHandler, REST};
use super::http::{
    Request, RequestWriter, Response, ResponseManager, ResponseStates, ResponseWriter,
};

use crate::support::{
    common::flush_buffer, common::write_to_buff, common::MapUpdates, debug, shared_pool, TaskType
};

static HEADER_END: [u8; 2] = [13, 10];
type ExecCode = u8;

#[derive(PartialEq, Eq, Clone, Copy)]
enum ConnError {
    EmptyRequest,
    ReadStreamFailure,
    AccessDenied,
    ServiceUnavailable,
}

//TODO: switch 'mpsc' to 'crossbeam-channel'

//TODO: still good for implementing middlewear
//pub fn handle_connection_with_states<T: Send + Sync + Clone + StatesProvider>(
//        stream: TcpStream,
//        router: Arc<Route>,
//        metadata: Arc<ConnMetadata>,
//        states: Arc<RwLock<T>>) -> Option<u8> {
//
//    let mut request = Box::new(Request::new());
//    let handler = match parse_request(&stream, &mut request, router) {
//        Err(err) => {
//            debug::print("Error on parsing request", 3);
//            return write_to_stream(stream, &build_err_response(&err, &metadata));
//        },
//        Ok(cb) => cb,
//    };
//
//    match metadata.get_state_interaction() {
//        &StatesInteraction::WithRequest | &StatesInteraction::Both => {
//            let require_updates = match states.read() {
//                Ok(s) => s.on_request(&mut request),
//                _ => false,
//            };
//
//            if require_updates {
//                if let Ok(mut s) = states.write() {
//                    s.update(&request, None);
//                }
//            }
//        },
//        _ => { /* Nothing */ },
//    };
//
//    let mut response = initialize_response(&metadata);
//    let result = handle_response(stream, handler, &request, &mut response, &metadata);
//
//    match metadata.get_state_interaction() {
//        &StatesInteraction::WithRequest | &StatesInteraction::Both => {
//            let require_updates = match states.read() {
//                Ok(s) => s.on_response(&mut response),
//                _ => false,
//            };
//
//            if require_updates {
//                if let Ok(mut s) = states.write() {
//                    s.update(&request, Some(&response));
//                }
//            }
//        },
//        _ => { /* Nothing */ },
//    };
//
//    result
//}

pub(crate) fn handle_connection(stream: TcpStream) -> ExecCode {
    let mut request = Box::new(Request::new());

    let handler = match parse_request(&stream, &mut request) {
        Err(err) => {
            debug::print("Error on parsing request", 3);

            let status: u16 = match err {
                ConnError::EmptyRequest => 400,
                ConnError::AccessDenied => 401,
                ConnError::ServiceUnavailable => 404,
                ConnError::ReadStreamFailure => 500,
            };

            return write_to_stream(&stream, &mut build_err_response(status));
        }
        Ok(cb) => cb,
    };

    handle_response(stream,handler, &request, &mut initialize_response())
}

pub(crate) fn send_err_resp(stream: TcpStream, err_code: u16) -> ExecCode {
    return write_to_stream(&stream, &mut build_err_response(err_code));
}

fn handle_response(
    stream: TcpStream,
    callback: Callback,
    request: &Box<Request>,
    response: &mut Box<Response>
) -> ExecCode {
    match request.header("connection") {
        Some(ref val) if val.eq(&String::from("close")) => response.can_keep_alive(false),
        _ => response.can_keep_alive(true),
    };

    if request.method.eq(&REST::OTHER(String::from("HEAD"))) {
        response.header_only(true);
    }

    Route::parse_request(callback, request, response);
    response.validate_and_update();

    write_to_stream(&stream, response)
}

fn initialize_response() -> Box<Response> {
    let header = ConnMetadata::get_default_header();
    match header {
        None => Box::new(Response::new()),
        Some(h) => Box::new(Response::new_with_default_header(h)),
    }
}

fn write_to_stream(stream: &TcpStream, response: &mut Box<Response>) -> ExecCode {
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
        return flush_buffer(&mut writer);
    }

    if let Ok(clone) = stream.try_clone() {
        // serialize_trunked_body will block until all the keep-alive i/o are done
        response.keep_long_conn(clone, &mut writer);
    }

    // trunked keep-alive i/o is done, shut down the stream for good since copies
    // can be listening on read/write
    if let Err(err) = stream.shutdown(Shutdown::Both) {
        debug::print(
            &format!(
                "Encountered errors while shutting down the trunked body stream: {}",
                err
            ),
            1,
        );
        return 1;
    }

    // Otherwise we're good to leave.
    0
}

fn parse_request(
    mut stream: &TcpStream,
    request: &mut Box<Request>,
) -> Result<Callback, ConnError> {
    let mut buffer = [0; 1024];

    if let Err(e) = stream.read(&mut buffer) {
        debug::print(&format!("Reading stream error -- {}", e), 3);
        Err(ConnError::ReadStreamFailure)
    } else {
        let request_raw = String::from_utf8_lossy(&buffer[..]);

        if request_raw.is_empty() {
            return Err(ConnError::EmptyRequest);
        }

        let auth_func = Route::get_auth_func();
        let callback =
            deserialize(request_raw.to_string(), request);

        let host = request.header("host");
        if let Some(host_name) = host {
            request.set_host(host_name);
        }

        if let Ok(client) = stream.peer_addr() {
            request.set_client(client);
        }

        if let Some(auth) = auth_func {
            let route_path = request.uri.to_owned();
            if !auth(&request, route_path) {
                return Err(ConnError::AccessDenied);
            }
        }

        if let Some(callback) = callback {
            Ok(callback)
        } else {
            Err(ConnError::ServiceUnavailable)
        }
    }
}

fn deserialize(request: String, store: &mut Box<Request>) -> Option<Callback> {
    if request.is_empty() {
        return None;
    }

    debug::print(&format!("\r\nPrint request: \r\n{}", request), 2);

    let mut res = None;
    let raw_lines: Vec<&str> = request.trim().splitn(2, '\n').collect();

    let base_line = match raw_lines.get(0) {
        Some(line) => line.trim(),
        _ => return None,
    };

    if let Some(rx) = deserialize_base_line(base_line, store) {

        let req_chan =
            if raw_lines.len() > 1 {
                let remainder = raw_lines[1].to_owned();
                let (tx_remainder, rx_remainder) = mpsc::channel();

                let mut header: HashMap<String, String> = HashMap::new();
                let mut cookie: HashMap<String, String> = HashMap::new();
                let mut body: Vec<String> = Vec::new();

                shared_pool::run(move || {
                    let mut is_body = false;

                    for line in remainder.lines() {
                        if line.is_empty() && !is_body {
                            // meeting the empty line dividing header and body
                            is_body = true;
                            continue;
                        }

                        deserialize_headers(line, is_body, &mut header, &mut cookie, &mut body);
                    }

                    if let Err(e) = tx_remainder.send((header, cookie, body)) {
                        eprintln!("Unable to construct the remainder of the request.");
                    }
                }, TaskType::Request);

                Some(rx_remainder)
            } else {
                None
            };

        if let Ok(route_info) = rx.recv_timeout(Duration::from_millis(128)) {
            if route_info.0.is_some() {
                store.create_param(route_info.1);
            }

            res = route_info.0;
        }

        if let Some(chan) = req_chan {
            if let Ok((header, cookie, body)) = chan.recv_timeout(Duration::from_secs(8)) {
                store.set_headers(header);
                store.set_cookies(cookie);
                store.set_bodies(body);
            }
        }
    }

    res
}

pub(crate) fn deserialize_base_line(
    source: &str,
    req: &mut Box<Request>
) -> Option<mpsc::Receiver<(Option<Callback>, HashMap<String, String>)>>
{
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

        let (tx, rx) = mpsc::channel();
        shared_pool::run(move || {
            Route::seek_handler(&req_method, &uri, header_only, tx);
        },TaskType::Request);

        // now do more work on non-essential parsing
        if !raw_fragment.is_empty() {
            req.set_fragment(raw_fragment);
        }

        if !raw_scheme.is_empty() {
            req.create_scheme(scheme_parser(raw_scheme));
        }

        return Some(rx);
    }

    None
}

fn deserialize_headers(
    line: &str, is_body: bool, header: &mut HashMap<String, String>,
    cookie: &mut HashMap<String, String>, body: &mut Vec<String>)
{
    if !is_body {
        let header_info: Vec<&str> = line.trim().splitn(2, ':').collect();

        if header_info.len() == 2 {
            let header_key = &header_info[0].trim().to_lowercase()[..];
            if header_key.eq("cookie") {
                cookie_parser(cookie, header_info[1].trim());
            } else {
                header.add(header_key, header_info[1].trim().to_owned(), true);
            }
        }
    } else {
        body.push(line.to_owned());
    }
}

fn split_path(
    full_uri: &str,
    final_uri: &mut String,
    final_scheme: &mut String,
    final_frag: &mut String,
) {
    let uri = full_uri.trim();
    if uri.is_empty() {
        final_uri.push_str("/");
        return;
    }

    let mut uri_parts: Vec<&str> = uri.rsplitn(2, "/").collect();

    // parse fragment out
    if let Some(pos) = uri_parts[0].find("#") {
        let (remains, frag) = uri_parts[0].split_at(pos);
        uri_parts[0] = remains;

        if !frag.is_empty() {
            final_frag.push_str(frag);
        }
    }

    // parse scheme out
    if let Some(pos) = uri_parts[0].find("?") {
        let (remains, scheme) = uri_parts[0].split_at(pos);
        uri_parts[0] = remains;

        if uri_parts[1].is_empty() {
            final_uri.push_str(&format!("/{}", uri_parts[0])[..]);
        } else {
            final_uri.push_str(&format!("{}/{}", uri_parts[1], uri_parts[0])[..]);
        };

        final_scheme.push_str(scheme.trim());
    } else {
        let uri_len = uri.len();
        if uri_len > 1 && uri.ends_with("/") {
            final_uri.push_str(&uri[..uri_len - 1]);
        } else {
            final_uri.push_str(uri)
        };
    }
}

/// Cookie parser will parse the request header's cookie field into a hash-map, where the
/// field is the key of the map, which map to a single value of the key from the Cookie
/// header field. Assuming no duplicate cookie keys, or the first cookie key-value pair
/// will be stored.
fn cookie_parser(cookie: &mut HashMap<String, String>, cookie_body: &str) {
    if cookie_body.is_empty() {
        return;
    }

    for set in cookie_body.trim().split(";").into_iter() {
        let pair: Vec<&str> = set.trim().splitn(2, "=").collect();
        if pair.len() == 2 {
            cookie.add(pair[0].trim(), pair[1].trim().to_owned(), false);
        } else if pair.len() > 0 {
            cookie.add(pair[0].trim(), String::from(""), false);
        }
    }
}

fn scheme_parser(scheme: String) -> HashMap<String, Vec<String>> {
    let mut scheme_result: HashMap<String, Vec<String>> = HashMap::new();
    for (_, kv_pair) in scheme.trim().split("&").enumerate() {
        let store: Vec<&str> = kv_pair.trim().splitn(2, "=").collect();

        if store.len() > 0 {
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

fn build_err_response(err_status: u16) -> Box<Response> {
    let mut resp = Box::new(Response::new());

    resp.status(err_status);
    resp.validate_and_update();
    resp.keep_alive(false);

    if resp.get_content_type().is_empty() {
        resp.set_content_type("text/html");
    }

    resp
}