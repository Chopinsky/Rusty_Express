#![allow(dead_code)]
#![allow(unused_mut)]
#![allow(unused_variables)]

use std::collections::HashMap;
use std::io::prelude::*;
use std::net::TcpStream;
use http::*;
use router::*;

pub fn handle_connection(stream: TcpStream, router: &Route) -> Option<u8> {
    let request: Request;
    match parse_request(&stream) {
        Ok(req) => {
            request = req;
        },
        Err(e) => {
            println!("Error on parsing request -- {}", e);
            return write_to_stream(stream, Response::get_status(400));
        },
    }

    match handle_request(request, &router) {
        Ok(response) => {
            return write_to_stream(stream, response.serialize());
        },
        Err(e) => {
            println!("Error on generating response -- {}", e);
            return write_to_stream(stream, Response::get_status(400));
        },
    }
}

fn write_to_stream(mut stream: TcpStream, response: String) -> Option<u8> {
    if let Ok(_) = stream.write(response.as_bytes()) {
        if let Ok(_) = stream.flush() {
            return Some(1);
        }
    }

    None
}

fn parse_request(mut stream: &TcpStream) -> Result<Request, String> {
    let mut buffer = [0; 512];
    let result: Result<Request, String>;

    if let Ok(_) = stream.read(&mut buffer) {
        let request = String::from_utf8_lossy(&buffer[..]);
        if request.is_empty() {
            return Err(String::from("Unable to parse the request: the incoming stream is blank..."));
        }

        result = match build_request_from_stream(&request) {
            Some(request_info) => Ok(request_info),
            None => Err(format!("Unable to parse the request from the stream...")),
        }
    } else {
        result = Err(format!("Unable to parse the request from the stream..."))
    }

    result
}

fn build_request_from_stream(request: &str) -> Option<Request> {
    if request.is_empty() {
        return None;
    }

    let mut method = REST::NONE;
    let mut path = String::new();
    let mut header = HashMap::new();

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
            let header_info: Vec<&str> = line.splitn(2, ':').collect();
            if header_info.len() == 2 {
                header.insert(
                    String::from(header_info[0]),
                    String::from(header_info[1])
                );
            }
        }
    }

    Some(Request::build_from(method, path, header))
}

fn handle_request(request_info: Request, router: &Route) -> Result<Response, String> {
    let mut resp = Response::new();

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
