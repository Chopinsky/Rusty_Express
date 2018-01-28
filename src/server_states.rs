pub struct ServerStates {
    going_to_shutdown: bool,
}

impl ServerStates {
    pub fn new() -> Self {
        ServerStates {
            going_to_shutdown: false,
        }
    }

    pub fn is_terminating(&self) -> bool {
        self.going_to_shutdown
    }

    pub fn set_to_terminate(&mut self) {
        self.going_to_shutdown = true;
    }
}