#![allow(unused_variables)]

use std::collections::HashMap;
use std::io::prelude::*;
use std::io::BufWriter;
use std::net::{Shutdown, TcpStream};
use std::sync::{Arc, RwLock, mpsc};
use std::time::Duration;

use core::config::ConnMetadata;
use core::states::{StatesProvider, StatesInteraction};
use core::http::{Request, RequestWriter, Response, ResponseStates, ResponseWriter, StreamWriter};
use core::router::{REST, Route, RouteHandler};
use support::debug;
use support::TaskType;
use support::shared_pool;

#[derive(PartialEq, Eq, Clone, Copy)]
enum ParseError {
    EmptyRequestErr,
    ReadStreamErr,
}

struct RequestBase {
    method: Option<REST>,
    uri: String,
    http_version: String,
    scheme: HashMap<String, Vec<String>>,
}

pub fn handle_connection_with_states<T: Send + Sync + Clone + StatesProvider>(
        stream: TcpStream,
        router: Arc<Route>,
        metadata: Arc<ConnMetadata>,
        states: Arc<RwLock<T>>) -> Option<u8> {

    let mut request = Box::new(Request::new());
    if let Err(err) = handle_request(&stream, &mut request) {
        debug::print("Error on parsing request", 3);
        return write_to_stream(stream, &build_err_response(&err, &metadata), false);
    }

    match metadata.get_state_interaction() {
        &StatesInteraction::WithRequest | &StatesInteraction::Both => {
            let require_updates = match states.read() {
                Ok(s) => s.on_request(&mut request),
                _ => false,
            };

            if require_updates {
                if let Ok(mut s) = states.write() {
                    s.update(&request, None);
                }
            }
        },
        _ => { /* Nothing */ },
    };

    let mut response = initialize_response(&metadata);
    let result = handle_response(stream, &mut request, &mut response, &router, &metadata);

    match metadata.get_state_interaction() {
        &StatesInteraction::WithRequest | &StatesInteraction::Both => {
            let require_updates = match states.read() {
                Ok(s) => s.on_response(&mut response),
                _ => false,
            };

            if require_updates {
                if let Ok(mut s) = states.write() {
                    s.update(&request, Some(&response));
                }
            }
        },
        _ => { /* Nothing */ },
    };

    result
}

pub fn handle_connection(
        stream: TcpStream,
        router: Arc<Route>,
        metadata: Arc<ConnMetadata>) -> Option<u8> {

    let mut request= Box::new(Request::new());
    if let Err(err) = handle_request(&stream, &mut request) {
        debug::print("Error on parsing request", 3);
        return write_to_stream(stream, &build_err_response(&err, &metadata), false);
    }

    handle_response(stream, &mut request, &mut initialize_response(&metadata), &router, &metadata)
}

fn handle_response(stream: TcpStream, request: &mut Box<Request>, response: &mut Box<Response>,
                   router: &Arc<Route>, metadata: &Arc<ConnMetadata>) -> Option<u8> {

    let (override_method , ignore_body) = match &request.method {
        &REST::OTHER(ref others) if others.eq("head") => {
            response.header_only(true);
            (REST::GET, true)
        },
        _ => { (request.method.to_owned(), false) },
    };

    router.handle_request_method(&override_method, request, response);
    response.check_and_update(&metadata.get_default_pages());

    write_to_stream(stream, &response, ignore_body)
}

fn initialize_response(metadata: &Arc<ConnMetadata>) -> Box<Response> {
    let header = metadata.get_default_header();
    match header.is_empty() {
        true => Box::new(Response::new()),
        _ => Box::new(Response::new_with_default_header(header)),
    }
}

fn write_to_stream(stream: TcpStream, response: &Box<Response>, ignore_body: bool) -> Option<u8> {
    let mut buffer = BufWriter::new(&stream);

    response.serialize_header(&mut buffer, ignore_body);
    if !ignore_body { response.serialize_body(&mut buffer); }

    if let Err(e) = buffer.flush() {
        debug::print(
            &format!("An error has taken place when flushing the response to the stream: {}", e)[..], 1);
        return Some(1);
    }

    if let Err(e) = stream.shutdown(Shutdown::Both) {
        debug::print(
            &format!("An error has taken place when flushing the response to the stream: {}", e)[..], 1);
        return Some(1);
    }

    // Otherwise we're good to leave.
    return Some(0);
}

fn handle_request(mut stream: &TcpStream, request: &mut Box<Request>) -> Result<(), ParseError> {
    let mut buffer = [0; 1024];

    if let Err(e) = stream.read(&mut buffer){
        debug::print(&format!("Reading stream error -- {}", e), 3);
        Err(ParseError::ReadStreamErr)
    } else {
        let request_raw = String::from_utf8_lossy(&buffer[..]);
        if request_raw.is_empty() {
            return Err(ParseError::EmptyRequestErr);
        }

        if !parse_request(&request_raw, request) {
            Err(ParseError::EmptyRequestErr)
        } else {
            return Ok(());
        }
    }
}

fn parse_request(request: &str, store: &mut Box<Request>) -> bool {
    if request.is_empty() {
        return false;
    }

    debug::print(&format!("\r\nPrint request: \r\n{}", request)[..], 2);

    let mut lines = request.trim().lines();
    let (tx_base, rx_base) = mpsc::channel();

    if let Some(line) = lines.nth(0) {
        if line.is_empty() { return false; }

        let base_line = line.to_owned();
        shared_pool::run(move || {
            parse_request_base(base_line, tx_base);
        }, TaskType::Request);

    } else {
        return false;
    }

    let mut is_body = false;
    for line in lines {
        if line.is_empty() && !is_body {
            // meeting the empty line dividing header and body
            is_body = true;
            continue;
        }

        parse_request_body(store, line, is_body);
    }

    if let Ok(base) = rx_base.recv_timeout(Duration::from_millis(128)) {
        if let Some(method) = base.method {
            store.method = method;
            store.uri = base.uri;
            store.write_header("http_version", &base.http_version, true);

            if !base.scheme.is_empty() {
                store.create_scheme(base.scheme);
            }
        } else {
            return false;
        }
    }

    true
}

fn parse_request_body(store: &mut Box<Request>, line: &str, is_body: bool) {
    //TODO: Better support for content-dispositions?

    if !is_body {
        let header_info: Vec<&str> = line.trim().splitn(2, ':').collect();

        if header_info.len() == 2 {
            let header_key = &header_info[0].trim().to_lowercase()[..];
            if header_key.eq("cookie") {
                cookie_parser(store, header_info[1].trim());
            } else {
                store.write_header(header_key, header_info[1].trim(), true);
            }
        }
    } else {
        store.extend_body(line);
    }
}

fn parse_request_base(line: String, tx: mpsc::Sender<RequestBase>) {
    let mut method = None;
    let mut uri = String::new();
    let mut http_version = String::new();
    let mut scheme = HashMap::new();

    for (index, info) in line.split_whitespace().enumerate() {
        if info.is_empty() { continue; }

        match index {
            0 => {
                let base_method = match &info.to_uppercase()[..] {
                    "GET" => REST::GET,
                    "PUT" => REST::PUT,
                    "POST" => REST::POST,
                    "DELETE" => REST::DELETE,
                    "OPTIONS" => REST::OPTIONS,
                    _ => REST::OTHER(info.to_owned()),
                };

                method = Some(base_method);
            },
            1 => split_path(info, &mut uri, &mut scheme),
            2 => http_version.push_str(info),
            _ => { break; },
        };
    }

    tx.send(RequestBase {
        method,
        uri,
        http_version,
        scheme,
    }).unwrap_or_else(|e| {
        debug::print(&format!("Unable to parse base request: {}", e)[..], 1);
    });
}

fn split_path(full_uri: &str, final_uri: &mut String, final_scheme: &mut HashMap<String, Vec<String>>) {
    let uri = full_uri.trim();
    if uri.is_empty() {
        final_uri.push_str("/");
        return;
    }

    let mut uri_parts: Vec<&str> = uri.rsplitn(2, "/").collect();
    if let Some(pos) = uri_parts[0].find("?") {
        let (last_uri_pc, scheme) = uri_parts[0].split_at(pos);
        uri_parts[0] = last_uri_pc;

        if uri_parts[1].is_empty() {
            final_uri.push_str(&format!("/{}", uri_parts[0])[..]);
        } else {
            final_uri.push_str(&format!("{}/{}", uri_parts[1], uri_parts[0])[..]);
        };

        scheme_parser(scheme.trim(), final_scheme);

    } else {
        let uri_len = uri.len();
        if uri_len > 1 && uri.ends_with("/") {
            final_uri.push_str(&uri[..uri_len-1]);
        } else {
            final_uri.push_str(uri)
        };

    }
}

/// Cookie parser will parse the request header's cookie field into a hash-map, where the
/// field is the key of the map, which map to a single value of the key from the Cookie
/// header field. Assuming no duplicate cookie keys, or the first cookie key-value pair
/// will be stored.
fn cookie_parser(store: &mut Box<Request>, cookie_body: &str) {
    if cookie_body.is_empty() { return; }

    for set in cookie_body.trim().split(";").into_iter() {
        let pair: Vec<&str> = set.trim().splitn(2, "=").collect();
        if pair.len() == 2 {
            store.set_cookie(pair[0].trim(), pair[1].trim(), false);
        } else if pair.len() > 0 {
            store.set_cookie(pair[0].trim(), "", false);
        }
    }
}

fn scheme_parser(scheme: &str, scheme_result: &mut HashMap<String, Vec<String>>) {
    for (_, kv_pair) in scheme.trim().split("&").enumerate() {
        let store: Vec<&str> = kv_pair.trim().splitn(2, "=").collect();

        if store.len() > 0 {
            let key = store[0].trim();
            let val =
                if store.len() == 2 {
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
}

fn build_err_response(err: &ParseError, metadata: &Arc<ConnMetadata>) -> Box<Response> {
    let mut resp = Box::new(Response::new());
    let status: u16 = match err {
        &ParseError::ReadStreamErr => 500,
        _ => 404,
    };

    resp.status(status);
    resp.check_and_update(&metadata.get_default_pages());
    resp.keep_alive(false);

    if resp.get_content_type().is_empty() {
        resp.set_content_type("text/html");
    }

    resp
}
