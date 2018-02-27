use std::thread::*;

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
            drop(handler);
        }
    }
}