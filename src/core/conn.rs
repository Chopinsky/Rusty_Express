#![allow(clippy::borrowed_box)]
#![allow(dead_code)]

use std::io::{prelude::*, BufWriter, ErrorKind};
use std::net::{Shutdown, SocketAddr};
use std::str;
use std::sync::Arc;
use std::time::Duration;

use crate::core::config::ConnMetadata;
use crate::core::http::{
    Request, RequestWriter, Response, ResponseManager, ResponseStates, ResponseWriter,
};
use crate::core::router::{Route, RouteSeeker, REST, RouteHandler};
use crate::core::stream::Stream;
use crate::support::{
    common::flush_buffer, common::write_to_buff, common::MapUpdates, debug, debug::InfoLevel,
    shared_pool, TaskType,
};

use crate::channel::{self, Sender, Receiver};
use crate::hashbrown::HashMap;

static HEADER_END: [u8; 2] = [13, 10];
static FLUSH_RETRY: u8 = 4;

type ExecCode = u8;
type BaseLine = Option<Receiver<(RouteHandler, HashMap<String, String>)>>;

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
enum StreamException {
    HeartBeat,
    EmptyRequest,
    ReadStreamFailure,
    AccessDenied,
    ServiceUnavailable,
}

pub(crate) trait StreamHandler {
    fn process(self, is_tls: bool);
}

impl StreamHandler for Stream {
    fn process(mut self, is_tls: bool) {
        // split the stream such that we can read while writing latest responses
        let mut reader_stream =
            match self.try_clone() {
                Ok(stream) => {
                    // get the stream clone for async-reader
                    stream
                },
                Err(_) => {
                    // failed to clone(?) and now try the old-fashion way to serve
                    async_handler::handle_connection(self);
                    return;
                },
            };

        // pipeline-1: keep listening to the reader stream
        let (sender, receiver) = channel::bounded(8);
        shared_pool::run(move || reader_stream.recv_request(sender), TaskType::StreamLoader);

        // pipeline-2: once receiving a request, parse and serve, then send the response back to be written back
        let (resp_tx, resp_rx) = channel::bounded(8);
        let addr = self.peer_addr();
        shared_pool::run(move || handle_requests(receiver, resp_tx, addr.ok(), is_tls), TaskType::Parser);

        // pipeline-end: receive the response, write them back
        while let Ok((id, resp)) = resp_rx.recv_timeout(Duration::from_secs(16)) {
            //TODO: check if the responses are in order, otherwise, push into a queue until its turn
            //      if id == 0, meaning an internal error happened, return immediately
            self.write_back(resp);
        }

        // shut down the stream after we're done
        if let Err(err) = self.shutdown(Shutdown::Both) {
            debug::print(
                &format!(
                    "Encountered errors while shutting down the trunked body stream: {}",
                    err
                ),
                InfoLevel::Warning,
            );
        }
    }
}

trait PipelineWorker {
    fn recv_request(&mut self, chan: Sender<Result<String, StreamException>>);
    fn write_back(&mut self, response: Box<Response>);
}

impl PipelineWorker for Stream {
    fn recv_request(&mut self, chan: Sender<Result<String, StreamException>>) {
        let mut buffer = [0u8; 512];
        let mut raw_req = String::with_capacity(512);

        loop {
            // read will block until there're data to read; if not, then we're good to quit
            match self.read(&mut buffer) {
                Ok(len) => {
                    // if no more request data left to read
                    if len == 0 {
                        if !raw_req.is_empty() {
                            // if we have no more incoming stream, sending it to parser and wrap up
                            chan.send(Ok(raw_req.clone())).unwrap_or_default();
                        } else {
                            // send a heart-beat
                            chan.send(Err(StreamException::HeartBeat)).unwrap_or_default();
                        };

                        // reader shall close because keep-alive header is not `keep-alive` or `close`
                        break;
                    }

                    // if there are request data to read, convert bytes to string
                    if let Ok(content) = str::from_utf8(&buffer[..len]) {
                        if len < 512 {
                            // trim end if we're at the end of the request stream
                            raw_req.push_str(content);

                            // done with this request, sending it to parser; if the channel is closed,
                            // meaning the stream is closed, we quit as well.
                            if chan.send(Ok(raw_req.clone())).is_err() {
                                break;
                            }

                            // reset the buffer-states
                            raw_req.clear();
                        } else {
                            // if there are more to read, don't trim and continue reading. Don't trim
                            // since the end of the `content` maybe the line breaker between head and
                            // body and we don't want to lose that info
                            raw_req.push_str(content);
                        }
                    } else {
                        // failed at string conversion, quit reader stream
                        debug::print("Failed to parse the request stream", InfoLevel::Warning);
                        chan.send(Err(StreamException::ReadStreamFailure)).unwrap_or_default();
                        break;
                    }
                },
                Err(e) => {
                    // handle read errors. If timeout, meaning we've waited long enough for more requests
                    // but none are received, close the stream now.
                    if e.kind() != ErrorKind::TimedOut {
                        debug::print(
                            &format!("Reading stream disconnected -- {}", e),
                            InfoLevel::Warning,
                        );

                        chan.send(Err(StreamException::ReadStreamFailure)).unwrap_or_default();
                    }

                    break;
                },
            };
        }

        // shutdown the read stream regardless of the reason
        self.shutdown(Shutdown::Read).unwrap_or_default();
    }

    fn write_back(&mut self, response: Box<Response>) {
        let mut writer = BufWriter::new(self);

        // Serialize the header to the stream
        response.write_header(&mut writer);

        // Blank line to indicate the end of the response header
        write_to_buff(&mut writer, &HEADER_END);

        // If header only, we're done
        if response.is_header_only() {
            flush_buffer(&mut writer);
            return;
        }

        // write the body to the stream
        response.write_body(&mut writer);

        // flush the buffer and shutdown the connection: we're done; no need for explicit shutdown
        // the stream as it's dropped automatically on out-of-the-scope.
        let mut retry: u8 = 0;
        while retry < FLUSH_RETRY {
            retry += 1;
            if flush_buffer(&mut writer) == 0 {
                break;
            }
        }
    }
}

fn handle_requests(
    inbox: Receiver<Result<String, StreamException>>,
    outbox: Sender<(usize, Box<Response>)>,
    peer_addr: Option<SocketAddr>,
    is_tls: bool,
)
{
    let mut req_id = 1;
    let outbox = Arc::new(outbox);

    for req in inbox {
        match req {
            Ok(source) => {
                //TODO: not so simple... need to parse the request to know if it could contain a body
                //      using `Content-Length` or `boundary(? future...)` header to determine

                //TODO: plan -> call split(`\r\n\r\n`) on raw, then parse, and determine if it contains
                //      a body, if so, split off at kth-bytes using split_off(k) on the trunk, attach the
                //      first half as the body of the last request, send it, and parse the remainder.
                //      Otherwise, just parse the remainder and set the flags (if no more body to
                //      append, just send); skip empty chunk.

                let outbox_clone = Arc::clone(&outbox);
                shared_pool::run(move || {
                    serve_connection(source, req_id, outbox_clone, peer_addr, is_tls);
                }, TaskType::Request);

                // TODO: incremental of id at every new request instead
                req_id += 1;
            },
            Err(err) => {
                if err != StreamException::HeartBeat {
                    // if only a read stream heart-beat, meaning we're still waiting for new requests
                    // to come, just continue with the listener.
                    outbox.send(
                        (0, Box::new(build_err_response(map_err_code(err))))
                    ).unwrap_or_default();
                }

                return;
            },
        }
    }
}

fn serve_connection(
    source: String, id: usize, outbox: Arc<Sender<(usize, Box<Response>)>>, peer_addr: Option<SocketAddr>, is_tls: bool
)
{
    //TODO: now the parser portion will be moved to the caller. Only the executor portion will be
    //      moved into this spawned thread.

    ///////////////////////////
    // >>> Build Request <<< /
    //////////////////////////

    // prepare the request source string to be parsed
    let trimmed = source.trim_end_matches(|c| c == '\r' || c == '\n' || c == '\u{0}');
    if trimmed.is_empty() {
        outbox.send((
            id, Box::new(build_err_response(map_err_code(StreamException::EmptyRequest))))
        ).unwrap_or_default();
        return;
    }

    // now parse the request and find the proper request handler
    let mut request = Box::new(Request::new());
    let mut callback = parse_request_sync(trimmed, &mut request);

    // not matching any given router, return null
    if callback.is_none() {
        outbox.send(
            (id, Box::new(build_err_response(map_err_code(StreamException::ServiceUnavailable))))
        ).unwrap_or_default();
        return;
    }

    // setup peer address
    if let Some(client) = peer_addr {
        request.set_client(client);
    }

    // check server authorization on certain path
    if let Some(auth) = Route::get_auth_func() {
        if !auth(&request, &request.uri) {
            outbox.send(
                (id, Box::new(build_err_response(map_err_code(StreamException::AccessDenied))))
            ).unwrap_or_default();
            return;
        }
    }

    ////////////////////////////
    // >>> Build Response <<< /
    ///////////////////////////

    // generating the reponse and setup stuff
    let mut response = initialize_response(is_tls);
    match request.header("connection") {
        Some(ref val) if val.eq(&String::from("close")) => response.can_keep_alive(false),
        _ => response.can_keep_alive(true),
    };

    if request.method.eq(&REST::OTHER(String::from("HEAD"))) {
        response.header_only(true);
    }

    // callback function will decide what to be written into the response
    callback.execute(&request, &mut response);

    // update the response based on critical conditions
    response.redirect_handling();
    response.validate_and_update();

    // done, send response back
    outbox.send((id, response)).unwrap_or_default();
}

fn parse_request_sync(source: &str, request: &mut Box<Request>) -> RouteHandler {
    if source.is_empty() {
        return RouteHandler::default();
    }

    let mut handler = RouteHandler::default();
    for (index, info) in source.trim().splitn(2, "\r\n").enumerate() {
        match index {
            0 => {
                let res = parse_start_line_sync(&info, request);
                if res.0.is_some() {
                    request.create_param(res.1);
                }

                handler = res.0;
            },
            1 => parse_remainder_sync(info, request),
            _ => break,
        }
    }

    handler
}

fn parse_start_line_sync(source: &str, req: &mut Box<Request>) -> (RouteHandler, HashMap<String, String>) {
    let mut raw_scheme = String::new();
    let mut raw_fragment = String::new();

    for (index, info) in source.split_whitespace().enumerate() {
        if index < 2 && info.is_empty() {
            return (RouteHandler::default(), HashMap::new());
        }

        match index {
            0 => {
                let base_method = match &info.to_uppercase()[..] {
                    "GET" => REST::GET,
                    "PUT" => REST::PUT,
                    "POST" => REST::POST,
                    "DELETE" => REST::DELETE,
                    "OPTIONS" => REST::OPTIONS,
                    _ => REST::OTHER(info.to_uppercase())
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
        let res = Route::seek_sync(&req.method, &req.uri);

        // now do more work on non-essential parsing
        if !raw_fragment.is_empty() {
            req.set_fragment(raw_fragment);
        }

        if !raw_scheme.is_empty() {
            req.create_scheme(parse_scheme(raw_scheme));
        }

        return res;
    }

    (RouteHandler::default(), HashMap::new())
}

fn parse_remainder_sync(info: &str, req: &mut Box<Request>) {
    let remainder: String = info.to_owned();
    if remainder.is_empty() {
        return;
    }

    let mut header: HashMap<String, String> = HashMap::new();
    let mut cookie: HashMap<String, String> = HashMap::new();
    let mut body: Vec<String> = Vec::with_capacity(64);
    let mut is_body = false;

    for line in remainder.lines() {
        if line.is_empty() && !is_body {
            // meeting the empty line dividing header and body
            is_body = true;
            continue;
        }

        parse_headers(
            line,
            is_body,
            &mut header,
            &mut cookie,
            &mut body,
        );
    }

    req.set_headers(header);
    req.set_cookies(cookie);
    req.set_bodies(body);
}

fn initialize_response(is_tls: bool) -> Box<Response> {
    let header = ConnMetadata::get_default_header();
    let mut resp = match header {
        None => Box::new(Response::new()),
        Some(h) => Box::new(Response::new_with_default_header(h)),
    };

    if is_tls {
        resp.can_keep_alive(false);
    }

    resp
}

fn parse_headers(
    line: &str,
    is_body: bool,
    header: &mut HashMap<String, String>,
    cookie: &mut HashMap<String, String>,
    body: &mut Vec<String>,
)
{
    if !is_body {
        let mut header_key: &str = "";
        let mut is_cookie = false;

        for (idx, info) in line.trim().splitn(2, ':').enumerate() {
            match idx {
                0 => {
                    header_key = &info.trim()[..];
                    is_cookie = header_key.eq("cookie");
                }
                1 => {
                    if is_cookie {
                        parse_cookie(info.trim(), cookie);
                    } else if !header_key.is_empty() {
                        header.add(header_key, info.trim().to_owned(), true);
                    }
                }
                _ => break,
            }
        }
    } else {
        body.push(line.to_owned());
    }
}

fn split_path(source: &str, path: &mut String, scheme: &mut String, frag: &mut String) {
    let uri = source.trim().trim_end_matches('/');
    if uri.is_empty() {
        path.push('/');
        return;
    }

    let mut uri_parts: Vec<&str> = uri.rsplitn(2, '/').collect();

    // parse fragment out
    if let Some(pos) = uri_parts[0].find('#') {
        let (remains, raw_frag) = uri_parts[0].split_at(pos);
        uri_parts[0] = remains;

        if !raw_frag.is_empty() {
            frag.push_str(raw_frag);
        }
    }

    // parse scheme out
    if let Some(pos) = uri_parts[0].find('?') {
        let (remains, raw_scheme) = uri_parts[0].split_at(pos);
        uri_parts[0] = remains;

        if uri_parts[1].is_empty() {
            path.push('/');
            path.push_str(uri_parts[0]);
        } else {
            path.push_str(uri_parts[1]);
            path.push('/');
            path.push_str(uri_parts[0]);
        };

        scheme.push_str(raw_scheme.trim());
    } else {
        let uri_len = uri.len();
        if uri_len > 1 && uri.ends_with('/') {
            path.push_str(&uri[..uri_len - 1]);
        } else {
            path.push_str(uri)
        };
    }
}

/// Cookie parser will parse the request header's cookie field into a hash-map, where the
/// field is the key of the map, which map to a single value of the key from the Cookie
/// header field. Assuming no duplicate cookie keys, or the first cookie key-value pair
/// will be stored.
fn parse_cookie(raw: &str, cookie: &mut HashMap<String, String>) {
    if raw.is_empty() {
        return;
    }

    for set in raw.trim().split(';') {
        let pair: Vec<&str> = set.trim().splitn(2, '=').collect();
        if pair.len() == 2 {
            cookie.add(pair[0].trim(), pair[1].trim().to_owned(), false);
        } else if !pair.is_empty() {
            cookie.add(pair[0].trim(), String::new(), false);
        }
    }
}

fn parse_scheme(scheme: String) -> HashMap<String, Vec<String>> {
    let mut scheme_result: HashMap<String, Vec<String>> = HashMap::new();
    for (_, kv_pair) in scheme.trim().split('&').enumerate() {
        let store: Vec<&str> = kv_pair.trim().splitn(2, '=').collect();

        if !store.is_empty() {
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

fn build_err_response(err_status: u16) -> Response {
    let mut resp = Response::new();

    resp.status(err_status);
    if err_status == 0 {
        return resp;
    }

    resp.validate_and_update();
    resp.keep_alive(false);

    if resp.get_content_type().is_empty() {
        resp.set_content_type("text/html");
    }

    resp
}

fn map_err_code(err: StreamException) -> u16 {
    match err {
        StreamException::EmptyRequest => 400,
        StreamException::AccessDenied => 401,
        StreamException::ServiceUnavailable => 404,
        StreamException::ReadStreamFailure | StreamException::HeartBeat => 0,
    }
}

#[inline]
pub(crate) fn send_err_resp(mut stream: Stream, err_code: u16) {
    stream.write_back(Box::new(build_err_response(err_code)));
}

mod async_handler {
    use std::io::{prelude::*, BufWriter};
    use std::net::Shutdown;
    use std::str;
    use std::time::Duration;
    use super::*;

    use crate::core::http::{
        Request, RequestWriter, Response, ResponseManager, ResponseStates, ResponseWriter,
    };
    use crate::core::router::{Route, RouteSeeker, REST, RouteHandler};
    use crate::core::stream::Stream;
    use crate::support::{
        common::flush_buffer, common::write_to_buff, debug, debug::InfoLevel,
        shared_pool, TaskType,
    };

    use crate::channel;
    use crate::hashbrown::HashMap;

    pub(crate) fn handle_connection(mut stream: Stream) -> ExecCode {
        let (callback, request) = match recv_request(&mut stream) {
            Err(err) => {
                let status = map_err_code(err);
                if status == 0 {
                    // connection is sour, shutdown now
                    if let Err(err) = stream.shutdown(Shutdown::Both) {
                        return 1;
                    }

                    return 0;
                }

                debug::print(
                    &format!("Error on parsing request: {}", status),
                    InfoLevel::Error,
                );

                return write_to_stream(stream, Box::new(build_err_response(status)));
            },
            Ok(cb) => cb,
        };

        let is_tls = stream.is_tls();
        send_response(stream, request, callback, is_tls)
    }

    fn send_response(
        stream: Stream,
        request: Box<Request>,
        mut callback: RouteHandler,
        is_tls: bool,
    ) -> ExecCode
    {
        let mut response = initialize_response(is_tls);
        match request.header("connection") {
            Some(ref val) if val.eq(&String::from("close")) => response.can_keep_alive(false),
            _ => response.can_keep_alive(true),
        };

        if request.method.eq(&REST::OTHER(String::from("HEAD"))) {
            response.header_only(true);
        }

        // callback function will decide what to be written into the response
        callback.execute(&request, &mut response);

        response.redirect_handling();
        response.validate_and_update();

        write_to_stream(stream, response)
    }

    fn write_to_stream(mut stream: Stream, mut response: Box<Response>) -> ExecCode {
        let s_clone = if response.to_keep_alive() {
            match  stream.try_clone() {
                Ok(s) => Some(s),
                _ => None,
            }
        } else {
            None
        };

        let mut writer = BufWriter::new(&mut stream);

        // Serialize the header to the stream
        response.write_header(&mut writer);

        // Blank line to indicate the end of the response header
        write_to_buff(&mut writer, &HEADER_END);

        // If header only, we're done
        if response.is_header_only() {
            return flush_buffer(&mut writer);
        }

        if let Some(s) = s_clone {
            // serialize_trunked_body will block until all the keep-alive i/o are done
            response.keep_long_conn(s, &mut writer);
        } else {
            // else, write the body to the stream
            response.write_body(&mut writer);

            // flush the buffer and shutdown the connection: we're done; no need for explicit shutdown
            // the stream as it's dropped automatically on out-of-the-scope.
            let mut retry: u8 = 0;
            while retry < FLUSH_RETRY {
                retry += 1;
                if flush_buffer(&mut writer) == 0 {
                    break;
                }
            }
        }

        // regardless of buffer being flushed, close the stream now.
        stream_shutdown(writer.get_mut())
    }

    fn recv_request(stream: &mut Stream) -> Result<(RouteHandler, Box<Request>), StreamException> {
        let raw = read_content(stream)?;
        let trimmed = raw.trim_matches(|c| c == '\r' || c == '\n' || c == '\u{0}');

        if trimmed.is_empty() {
            return Err(StreamException::EmptyRequest);
        }

        let mut request = Box::new(Request::new());
        let result = parse_request(trimmed, &mut request);

        if result.is_none() {
            return Err(StreamException::ServiceUnavailable)
        }

        if let Ok(client) = stream.peer_addr() {
            request.set_client(client);
        }

        if let Some(auth) = Route::get_auth_func() {
            if !auth(&request, &request.uri) {
                return Err(StreamException::AccessDenied);
            }
        }

        Ok((result, request))
    }

    fn read_content(stream: &mut Stream) -> Result<String, StreamException> {
        let mut buffer = [0u8; 512];
        let mut raw_req = String::with_capacity(512);

        loop {
            match stream.read(&mut buffer) {
                Ok(len) => {
                    if len == 0 && raw_req.is_empty() {
                        // if the request is a mere handshake with no request data, we return
                        return Err(StreamException::HeartBeat);
                    }

                    if let Ok(req_slice) = str::from_utf8(&buffer[..len]) {
                        if len < 512 {
                            // trim end if we're at the end of the request stream
                            raw_req.push_str(
                                req_slice.trim_end_matches(|c| c == '\r' || c == '\n' || c == '\u{0}')
                            );

                            return Ok(raw_req);
                        } else {
                            // if there are more to read, don't trim and continue
                            raw_req.push_str(req_slice);
                        }
                    } else {
                        debug::print("Failed to parse the request stream", InfoLevel::Warning);
                        return Err(StreamException::ReadStreamFailure);
                    }
                },
                Err(e) => {
                    debug::print(
                        &format!("Reading stream disconnected -- {}", e),
                        InfoLevel::Warning,
                    );

                    return Err(StreamException::ReadStreamFailure);
                },
            };
        }
    }

    fn parse_request(source: &str, store: &mut Box<Request>) -> RouteHandler {
        if source.is_empty() {
            return RouteHandler::default();
        }

        let mut res = RouteHandler::default();
        let mut baseline_chan = None;
        let mut remainder_chan = None;

        for (index, info) in source.trim().splitn(2, "\r\n").enumerate() {
            match index {
                0 => baseline_chan = parse_start_line(&info, store),
                1 => {
                    let remainder: String = info.to_owned();
                    if remainder.is_empty() {
                        break;
                    }

                    let (tx_remainder, rx_remainder) = channel::bounded(1);
                    let mut header: HashMap<String, String> = HashMap::new();
                    let mut cookie: HashMap<String, String> = HashMap::new();
                    let mut body: Vec<String> = Vec::with_capacity(64);

                    shared_pool::run(move || {
                        let mut is_body = false;
                        for line in remainder.lines() {
                            if line.is_empty() && !is_body {
                                // meeting the empty line dividing header and body
                                is_body = true;
                                continue;
                            }

                            parse_headers(
                                line,
                                is_body,
                                &mut header,
                                &mut cookie,
                                &mut body,
                            );
                        }

                        if tx_remainder.send((header, cookie, body)).is_err() {
                            debug::print(
                                "Unable to construct the remainder of the request.",
                                InfoLevel::Error,
                            );
                        }
                    }, TaskType::Request);

                    remainder_chan = Some(rx_remainder)
                }
                _ => break,
            }
        }

        if let Some(rx) = baseline_chan {
            if let Ok(result) = rx.recv_timeout(Duration::from_millis(128)) {
                res = result.0;
                if res.is_some() {
                    store.create_param(result.1);
                }
            }

            if let Some(chan) = remainder_chan {
                if let Ok((header, cookie, body)) = chan.recv_timeout(Duration::from_secs(8)) {
                    store.set_headers(header);
                    store.set_cookies(cookie);
                    store.set_bodies(body);
                }
            }
        }

        res
    }

    fn parse_start_line(source: &str, req: &mut Box<Request>) -> BaseLine {
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
                        _ => REST::OTHER(info.to_uppercase())
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

            let (tx, rx) = channel::bounded(1);
            shared_pool::run(
                move || Route::seek(&req_method, &uri, tx),
                TaskType::Request,
            );

            // now do more work on non-essential parsing
            if !raw_fragment.is_empty() {
                req.set_fragment(raw_fragment);
            }

            if !raw_scheme.is_empty() {
                req.create_scheme(parse_scheme(raw_scheme));
            }

            return Some(rx);
        }

        None
    }

    fn stream_shutdown(stream: &mut Stream) -> u8 {
        if let Err(err) = stream.shutdown(Shutdown::Both) {
            debug::print(
                &format!(
                    "Encountered errors while shutting down the trunked body stream: {}",
                    err
                ),
                InfoLevel::Warning,
            );
            return 1;
        }

        0
    }
}