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

pub fn handle_connection(stream: TcpStream, router: &Route, header: &HashMap<String, String>) -> Option<u8> {
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
            return write_to_stream(stream, None);
        },
    }

    match handle_request(request, &router, &header) {
        Ok(response) => {
            return write_to_stream(stream, Some(response));
        },
        Err(e) => {
            println!("Error on generating response -- {}", e);
            return write_to_stream(stream, None);
        },
    }
}

fn write_to_stream(mut stream: TcpStream, response: Option<Response>) -> Option<u8> {
    match response {
        Some(resp) => {
            if let Ok(_) = stream.write(resp.serialize().as_bytes()) {
                if let Ok(_) = stream.flush() {
                    return Some(0);
                }
            }
        },
        None => {
            let mut resp = Response::new();
            resp.status(500);

            if let Ok(_) = stream.write(resp.serialize().as_bytes()) {
                if let Ok(_) = stream.flush() {
                    return Some(1);
                }
            }
        },
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
                    1 => { path.push_str(info); },
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
                    if header_info[0].to_lowercase().eq("cookie") {
                        cookie_parser(header_info[1], &mut cookie);
                    } else {
                        header.insert(
                            String::from(header_info[0].to_lowercase()),
                            String::from(header_info[1])
                        );
                    }
                }
            } else {
                body.push(line.to_owned());
            }
        }
    }

    Some(Request::build_from(method, path, cookie, header, body))
}

fn handle_request(request_info: Request, router: &Route, header: &HashMap<String, String>) -> Result<Response, String> {
    let mut resp =
        if header.is_empty() {
            Response::new()
        } else {
            Response::new_with_default_header(&header)
        };

    match request_info.method {
        REST::GET => {
            router.handle_get(request_info, &mut resp);
            Ok(resp)
        },
        REST::PUT => {
            router.handle_put(request_info, &mut resp);
            Ok(resp)
        },
        REST::POST => {
            router.handle_post(request_info, &mut resp);
            Ok(resp)
        },
        REST::DELETE => {
            router.handle_delete(request_info, &mut resp);
            Ok(resp)
        },
        REST::OTHER(_) => {
            router.handle_other(request_info, &mut resp);
            Ok(resp)
        },
        _ => {
            Err(String::from("Invalid request method"))
        },
    }
}

fn cookie_parser(request_info: &str, cookie: &mut HashMap<String, String>) {
    if request_info.is_empty() { return; }

    let cookie_set: Vec<&str> = request_info.split(";").collect();
    let mut pair: Vec<&str>;

    for set in cookie_set {
        pair = set.trim().splitn(2, "=").collect();
        if pair.len() == 2 {
            cookie.entry(pair[0].to_owned()).or_insert(pair[1].to_owned());
        } else if pair.len() > 0 {
            cookie.entry(pair[0].to_owned()).or_insert(String::new());
        }
    }
}
