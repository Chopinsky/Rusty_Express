#![allow(dead_code)]

use std::collections::HashMap;
use regex::Regex;
use http::*;

pub enum REST {
    NONE,
    GET,
    POST,
    PUT,
    DELETE,
}

impl Default for REST {
    fn default() -> REST { REST::NONE }
}

#[derive(PartialEq, Eq, Hash, Clone, Copy)]
pub enum RequestPath {
    Literal(&'static str),
    WildCard(&'static str),
}

/* Manual mayham...

impl PartialEq for RequestPath {
    fn eq(&self, other: &RequestPath) -> bool {
        match self {
            &RequestPath::Literal(lit_val) => {
                match other {
                    &RequestPath::Literal(other_val) => lit_val == other_val,
                    _ => false,
                }
            },
            &RequestPath::WildCard(wild_card_val) => {
                match other {
                    &RequestPath::WildCard(other_val) => wild_card_val == other_val,
                    _ => false,
                }
            }
        }
    }
}

impl Eq for RequestPath {}

impl Hash for RequestPath {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            &RequestPath::Literal(lit_val) => lit_val.hash(state),
            &RequestPath::WildCard(wild_card_val) => wild_card_val.hash(state)
        }
    }
}

 * End of manual mayham
 */

pub type Callback = fn(String, &Request) -> String;

pub struct Route {
    get: HashMap<RequestPath, Callback>,
    post: HashMap<RequestPath, Callback>,
    put: HashMap<RequestPath, Callback>,
    delete: HashMap<RequestPath, Callback>,
}

pub trait Router {
    fn get(&mut self, uri: RequestPath, callback: Callback);
    fn post(&mut self, uri: RequestPath, callback: Callback);
    fn put(&mut self, uri: RequestPath, callback: Callback);
    fn delete(&mut self, uri: RequestPath, callback: Callback);
}

pub trait RouteHandler {
    fn handle_get(&self, req: Request) -> Option<Response>;
    fn handle_put(&self, req: Request) -> Option<Response>;
    fn handle_post(&self, req: Request) -> Option<Response>;
    fn handle_delete(&self, req: Request) -> Option<Response>;
}

impl Route {
    pub fn new() -> Self {
        Route {
            get: HashMap::new(),
            post: HashMap::new(),
            put: HashMap::new(),
            delete: HashMap::new(),
        }
    }

    pub fn from(source: &Route) -> Self {
        let mut router = Route::new();
        router.clone_from(&source);
        router
    }

    pub fn clone_from(&mut self, source: &Route) {
        for (key, val) in source.get.iter() {
            self.get.insert(key.to_owned(), val.to_owned());
        }

        for (key, val) in source.post.iter() {
            self.get.insert(key.to_owned(), val.to_owned());
        }

        for (key, val) in source.put.iter() {
            self.get.insert(key.to_owned(), val.to_owned());
        }

        for (key, val) in source.delete.iter() {
            self.get.insert(key.to_owned(), val.to_owned());
        }
    }
}

impl Router for Route {
    fn get(&mut self, uri: RequestPath, callback: Callback) {
        self.get.insert(uri, callback);
    }

    fn put(&mut self, uri: RequestPath, callback: Callback) {
        self.put.insert(uri, callback);
    }

    fn post(&mut self, uri: RequestPath, callback: Callback) {
        self.post.insert(uri, callback);
    }

    fn delete(&mut self, uri: RequestPath, callback: Callback) {
        self.delete.insert(uri, callback);
    }
}

impl RouteHandler for Route {
    fn handle_get(&self, req: Request) -> Option<Response> {
        handle_request_worker(&self.get, &req)
    }

    fn handle_put(&self, req: Request) -> Option<Response> {
        handle_request_worker(&self.put, &req)
    }

    fn handle_post(&self, req: Request) -> Option<Response> {
        handle_request_worker(&self.post, &req)
    }

    fn handle_delete(&self, req: Request) -> Option<Response> {
        handle_request_worker(&self.delete, &req)
    }
}

fn handle_request_worker(routes: &HashMap<RequestPath, Callback>, req: &Request) -> Option<Response> {
    let uri = req.path.clone();
    for (req_path, callback) in routes.iter() {
        match req_path.to_owned() {
            RequestPath::Literal(literal) => {
                if req.path.eq(literal) {

                    //TODO: write to the response stream
                    let result = callback(uri, req);

                    return Some(Response{
                        header: String::new(),
                        body: result,
                    });
                }
            },
            RequestPath::WildCard(wild) => {
                if let Ok(re) = Regex::new(wild) {
                    if re.is_match(&uri) {

                        //TODO: write to the response stream
                        let result = callback(uri, req);

                        return Some(Response{
                            header: String::new(),
                            body: result,
                        });
                    }
                }
            }
        }
    }

    None
}