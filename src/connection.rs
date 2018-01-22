#![allow(dead_code)]

use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::io::prelude::*;
use std::net::TcpStream;
use std::path::Path;
use http::*;
use router::*;

/* no need to instanciate this
pub struct ConnectionHandler {
    router: Route,
}

impl ConnectionHandler {
    pub fn new() -> Self {
        ConnectionHandler {
            router: Route::new(),
        }
    }

    pub fn use_router(&mut self, router: Route) {
        self.router = router;
    }
}
*/

pub fn handle_connection(stream: TcpStream, router: Route) -> Option<u8> {
    let request: Request;
    match request_from_stream(&stream) {
        Ok(req) => {
            request = req;
        },
        Err(e) => {
            println!("Error on parsing request -- {}", e);
            return write_to_stream(stream, get_status(400));
        },
    }

    match make_response(&request, &router) {
        Ok(response) => {
            return write_to_stream(stream, response);
        },
        Err(e) => {
            println!("Error on generating response -- {}", e);
            return write_to_stream(stream, get_status(400));
        },
    }
}

fn write_to_stream(mut stream: TcpStream, response: String) -> Option<u8> {
    match stream.write(response.as_bytes()) {
        Ok(_) => {
            match stream.flush() {
                Ok(_) => return Some(1),
                Err(e) => {
                    println!("Failed to flush the stream: {}", e);
                    None
                },
            }
        },
        Err(e) => {
            println!("Failed to write to the stream: {}", e);
            None
        },
    }
}

fn request_from_stream(mut stream: &TcpStream) -> Result<Request, String> {
    let mut buffer = [0; 512];

    match stream.read(&mut buffer) {
        Ok(_) => {
            let request = String::from_utf8_lossy(&buffer[..]);
            if request.is_empty() {
                return Err(String::from("Unable to parse the request: the incoming stream is blank..."));
            }

            match parse_request(&request) {
                Some(request_info) => Ok(request_info),
                None => Err(format!("Unable to parse the request from the stream...")),
            }
        },
        Err(e) => {
            Err(format!("Failed to read request from stream: {}", e))
        }
    }
}

fn parse_request(request: &str) -> Option<Request> {
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
                            _ => REST::NONE,
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

fn make_response(request_info: &Request, router: &Route) -> Result<String, String> {
    match request_info.method {
        REST::GET => {

            Ok(get_response_content(&request_info.path[..]))
        },
        REST::NONE => {
            Err(String::from("Invalid request method"))
        },
        _ => {
            /* Don't do anything special here */
            Ok(get_response_content(""))
        }
    }
}

fn get_response_content(request_path: &str) -> String {

    let (status_line, path) =
        match &request_path[..] {
            "/" => (
                get_status(200),
                get_source_path("index.html"),
            ),
            "/styles.css" => (
                get_status(200),
                get_source_path("styles.css"),
            ),
            "/bundle.js" => (
                get_status(200),
                get_source_path("bundle.js"),
            ),
            "/favicon.ico" => (
                get_status(200),
                get_source_path(""),
            ),
            _ => (
                get_status(404),
                get_source_path("404.html"),
            ),
        };

    let mut response = String::new();

    if !path.is_empty() {
        let file_path = Path::new(&path);
        if !file_path.is_file() {
            // if doesn't exist or not a file, fail now
            println!("Can't locate requested file");
            response = get_status(404);
        } else {
            // try open the file
            match File::open(file_path) {
                Ok(file) => {
                    let mut buf_reader = BufReader::new(file);
                    let mut contents: String = String::new();

                    match buf_reader.read_to_string(&mut contents) {
                        Err(e) => {
                            println!("Unable to read file: {} (requested path: {})", e, path);
                            response = get_status(500);
                        },
                        Ok(_) if contents.len() > 0 => {
                            //things are truly ok now
                            response.push_str(&status_line);
                            response.push_str(&contents);
                        },
                        _ => {
                            println!("File stream finds nothing...");
                            response = get_status(404);
                        }
                    };
                },
                Err(e) => {
                    println!("Unable to open requested file: {} (requested path: {})", e, path);
                    response = get_status(404);
                },
            };
        }
    } else {
        println!("Can't locate requested file");
        response = get_status(404);
    }

    response
}

fn get_source_path(source: &str) -> String {

    let mut path = String::new();

    if !source.is_empty() {
        path.push_str("../client/public/");
        path.push_str(&source);
    }

    return path;
}

fn get_status(status: u16) -> String {
    let status_base =
        match status {
            200 => "200 OK",
            500 => "500 INTERNAL SERVER ERROR",
            400 => "400 BAD REQUEST",
            404 | _ => "404 NOT FOUND",
        };

    return format!("HTTP/1.1 {}\r\n\r\n", status_base);
}
