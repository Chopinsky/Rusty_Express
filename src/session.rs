use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime};

lazy_static! {
    static ref STORE: Arc<Mutex<HashMap<u32, &'static str>>> = Arc::new(Mutex::new(HashMap::new()));

    static ref LIFE_MAP: Arc<Mutex<HashMap<u32, SystemTime>>> = Arc::new(Mutex::new(HashMap::new()));

    static ref NEXT_ID: Arc<Mutex<u32>> = Arc::new(Mutex::new(1));
}

pub struct Session {
    id: u32,
    store: String
}

fn new_id() -> Option<u32> {
    let next_id = NEXT_ID.clone();
    let new_id: u32;

    if let Ok(mut id) = next_id.lock() {
        *id = *id + 1;
        new_id = *id;
    } else {
        return None;
    }

    Some(new_id)
}

impl Session {
    pub fn new() -> Self {
        if let Some(new_id) = new_id() {
            Session {
                id: new_id,
                store: String::new(),
            }
        } else {
            Session {
                id: 0,
                store: String::new(),
            }
        }
    }

    pub fn get_id(&self) -> u32 {
        self.id
    }

    pub fn get_store(&self) -> String {
        self.store.to_owned()
    }

    pub fn set_store(&mut self, new_store: String, replace: bool) {
        if replace {
            self.store = new_store.to_owned();
        } else {
            self.store.push_str(&new_store);
        }
    }
}