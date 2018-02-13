use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime};
use std::thread;

lazy_static! {
    static ref STORE: Arc<Mutex<HashMap<u32, HashMap<String, String>>>> = Arc::new(Mutex::new(HashMap::new()));

    static ref LIFE_MAP: Arc<Mutex<HashMap<u32, SystemTime>>> = Arc::new(Mutex::new(HashMap::new()));

    static ref NEXT_ID: Arc<Mutex<u32>> = Arc::new(Mutex::new(1));
}

pub struct Session {
    id: u32,
    store: HashMap<String, String>,
}

pub trait SessionExchange {
    fn new() -> Session;
    fn from_id(id: u32) -> Option<Session>;
    fn from_or_new(id: u32) -> Session;
    fn release(id: u32);
}

impl SessionExchange for Session {
    fn new() -> Self {
        if let Some(new_id) = new_id() {
            Session {
                id: new_id,
                store: HashMap::new(),
            }
        } else {
            Session {
                id: 0,
                store: HashMap::new(),
            }
        }
    }

    fn from_id(id: u32) -> Option<Self> {
        if let Ok(store) = STORE.lock() {
            if let Some(val) = store.get(&id) {
                Some(Session {
                   id,
                   store: val.to_owned(),
                })
            } else {
                None
            }
        } else {
            None
        }
    }

    fn from_or_new(id: u32) -> Self {
        if let Some(session) = Session::from_id(id) {
            session
        } else {
            Session::new()
        }
    }

    fn release(id: u32) {
        release(id);
    }
}

pub trait SessionHandler {
    fn get_id(&self) -> u32;
    fn get_value(&self, key: &str) -> Option<String>;
    fn set_value(&mut self, key: &str, val: &str) -> Option<String>;
    fn save(&self);
}

impl SessionHandler for Session {
    fn get_id(&self) -> u32 {
        self.id
    }

    fn get_value(&self, key: &str) -> Option<String> {
        if let Some(val) = self.store.get(key) {
            Some(val.to_owned())
        } else {
            None
        }
    }

    // Set new session key-value pair, returns the old value if the key
    // already exists
    fn set_value(&mut self, key: &str, val: &str) -> Option<String> {
        self.store.insert(key.to_owned(), val.to_owned())
    }

    fn save(&self) {
        save(self.id, self.store.to_owned());
    }
}

fn new_id() -> Option<u32> {
    if let Ok(mut id) = NEXT_ID.lock() {
        *id = *id + 1;
        let new_id = *id;
        return Some(new_id);
    } else {
        return None;
    }
}

fn save(id: u32, content: HashMap<String, String>) -> bool {
    if let Ok(mut store) = STORE.lock() {
        store.insert(id, content);

        thread::spawn(move || {
            update_last_access(id, false);
        });

    } else {
        return false;
    }

    true
}

fn release(id: u32) -> bool {
    if let Ok(mut store) = STORE.lock() {
        store.remove(&id);

        thread::spawn(move || {
            update_last_access(id, true);
        });

    } else {
        return false;
    }

    true
}

fn update_last_access(id: u32, to_remove: bool) {
    if let Ok(mut map) = LIFE_MAP.lock() {
        if !to_remove {
            let now = SystemTime::now();
            map.insert(id, now);
        } else {
            map.remove(&id);
        }
    }
}
