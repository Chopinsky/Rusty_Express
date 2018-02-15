extern crate rand;

use std::collections::HashMap;
use std::cmp::Ordering;
use std::ops::*;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};
use rand::Rng;

lazy_static! {
    static ref STORE: Arc<Mutex<HashMap<u32, Session>>> = Arc::new(Mutex::new(HashMap::new()));
    static ref DEFAULT_LIFETIME: Arc<Mutex<Duration>> = Arc::new(Mutex::new(Duration::new(172800, 0)));
}

pub struct Session {
    id: u32,
    expires_at: SystemTime,
    auto_renew: bool,
    store: HashMap<String, String>,
}

impl Session {
    pub fn clone(&self) -> Self {
        Session {
            id: self.id,
            expires_at: self.expires_at.clone(),
            auto_renew: self.auto_renew,
            store: self.store.clone(),
        }
    }

    pub fn to_owned(&self) -> Self {
        Session {
            id: self.id,
            expires_at: self.expires_at.to_owned(),
            auto_renew: self.auto_renew,
            store: self.store.to_owned(),
        }
    }
}

pub trait SessionExchange {
    fn new() -> Option<Session>;
    fn from_id(id: u32) -> Option<Session>;
    fn from_or_new(id: u32) -> Option<Session>;
    fn release(id: u32);
    fn set_default_session_lifetime(lifetime: Duration);
}

impl SessionExchange for Session {
    fn new() -> Option<Self> {
        new_session()
    }

    fn from_id(id: u32) -> Option<Self> {
        if let Ok(store) = STORE.lock() {
            if let Some(val) = store.get(&id) {
                if val.expires_at.cmp(&SystemTime::now()) != Ordering::Less {
                    //found the session, return now
                    return Some(val.to_owned());

                } else {
                    //expired, remove it from the store
                    release(id);
                    return None;

                }
            }
        }

        None
    }

    fn from_or_new(id: u32) -> Option<Self> {
        if let Some(session) = Session::from_id(id) {
            Some(session)
        } else {
            Session::new()
        }
    }

    fn release(id: u32) {
        release(id);
    }

    fn set_default_session_lifetime(lifetime: Duration) {
        if let Ok(mut default_lifetime) = DEFAULT_LIFETIME.lock() {
            *default_lifetime = lifetime;
        }
    }
}

pub trait SessionHandler {
    fn get_id(&self) -> u32;
    fn get_value(&self, key: &str) -> Option<String>;
    fn set_value(&mut self, key: &str, val: &str) -> Option<String>;
    fn auto_lifetime_renew(&mut self, auto_renew: bool);
    fn expires_at(&mut self, expires_at: SystemTime);
    fn save(&mut self);
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

    fn auto_lifetime_renew(&mut self, auto_renew: bool) {
        self.auto_renew = auto_renew;
    }

    fn expires_at(&mut self, expires_time: SystemTime) {
        if self.auto_renew {
            self.auto_renew = false;
        }

        self.expires_at = expires_time;
    }

    fn save(&mut self) {
        save(self.id, self);
    }
}

fn new_session() -> Option<Session> {
    if let Ok(mut store) = STORE.lock() {
        let now = SystemTime::now();
        let mut rng = rand::thread_rng();
        let mut next_id = rng.gen::<u32>();

        loop {
            if !store.contains_key(&next_id) { break; }

            if let Ok(elapsed) = now.elapsed() {
                // 1 sec for get a good guess is already too expansive...
                if elapsed.as_secs() > 1 { return None; }
            }

            // now take the next guess
            next_id = rng.gen::<u32>();
        }

        let session = Session {
            id: next_id,
            expires_at: get_expires_lifetime(),
            auto_renew: true,
            store: HashMap::new(),
        };

        store.insert(next_id, session.to_owned());
        Some(session)

    } else {
        None

    }
}

fn save(id: u32, session: &mut Session) -> bool {
    if let Ok(mut store) = STORE.lock() {
        if session.auto_renew {
            session.expires_at = get_expires_lifetime();
        }

        let old_store = store.insert(id, session.to_owned());
        drop(old_store);

    } else {
        return false;
    }

    true
}

fn get_expires_lifetime() -> SystemTime {
    if let Ok(default_lifetime) = DEFAULT_LIFETIME.lock() {
        SystemTime::now().add(*default_lifetime)
    } else {
        SystemTime::now().add(Duration::new(172800, 0))
    }
}

fn release(id: u32) -> bool {
    if let Ok(mut store) = STORE.lock() {
        store.remove(&id);
    } else {
        return false;
    }

    true
}
