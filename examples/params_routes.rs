extern crate rusty_express;

use rusty_express::prelude::*;

fn main() {
    // define http server now
    let mut server = HttpServer::new();
    server.set_pool_size(12);

    //define router directly
    server.get(RequestPath::Explicit("/"), simple_response);
    server.get(RequestPath::ExplicitWithParams("/api/user/:id"), user_param_response);
    server.get(RequestPath::ExplicitWithParams("/api/user/:id1/friend/:id2"), user_param_response);
    server.get(RequestPath::ExplicitWithParams("/api/blog/:id/:dates"), blog_param_response);

    server.listen(8080);
}

pub fn simple_response(req: &Box<Request>, resp: &mut Box<Response>) {
    resp.send(&format!("Hello world from rusty server from {}!<br />", req.uri));
    resp.status(200);
    resp.set_content_type("text/html");
}

pub fn user_param_response(req: &Box<Request>, resp: &mut Box<Response>) {
    resp.send(&format!("Hello world from rusty server from {}<br />", req.uri));

    resp.send(&format!("<ul>"));
    for param in req.param_iter() {
        resp.send(&format!("<li>Param: [{}] --- Set as: [{}]</li>", param.0, param.1));
    }
    resp.send(&format!("</ul>"));

    resp.status(200);
    resp.set_content_type("text/html");
}

pub fn blog_param_response(req: &Box<Request>, resp: &mut Box<Response>) {
    resp.send(&format!("Hello world from rusty server from {}<br />", req.uri));

    resp.send(&format!("<ul>"));
    for param in req.param_iter() {
        resp.send(&format!("<li>Param: [{}] --- Set as: [{}]</li>", param.0, param.1));
    }
    resp.send(&format!("</ul>"));

    resp.status(200);
    resp.set_content_type("text/html");
}