extern crate rusty_express;

use rusty_express::prelude::*;
use std::path::PathBuf;

fn main() {
    // define http server now
    let mut server = HttpServer::new();

    //define router directly
    server
        .use_static(PathBuf::from(r".\examples\static"))
        .use_custom_static(
            RequestPath::Explicit("/path/to/folder"),
            PathBuf::from(r".\examples\static"),
        )
        .use_custom_static(
            RequestPath::ExplicitWithParams("/path/to/user/:id"),
            PathBuf::from(r".\examples\static"),
        );

    server.listen(8080);
}
