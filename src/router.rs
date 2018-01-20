#![allow(dead_code)]

use std::collections::HashMap;
use http::Request;

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

#[derive(PartialEq, Eq, Hash)]
pub enum RequestPath {
    Literal(&'static str),
    WildCard(&'static str),
}

/* Manual mayham...

//impl PartialEq for RequestPath {
//    fn eq(&self, other: &RequestPath) -> bool {
//        match self {
//            &RequestPath::Literal(lit_val) => {
//                match other {
//                    &RequestPath::Literal(other_val) => lit_val == other_val,
//                    _ => false,
//                }
//            },
//            &RequestPath::WildCard(wild_card_val) => {
//                match other {
//                    &RequestPath::WildCard(other_val) => wild_card_val == other_val,
//                    _ => false,
//                }
//            }
//        }
//    }
//}
//
//impl Eq for RequestPath {}
//
//impl Hash for RequestPath {
//    fn hash<H: Hasher>(&self, state: &mut H) {
//        match self {
//            &RequestPath::Literal(lit_val) => lit_val.hash(state),
//            &RequestPath::WildCard(wild_card_val) => wild_card_val.hash(state)
//        }
//    }
//}

 * End of manual mayham
 */

pub type Callback = fn(String, Request) -> String;

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

impl Route {
    pub fn new() -> Self {
        Route {
            get: HashMap::new(),
            post: HashMap::new(),
            put: HashMap::new(),
            delete: HashMap::new(),
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
