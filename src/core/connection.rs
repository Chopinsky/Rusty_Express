#![allow(unused_variables)]

use std::collections::HashMap;
use std::io::prelude::*;
use std::io::BufWriter;
use std::net::{TcpStream, Shutdown};
use std::sync::{Arc, RwLock, mpsc};
use std::time::Duration;

use core::config::ConnMetadata;
use core::states::{StatesProvider, StatesInteraction};
use core::http::{Request, RequestWriter, Response, ResponseStates, ResponseWriter, ResponseStreamer};
use core::router::{REST, Route, RouteHandler};
use support::shared_pool;

#[derive(PartialEq, Eq, Clone, Copy)]
enum ParseError {
    EmptyRequestErr,
    ReadStreamErr,
    WriteStreamErr,
}

struct RequestBase {
    method: Option<REST>,
    uri: String,
    http_version: String,
    scheme: Option<HashMap<String, Vec<String>>>,
}

pub fn handle_connection_with_states<T: Send + Sync + Clone + StatesProvider>(
        stream: TcpStream,
        router: Arc<Route>,
        metadata: Arc<ConnMetadata>,
        states: Arc<RwLock<T>>) -> Option<u8> {

    let mut request = Request::new();
    if let Err(err) = handle_request(&stream, &mut request) {
        eprintln!("Error on parsing request");
        return write_to_stream(stream,
                               build_err_response(&err,metadata.get_default_pages()),
                               false);
    }

    match metadata.get_state_interaction() {
        &StatesInteraction::WithRequest | &StatesInteraction::Both => {

        },
        _ => { /* Nothing */ },
    }

    let result = handle_response(stream, request, router, &metadata);

    match metadata.get_state_interaction() {
        &StatesInteraction::WithRequest | &StatesInteraction::Both => {

        },
        _ => { /* Nothing */ },
    }

    result
}

pub fn handle_connection(
        stream: TcpStream,
        router: Arc<Route>,
        metadata: Arc<ConnMetadata>) -> Option<u8> {

    let mut request= Request::new();
    if let Err(err) = handle_request(&stream, &mut request) {
        eprintln!("Error on parsing request");
        return write_to_stream(stream,
                               build_err_response(&err,metadata.get_default_pages()),
                               false);
    }

    handle_response(stream, request, router, &metadata)
}

fn handle_response(stream: TcpStream, request: Request, router: Arc<Route>, conn_handler: &Arc<ConnMetadata>) -> Option<u8> {
    match get_response_with_fallback(&request, &router,
                                 &conn_handler.get_default_header(),
                                 &conn_handler.get_default_pages()) {
        Ok(response) => {
            let ignore_body =
                match request.method {
                    Some(REST::OTHER(other_method)) => other_method.eq("head"),
                    _ => false,
                };

            write_to_stream(stream, response, ignore_body)
        },
        Err(e) => {
            println!("Error on generating response -- {}", e);
            write_to_stream(stream,
                                   build_err_response(&ParseError::WriteStreamErr, conn_handler.get_default_pages()),
                                   false)
        },
    }
}

fn write_to_stream(stream: TcpStream, response: Response, ignore_body: bool) -> Option<u8> {
    let mut buffer = BufWriter::new(&stream);

    response.serialize_header(&mut buffer, ignore_body);
    if !ignore_body { response.serialize_body(&mut buffer); }

    if let Err(e) = buffer.flush() {
        println!("An error has taken place when flushing the response to the stream: {}", e);
        return Some(1);
    }

    if response.to_close_connection() {
        // Told to close the connection, shut down the socket now.
        if let Ok(_) = stream.shutdown(Shutdown::Both) {
            return Some(0);
        }
    } else {
        // Otherwise we're good to leave.
        return Some(0);
    }

    return Some(1);
}

fn handle_request(mut stream: &TcpStream, request: &mut Request) -> Result<(), ParseError> {
    let mut buffer = [0; 512];

    if let Ok(_) = stream.read(&mut buffer) {
        let request_raw = String::from_utf8_lossy(&buffer[..]);
        if request_raw.is_empty() {
            return Err(ParseError::EmptyRequestErr);
        }

        if !parse_request(&request_raw, request) {
            Err(ParseError::EmptyRequestErr)
        } else {
            return Ok(());
        }
    } else {
        Err(ParseError::ReadStreamErr)
    }
}

fn parse_request(request: &str, store: &mut Request) -> bool {
    if request.is_empty() {
        return false;
    }

    //println!("{}", request);

    let (tx_base, rx_base) = mpsc::channel();
    let (tx_cookie, rx_cookie) = mpsc::channel();

    let mut is_body = false;
    for (num, line) in request.trim().lines().enumerate() {
        if num == 0 {
            if line.is_empty() { continue; }

            let val = line.to_owned();
            let tx_clone = mpsc::Sender::clone(&tx_base);

            shared_pool::run(move || {
                parse_request_base(val, tx_clone);
            })

        } else {
            if line.is_empty() {
                // meeting the empty line dividing header and body
                is_body = true;
                continue;
            }

            parse_request_body(store, line, &tx_cookie, is_body);

        }
    }

    /* Since we don't move the tx but cloned them, need to drop them
     * specifically here, or we would hang forever before getting the
     * messages back.
     */
    drop(tx_base);
    drop(tx_cookie);

    if let Ok(base) = rx_base.recv_timeout(Duration::from_millis(200)) {
        store.method = base.method;
        store.uri = base.uri;
        store.write_header("http_version", &base.http_version, true);

        if let Some(s) = base.scheme {
            store.create_scheme(s);
        }
    }

    let mut cookie_created = false;
    for cookie in rx_cookie {
        if cookie_created {
            for pair in cookie.iter() {
                store.set_cookie(&pair.0[..], &pair.1[..], false);
            };
        } else {
            store.create_cookie(cookie);
            cookie_created = true;
        }
    }

    true
}

fn parse_request_body(store: &mut Request, line: &str, tx_cookie: &mpsc::Sender<HashMap<String, String>>, is_body: bool) {
    if !is_body {
        let header_info: Vec<&str> = line.trim().splitn(2, ':').collect();

        if header_info.len() == 2 {
            if header_info[0].trim().to_lowercase().eq("cookie") {
                let cookie_body = header_info[1].to_owned();
                let tx_clone = mpsc::Sender::clone(&tx_cookie);

                shared_pool::run(move || {
                    cookie_parser(cookie_body, tx_clone);
                });

            } else {
                store.write_header(
                    &header_info[0].trim().to_lowercase(),
                    header_info[1].trim(),
                    true
                );
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
    let mut scheme = None;

    for (index, info) in line.split_whitespace().enumerate() {
        match index {
            0 => {
                method = match &info[..] {
                    "GET" => Some(REST::GET),
                    "PUT" => Some(REST::PUT),
                    "POST" => Some(REST::POST),
                    "DELETE" => Some(REST::DELETE),
                    "OPTIONS" => Some(REST::OPTIONS),
                    "" => None,
                    _ => Some(REST::OTHER(info.to_lowercase().to_owned())),
                };
            },
            1 => {
                let (req_uri, req_scheme) = split_path(info);

                if !req_uri.is_empty() { uri = req_uri; }
                if !req_scheme.is_empty() {
                    scheme = scheme_parser(&req_scheme[..]);
                }
            },
            2 => {
                http_version.push_str(info);
            },
            _ => { break; },
        };
    }

    tx.send(RequestBase {
        method,
        uri,
        http_version,
        scheme,
    }).unwrap_or_else(|e| {
        eprintln!("Unable to parse base request: {}", e);
    });
}

fn get_response_with_fallback(
        request_info: &Request,
        router: &Route,
        header: &HashMap<String, String>,
        fallback: &HashMap<u16, String>
    ) -> Result<Response, String> {

    let mut resp =
        if header.is_empty() {
            Response::new()
        } else {
            Response::new_with_default_header(&header)
        };

    match request_info.method {
        None => {
            return Err(String::from("Invalid request method"));
        },
        _ => {
            router.handle_request_method(&request_info, &mut resp);
        }
    }

    resp.check_and_update(&fallback);
    Ok(resp)
}

fn split_path(full_uri: &str) -> (String, String) {
    let uri = full_uri.trim();
    if uri.is_empty() {
        return (String::from("/"), String::new());
    }

    let mut uri_parts: Vec<&str> = uri.rsplitn(2, "/").collect();
    if let Some(pos) = uri_parts[0].find("?") {
        let (last_uri_pc, scheme) = uri_parts[0].split_at(pos);
        uri_parts[0] = last_uri_pc;

        let result_uri =
            if uri_parts[1].is_empty() {
                format!("/{}", uri_parts[0])
            } else {
                format!("{}/{}", uri_parts[1], uri_parts[0])
            };

        (result_uri, scheme.trim().to_owned())
    } else {
        let uri_len = uri.len();
        let result_uri =
            if uri_len > 1 && uri.ends_with("/") {
                uri[..uri_len-1].to_owned()
            } else {
                uri.to_owned()
            };

        (result_uri, String::new())
    }
}

/// Cookie parser will parse the request header's cookie field into a hash-map, where the
/// field is the key of the map, which map to a single value of the key from the Cookie
/// header field. Assuming no duplicate cookie keys, or the first cookie key-value pair
/// will be stored.
fn cookie_parser(cookie_body: String, tx: mpsc::Sender<HashMap<String, String>>) { //cookie: &mut HashMap<String, String>) {
    if cookie_body.is_empty() { return; }

    let mut cookie = HashMap::new();
    for set in cookie_body.trim().split(";").into_iter() {
        let pair: Vec<&str> = set.trim().splitn(2, "=").collect();
        if pair.len() == 2 {
            cookie.entry(pair[0].trim().to_owned())
                .or_insert(pair[1].trim().to_owned());
        } else if pair.len() > 0 {
            cookie.entry(pair[0].trim().to_owned())
                .or_insert(String::new());
        }
    }

    if let Err(e) = tx.send(cookie) {
        println!("Unable to parse base request cookies: {}", e);
    }
}

fn scheme_parser(scheme: &str) -> Option<HashMap<String, Vec<String>>> {
    let mut scheme_result: HashMap<String, Vec<String>> = HashMap::new();
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

    Some(scheme_result)
}

fn build_err_response(err: &ParseError, default_pages: &HashMap<u16, String>) -> Response {
    let mut resp = Response::new();
    let status: u16 = match err {
        &ParseError::WriteStreamErr | &ParseError::ReadStreamErr => 500,
        _ => 404,
    };

    resp.status(status);
    resp.check_and_update(&default_pages);

    resp
}
