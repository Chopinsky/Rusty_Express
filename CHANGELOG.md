# 2019-01
## 0.4.2
- Small fixes

# 2019-01
## 0.4.1
- Default read/write timeout to 0, unless specified otherwise
- More rewrite to boost performance

# 2019-01
## 0.4.0
- Performance improvement to request parser
- Switching to use crossbeam_channel for async communications in the main connection workflow
- Fixing various small bugs

# 2018-10
## 0.3.6
- Router has been updated for better performance.
- Native logger service. More documentation coming in 0.4.0.
- Next version will be in 0.4.x after updating to the Rust 20118 version and fixing lexical differences. 

# 2018-08
## 0.3.5
- Now you can define regular expressions for validating the `RequestPath::ExplicitWithParams` 
routes. For example, your parameterized route can now be defined as: `/api/:userId(\d{7})` which only allows users with 
7 digits IDs. This will help reduce the server burden if the incoming request is trying to guess the parameters.
- The following config related APIs are changed to be static methods, and you can use them thread-safe now:
 
 Before 0.3.5  | After 0.3.5
 ------------- | -------------
 config.use_default_header(...)  | ServerConfig::use_default_header(...)
 config.set_default_header(...)  | ServerConfig::set_default_header(...)
 config.set_status_page_generator(...)  | ServerConfig::set_status_page_generator(...)

## 0.3.4
- Fixing bugs

# 2018-07
## 0.3.3
- Removing the `state_interaction` mechanism. The replacement feature, the server `context` module, has been introduced
in 0.3.0
- Now supporting hot-loading of server `config` and `router` objects, which could help reduce the needs of the server
downtime.
- Providing request's IP, if that information is available.
- Now you can use "all" API to add a route to all accepted requests.

# 2018-06
## 0.3.2
- Update 'session' module to be more robust for use with generic session data types.
- Improving documentation.

## What's new in 0.3.1
- Fixing a few obvious bugs and improve the performance.
- Now the template framework is mostly done. A simple template engine will be added in the next main version (0.3.3).

# 2018-05
## Major version break: 0.3.0
0.2.x versions are good experiments with this project. But we're growing fast with better
features and more performance enhancement! That's why we need to start the 0.3.x versions
with slight changes to the interface APIs.

## Migrating from 0.2.x to 0.3.0
Here're what to expect when updating from 0.2.x to 0.3.0:

- The route handler function's signature has changed, now the request and response objects
are boxed! So now your route handler should have something similar to this:
```rust
pub fn handler(req: &Box<Request>, resp: &mut Box<Response>) {
    /// work hard to generate the response here...
}
```

- The `StateProvider` trait is deprecated (and de-factor no-op in 0.3.0), and it will be removed in
the 0.3.3 release. Please switch to use the `ServerContext` features instead. You can find how to
use the `ServerContext` in this example: [Server with defined router](https://github.com/Chopinsky/Rusty_Express/blob/master/examples/use_router.rs)