extern crate rusty_express;

use rusty_express::prelude::*;

fn main() {
    // define http server now
    let mut server = HttpServer::new();
    server.set_pool_size(8);

    //define router directly
    server
        .get(RequestPath::Explicit("/"), simple_response)
        .get(RequestPath::Explicit("/index"), simple_redirect)
        .get(RequestPath::Explicit("/fail_check"), simple_redir_fail);

    server.listen(8080);
}

pub fn simple_response(_req: &Box<Request>, resp: &mut Box<Response>) {
    //this content will be skipped because of the redirection
    resp.send("Hello world from rusty server!\n");

    //call redirect
    resp.redirect("/index");
}

pub fn simple_redirect(_req: &Box<Request>, resp: &mut Box<Response>) {
    resp.send("Now empowered with the redirect!\n");
}

pub fn simple_redir_fail(_req: &Box<Request>, resp: &mut Box<Response>) {
    //call redirect
    resp.redirect("/fail");
}
