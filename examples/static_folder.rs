extern crate rusty_express;

use std::path::PathBuf;
use rusty_express::prelude::*;

fn main() {
    // define http server now
    let mut server = HttpServer::new();

    //define router directly
    server.use_static(PathBuf::from(r".\examples\static"));

    server.listen(8080);
}
