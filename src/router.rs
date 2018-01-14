
use std::collections::HashMap;

pub enum REST {
    NONE,
    GET,
    POST,
    PUT,
    DELETE,
}

pub struct Route {
    pub get: HashMap<String, fn(String) -> String>,
    pub post: HashMap<String, fn(String) -> String>,
    pub put: HashMap<String, fn(String) -> String>,
    pub delete: HashMap<String, fn(String) -> String>,
}

impl Route {

}
