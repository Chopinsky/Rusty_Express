#![allow(dead_code)]

use std::collections::HashMap;
use std::fs::File;
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
    let request_info: Request;
    match get_request_from_stream(&stream) {
        Ok(result) => {
            request_info = result;
        },
        Err(e) => {
            println!("Error on parsing request -- {}", e);
            return;
        },
    };

    let response: String;
    match generate_response(&request_info) {
        Ok(result) => {
            response = result;
        },
        Err(e) => {
            println!("Error on generating response -- {}", e);
            return;
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

fn get_response_content(path: &str) -> String {

    let (status_line, path, content_type) =
        match &path[..] {
            "/" => (
                get_status(200),
                get_source_path("index.html"),
                String::from("Content-Type: text/html; charset=UTF-8\r\n"),
            ),
            "/styles.css" => (
                get_status(200),
                get_source_path("styles.css"),
                String::from("Content-Type: text/css;\r\n"),
            ),
            "/bundle.js" => (
                get_status(200),
                get_source_path("bundle.js"),
                String::from("Content-Type: application/javascript;\r\n"),
            ),
            "/favicon.ico" => (
                get_status(200),
                get_source_path(""),
                String::from("Content-Type: image/x-icon;\r\n"),
            ),
            _ => (
                get_status(404),
                get_source_path("404.html"),
                String::from("Content-Type: text/html; charset=UTF-8\r\n"),
            ),
        };

    let mut response = String::new();
    response.push_str(&status_line);

    if !path.is_empty() {
        let mut file = File::open(&path).unwrap();
        let mut contents = String::new();

        file.read_to_string(&mut contents).unwrap();

        if contents.len() > 0 {
            response.push_str(&content_type);
            response.push_str(&contents);
        } else {
            println!("Can't load contents!");
        }
    }

    return response;
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
            _ => "404 NOT FOUND"
        };

    return format!("HTTP/1.1 {}\r\n\r\n", status_base);
}