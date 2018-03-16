#![allow(unused_variables)]

use std::thread::*;

use core::http::{Request, Response};
use support::session::*;

pub struct ServerStates {
    going_to_shutdown: bool,
    session_auto_clean_handler: Option<Thread>,
}

impl ServerStates {
    pub fn new() -> Self {
        ServerStates {
            going_to_shutdown: false,
            session_auto_clean_handler: None
        }
    }

    pub fn is_terminating(&self) -> bool {
        self.going_to_shutdown
    }

    pub fn ack_to_terminate(&mut self) {
        self.going_to_shutdown = true;
    }

    pub fn set_session_handler(&mut self, handler: &Thread) {
        self.session_auto_clean_handler = Some(handler.to_owned());
    }

    pub fn drop_session_auto_clean(&mut self) {
        if let Some(handler) = self.session_auto_clean_handler.to_owned() {
            Session::auto_clean_stop();
            drop(handler);
        }
    }
}

#[derive(PartialEq, Eq, Clone, Copy)]
pub enum StatesInteraction {
    WithRequest,
    WithResponse,
    Both,
    None,
}

pub type RequireStateUpdates = bool;

pub trait StatesProvider {
    fn interaction_stage(&self) -> StatesInteraction;
    fn on_request(&self, req: &mut Box<Request>) -> RequireStateUpdates;
    fn on_response(&self, resp: &mut Box<Response>) -> RequireStateUpdates;
    fn update(&mut self, req: &Box<Request>, resp: Option<&Box<Response>>);
}

pub struct EmptyState {}

impl Clone for EmptyState {
    fn clone(&self) -> Self { EmptyState {} }
}

impl StatesProvider for EmptyState {
    #[inline]
    fn interaction_stage(&self) -> StatesInteraction {
        StatesInteraction::None
    }

    #[inline]
    fn on_request(&self, req: &mut Box<Request>) -> RequireStateUpdates {
        false
    }

    #[inline]
    fn on_response(&self, resp: &mut Box<Response>) -> RequireStateUpdates {
        false
    }

    #[inline]
    fn update(&mut self, req: &Box<Request>, resp: Option<&Box<Response>>) { }
}