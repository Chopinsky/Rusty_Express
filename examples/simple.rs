extern crate rusty_express;

use rusty_express::prelude::*;

fn main() {
    // define http server now
    let mut server = HttpServer::new();

    //define router directly
    server.get(RequestPath::WildCard(r"/\w*"), simple_response);

    server.listen(8080);
}

pub fn simple_response(req: &Box<Request>, resp: &mut Box<Response>) {
    /*        Test: generate new Sessions
    //        if let Some(mut session) = Session::new() {
    //            session.expires_at(SystemTime::now().add(Duration::new(5, 0)));
    //            session.save();
    //            println!("New session: {}", session.get_id());
    //        }
     */

    resp.send(&format!(
        "Hello world from rusty server from path: {}",
        req.uri
    ));
    resp.status(200);
}
