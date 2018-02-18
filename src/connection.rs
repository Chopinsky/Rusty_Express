use std::collections::HashMap;
use std::io::prelude::*;
use std::net::{TcpStream, Shutdown};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
use config::ConnMetadata;
use http::*;
use router::*;

#[derive(PartialEq, Eq, Clone, Copy)]
enum ParseError {
    EmptyRequestErr,
    ReadStreamErr,
}

struct RequestBase {
    method: REST,
    uri: String,
    http_version: String,
    scheme: HashMap<String, Vec<String>>,
}

pub fn handle_connection(
        stream: TcpStream,
        router: &Route,
        conn_handler: &ConnMetadata
    ) -> Option<u8> {

    let request: Request;
    match read_request(&stream) {
        Ok(req) => {
            request = req;
        },
        Err(ParseError::ReadStreamErr) => {
            //can't read from the stream, no need to write back...
            stream.shutdown(Shutdown::Both).unwrap();
            return None;
        },
        Err(ParseError::EmptyRequestErr) => {
            println!("Error on parsing request");
            return write_to_stream(stream,
                                   build_default_response(&conn_handler.get_default_pages()),
                                   false);
        },
    }

    match handle_request_with_fallback(&request, &router,
                                       &conn_handler.get_default_header(),
                                       &conn_handler.get_default_pages()) {
        Ok(response) => {
            let ignore_body =
                if request.method.eq(&REST::OTHER(String::from("head"))) {
                    true
                } else {
                    false
                };

            return write_to_stream(stream, response, ignore_body);
        },
        Err(e) => {
            println!("Error on generating response -- {}", e);
            return write_to_stream(stream,
                                   build_default_response(&conn_handler.get_default_pages()),
                                   false);
        },
    }
}

fn write_to_stream(mut stream: TcpStream, response: Response, ignore_body: bool) -> Option<u8> {

    if let Ok(_) = stream.write(response.serialize(ignore_body).as_bytes()) {
        if let Ok(_) = stream.flush() {
            if response.to_close_connection() {
                if let Ok(_) = stream.shutdown(Shutdown::Both) {
                    return Some(0);
                } else {
                    return Some(1);
                }
            }

            return Some(0);
        }
    }

    None
}

fn read_request(mut stream: &TcpStream) -> Result<Request, ParseError> {
    let mut buffer = [0; 512];
    let result: Result<Request, ParseError>;

    if let Ok(_) = stream.read(&mut buffer) {
        let request = String::from_utf8_lossy(&buffer[..]);
        if request.is_empty() {
            return Err(ParseError::EmptyRequestErr);
        }

        result = match parse_request(&request) {
            Some(request_info) => Ok(request_info),
            None => Err(ParseError::EmptyRequestErr),
        };
    } else {
        result = Err(ParseError::ReadStreamErr);
    }

    result
}

fn parse_request(request: &str) -> Option<Request> {
    if request.is_empty() {
        return None;
    }

    //println!("{}", request);

    let mut method = REST::NONE;
    let mut uri = String::new();
    let mut scheme = HashMap::new();

    let mut cookie = HashMap::new();
    let mut header = HashMap::new();
    let mut body = Vec::new();
    let mut is_body = false;

    let (tx_base, rx_base) = mpsc::channel();
    let (tx_cookie, rx_cookie) = mpsc::channel();

    for (num, line) in request.trim().lines().enumerate() {
        if num == 0 {
            if line.is_empty() { continue; }

            let val = line.to_owned();
            let tx_clone = mpsc::Sender::clone(&tx_base);

            thread::spawn(move || {
                parse_request_base(val, tx_clone);
            });

        } else {
            if line.is_empty() {
                // meeting the empty line dividing header and body
                is_body = true;
                continue;
            }

            if !is_body {
                let val = line.to_owned();
                let header_info: Vec<&str> = val.splitn(2, ':').collect();

                if header_info.len() == 2 {
                    if header_info[0].trim().to_lowercase().eq("cookie") {
                        let cookie_body = header_info[1].to_owned();
                        let tx_clone = mpsc::Sender::clone(&tx_cookie);

                        thread::spawn(move || {
                            cookie_parser(cookie_body, tx_clone);
                        });

                    } else {
                        header.insert(
                            String::from(header_info[0].trim().to_lowercase()),
                            String::from(header_info[1].trim())
                        );
                    }
                }
            } else {
                body.push(line.to_owned());
                body.push(String::from("\r\n"));  //keep the line break
            }
        }
    }

    /* Since we don't move the tx but cloned them, need to drop them
     * specifically here, or we would hang forever before getting the
     * messages back.
     */
    drop(tx_base);
    drop(tx_cookie);

    if let Ok(base) = rx_base.recv_timeout(Duration::from_millis(200)) {
        method = base.method;
        uri = base.uri;
        scheme = base.scheme;
        header.entry(String::from("http_version")).or_insert(base.http_version);
    }

    if let Ok(cookie_set) = rx_cookie.recv_timeout(Duration::from_millis(200)) {
        cookie = cookie_set;
    }

    Some(Request::build_from(method, uri, scheme, cookie, header, body))
}

fn parse_request_base(line: String, tx: mpsc::Sender<RequestBase>) {
    let mut method = REST::NONE;
    let mut uri = String::new();
    let mut http_version = String::new();
    let mut scheme = HashMap::new();

    let request_info: Vec<&str> = line.split_whitespace().collect();
    for (num, info) in request_info.iter().enumerate() {
        match num {
            0 => {
                method = match &info[..] {
                    "GET" => REST::GET,
                    "PUT" => REST::PUT,
                    "POST" => REST::POST,
                    "DELETE" => REST::DELETE,
                    "OPTIONS" => REST::OPTIONS,
                    "" => REST::NONE,
                    _ => REST::OTHER(request_info[0].to_lowercase().to_owned()),
                };
            },
            1 => {
                let (req_uri, req_scheme) = split_path(info);
                uri.push_str(&req_uri[..]);

                if !req_scheme.is_empty() {
                    scheme_parser(&req_scheme[..], &mut scheme);
                }
            },
            2 => {
                http_version.push_str(*info);
            },
            _ => { /* Shouldn't happen, do nothing for now */ },
        };
    }

    match tx.send(RequestBase {
        method,
        uri,
        http_version,
        scheme,
    }) {
        _ => { drop(tx); }
    }
}

fn handle_request_with_fallback(
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
        REST::NONE => {
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
    let mut uri_parts: Vec<&str> = full_uri.trim().rsplitn(2, "/").collect();

    if let Some(pos) = uri_parts[0].find("?") {
        let (last_uri_pc, scheme) = uri_parts[0].split_at(pos);
        uri_parts[0] = last_uri_pc;

        let real_uri =
            if uri_parts[1].is_empty() {
                format!("/{}", uri_parts[0])
            } else {
                format!("{}/{}", uri_parts[1], uri_parts[0])
            };

        (real_uri, scheme.trim().to_owned())
    } else {
        (full_uri.trim().to_owned(), String::new())
    }
}

// Cookie parser will parse the request header's cookie field into a hash-map, where the
// field is the key of the map, which map to a single value of the key from the Cookie
// header field. Assuming no duplicate cookie keys, or the first cookie key-value pair
// will be stored.
fn cookie_parser(cookie_body: String, tx: mpsc::Sender<HashMap<String, String>>) { //cookie: &mut HashMap<String, String>) {
    if cookie_body.is_empty() { return; }

    let mut cookie = HashMap::new();
    let cookie_pairs: Vec<&str> = cookie_body.split(";").collect();
    let mut pair: Vec<&str>;

    for set in cookie_pairs.into_iter() {
        pair = set.trim().splitn(2, "=").collect();
        if pair.len() == 2 {
            cookie.entry(pair[0].trim().to_owned()).or_insert(pair[1].trim().to_owned());
        } else if pair.len() > 0 {
            cookie.entry(pair[0].trim().to_owned()).or_insert(String::new());
        }
    }

    match tx.send(cookie) {
        _ => { drop(tx); }
    }
}

fn scheme_parser(scheme: &str, scheme_collection: &mut HashMap<String, Vec<String>>) {
    let schemes: Vec<&str> = scheme.trim().split("&").collect();

    for kv_pair in schemes.into_iter() {
        let store: Vec<&str> = kv_pair.trim().splitn(2, "=").collect();
        if store.len() > 0 {
            let key = store[0].trim();
            let val =
                if store.len() == 2 {
                    store[1].trim().to_owned()
                } else {
                    String::new()
                };

            if scheme_collection.contains_key(key) {
                if let Some(val_vec) = scheme_collection.get_mut(key) {
                    val_vec.push(val);
                }
            } else {
                scheme_collection.insert(key.to_owned(), vec![val]);
            }
        }
    }
}

fn build_default_response(default_pages: &HashMap<u16, String>) -> Response {
    let mut resp = Response::new();

    resp.status(500);
    resp.check_and_update(&default_pages);

    resp
}
