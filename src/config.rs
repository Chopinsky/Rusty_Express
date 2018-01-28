pub struct ServerConfig {
    pub pool_size: usize,
    pub read_timeout: u8,
    pub write_timeout: u8,
}

impl ServerConfig {
    pub fn new() -> Self {
        ServerConfig {
            pool_size: 4,
            read_timeout: 5,
            write_timeout: 5,
        }
    }
}