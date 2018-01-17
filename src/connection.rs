#![allow(dead_code)]

use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::io::prelude::*;
use std::net::TcpStream;
use router::REST;

pub struct Request {
    pub method: REST,
    pub path: String,
    pub header_info: HashMap<String, String>,
}

pub struct Response {
    header: String,
    body: String,
}

pub fn handle_connection(mut stream: TcpStream) {
    let response: String;

    match get_request_from_stream(&stream) {
        Ok(request) => {
            match generate_response(&request) {
                Ok(result) => {
                    response = result;
                },
                Err(e) => {
                    println!("Error on generating response -- {}", e);
                    response = get_status(400);
                },
            };
        },
        Err(e) => {
            println!("Error on parsing request -- {}", e);
            response = get_status(400);
        },
    };

    stream.write(response.as_bytes()).unwrap();
    stream.flush().unwrap();
}

fn get_request_from_stream(mut stream: &TcpStream) -> Result<Request, String> {
    let mut buffer = [0; 512];

    match stream.read(&mut buffer) {
        Ok(_) => {
            let request = String::from_utf8_lossy(&buffer[..]);
            if request.is_empty() {
                return Err(String::from("Unable to parse the request: the incoming stream is blank..."));
            }

            Ok(parse_request(&request))
        },
        Err(e) => {
            Err(format!("Failed to read request from stream: {}", e))
        }
    }
}

fn parse_request(request: &str) -> Request {
    let mut rest = REST::NONE;
    let mut path = String::new();
    let mut header = HashMap::new();

    if request.is_empty() {
        return Request {
            method: rest,
            path,
            header_info: header,
        };
    }

    let lines = request.trim().lines();

    for (num, line) in lines.enumerate() {
        if num == 0 {
            let request_info: Vec<&str> = line.split_whitespace().collect();
            for (num, info) in request_info.iter().enumerate() {
                match num {
                    0 => {
                        rest = match &info[..] {
                            "GET" => REST::GET,
                            "PUT" => REST::PUT,
                            "POST" => REST::POST,
                            "DELETE" => REST::DELETE,
                            _ => REST::NONE,
                        };

                    },
                    1 => { path.push_str(info); },
                    _ => { /* Do nothing for now */ },
                };
            }
        } else {
            let header_info: Vec<&str> = line.splitn(2, ':').collect();
            if header_info.len() == 2 {
                header.insert(String::from(header_info[0]), String::from(header_info[1]));
            }
        }
    }

    return Request {
        method: rest,
        path,
        header_info: header,
    };
}

fn generate_response(request_info: &Request) -> Result<String, String> {
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
        match File::open(&path) {
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