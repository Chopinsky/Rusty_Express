use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime};
use std::thread;

lazy_static! {
    static ref STORE: Arc<Mutex<HashMap<u32, String>>> = Arc::new(Mutex::new(HashMap::new()));

    static ref LIFE_MAP: Arc<Mutex<HashMap<u32, SystemTime>>> = Arc::new(Mutex::new(HashMap::new()));

    static ref NEXT_ID: Arc<Mutex<u32>> = Arc::new(Mutex::new(1));
}

pub struct Session {
    id: u32,
    store: String
}

pub trait SessionExchange {
    fn new() -> Session;
    fn from(id: u32) -> Session;
}

impl SessionExchange for Session {
    fn new() -> Self {
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

    fn from(id: u32) -> Self {
        
    }
}

impl Session {
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

    pub fn save(&self) {
        if self.store.is_empty() {

        } else {
            save(self.id, &self.store[..]);
        }
    }
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

fn save(id: u32, content: &str) -> bool {
    if let Ok(mut store) = STORE.lock() {
        if store.contains_key(&id) {
            if let Some(session) = store.get_mut(&id) {
                *session = content.to_owned();
            } else {
                return false;
            }
        } else {
            store.insert(id, content.to_owned());
        }
    } else {
        return false;
    }

    thread::spawn(move || {
        update_last_access(id);
    });

    true
}

fn update_last_access(id: u32) {
    let now = SystemTime::now();
    if let Ok(mut map) = LIFE_MAP.lock() {
        if map.contains_key(&id) {
            if let Some(session) = map.get_mut(&id) {
                *session = now;
            }
        } else {
            map.insert(id, now);
        }
    }
}
