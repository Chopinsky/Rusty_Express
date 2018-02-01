use std::collections::HashMap;
use std::io::prelude::*;
use std::net::{TcpStream, Shutdown};
use http::*;
use router::*;

#[derive(PartialEq, Eq, Clone, Copy)]
enum ParseError {
    EmptyRequestErr,
    ReadStreamErr,
}

pub fn handle_connection(
        stream: TcpStream,
        router: &Route,
        header: &HashMap<String, String>,
        default_pages: &HashMap<u16, String>
    ) -> Option<u8> {

    let request: Request;
    match parse_request(&stream) {
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
            return write_to_stream(stream, build_default_response(&default_pages));
        },
    }

    match handle_request_with_fallback(request, &router, &header, &default_pages) {
        Ok(response) => {
            return write_to_stream(stream, response);
        },
        Err(e) => {
            println!("Error on generating response -- {}", e);
            return write_to_stream(stream, build_default_response(&default_pages));
        },
    }
}

fn write_to_stream(mut stream: TcpStream, response: Response) -> Option<u8> {

    if let Ok(_) = stream.write(response.serialize().as_bytes()) {
        if let Ok(_) = stream.flush() {
            return Some(0);
        }
    }

    None
}

fn parse_request(mut stream: &TcpStream) -> Result<Request, ParseError> {
    let mut buffer = [0; 512];
    let result: Result<Request, ParseError>;

    if let Ok(_) = stream.read(&mut buffer) {
        let request = String::from_utf8_lossy(&buffer[..]);
        if request.is_empty() {
            return Err(ParseError::EmptyRequestErr);
        }

        result = match build_request_from_stream(&request) {
            Some(request_info) => Ok(request_info),
            None => Err(ParseError::EmptyRequestErr),
        };
    } else {
        result = Err(ParseError::ReadStreamErr);
    }

    result
}

fn build_request_from_stream(request: &str) -> Option<Request> {
    if request.is_empty() {
        return None;
    }

    let mut method = REST::NONE;
    let mut path = String::new();
    let mut scheme = HashMap::new();
    let mut cookie = HashMap::new();
    let mut header = HashMap::new();
    let mut body = Vec::new();
    let mut is_body = false;

    for (num, line) in request.trim().lines().enumerate() {
        if num == 0 {
            let request_info: Vec<&str> = line.split_whitespace().collect();
            for (num, info) in request_info.iter().enumerate() {
                match num {
                    0 => {
                        method = match &info[..] {
                            "GET" => REST::GET,
                            "PUT" => REST::PUT,
                            "POST" => REST::POST,
                            "DELETE" => REST::DELETE,
                            "" => REST::NONE,
                            _ => REST::OTHER(request_info[0].to_owned()),
                        };
                    },
                    1 => {
                        let (req_path, req_scheme) = split_path(info);
                        path.push_str(&req_path[..]);

                        if !req_scheme.is_empty() {
                            scheme_parser(&req_scheme[..], &mut scheme);
                        }
                    },
                    2 => {
                        header.insert(
                            String::from("HttpProtocol"),
                            info.to_string()
                        );
                    },
                    _ => { /* Shouldn't happen, do nothing for now */ },
                };
            }
        } else {
            if line.is_empty() {
                // meeting the empty line dividing header and body
                is_body = true;
                continue;
            }

            if !is_body {
                let header_info: Vec<&str> = line.splitn(2, ':').collect();
                if header_info.len() == 2 {
                    if header_info[0].trim().to_lowercase().eq("cookie") {
                        cookie_parser(header_info[1], &mut cookie);
                    } else {
                        header.insert(
                            String::from(header_info[0].trim().to_lowercase()),
                            String::from(header_info[1].trim())
                        );
                    }
                }
            } else {
                body.push(line.to_owned());
            }
        }
    }

    Some(Request::build_from(method, path, scheme, cookie, header, body))
}

fn handle_request_with_fallback(
        request_info: Request,
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
        REST::GET => {
            router.handle_get(request_info, &mut resp);
        },
        REST::PUT => {
            router.handle_put(request_info, &mut resp);
        },
        REST::POST => {
            router.handle_post(request_info, &mut resp);
        },
        REST::DELETE => {
            router.handle_delete(request_info, &mut resp);
        },
        REST::OTHER(_) => {
            router.handle_other(request_info, &mut resp);
        },
        _ => {
            return Err(String::from("Invalid request method"));
        },
    }

    resp.check_and_update(&fallback);
    Ok(resp)
}

fn split_path(path: &str) -> (String, String) {
    let mut path_parts: Vec<&str> = path.rsplitn(2, "/").collect();

    if path_parts[0].starts_with("?") {
        let scheme: &str = &path_parts.swap_remove(0)[1..];
        let mut act_path = String::new();

        for part in path_parts.into_iter() {
            if part.is_empty() { continue; }
            act_path = format!("/{}{}", part.trim(), act_path);
        }

        (act_path, scheme.trim().to_owned())
    } else {
        (path.trim().to_owned(), String::new())
    }
}

// Cookie parser will parse the request header's cookie field into a hash-map, where the
// field is the key of the map, which map to a single value of the key from the Cookie
// header field. Assuming no duplicate cookie keys, or the first cookie key-value pair
// will be stored.
fn cookie_parser(request_info: &str, cookie: &mut HashMap<String, String>) {
    if request_info.is_empty() { return; }

    let cookie_set: Vec<&str> = request_info.split(";").collect();
    let mut pair: Vec<&str>;

    for set in cookie_set.into_iter() {
        pair = set.trim().splitn(2, "=").collect();
        if pair.len() == 2 {
            cookie.entry(pair[0].trim().to_owned()).or_insert(pair[1].trim().to_owned());
        } else if pair.len() > 0 {
            cookie.entry(pair[0].trim().to_owned()).or_insert(String::new());
        }
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
