#![allow(unused_variables)]
extern crate rand;

use std::collections::HashMap;
use std::cmp::Ordering;
use std::ops::*;
use std::path::Path;
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime};
use std::thread;
use std::thread::*;
use rand::{thread_rng, Rng};

lazy_static! {
    static ref STORE: Arc<RwLock<HashMap<String, Session>>> = Arc::new(RwLock::new(HashMap::new()));
    static ref DEFAULT_LIFETIME: Arc<RwLock<Duration>> = Arc::new(RwLock::new(Duration::new(172800, 0)));
}

pub struct Session {
    id: String,
    expires_at: SystemTime,
    auto_renew: bool,
    store: HashMap<String, String>,
}

impl Session {
    pub fn clone(&self) -> Self {
        Session {
            id: self.id.to_owned(),
            expires_at: self.expires_at.clone(),
            auto_renew: self.auto_renew,
            store: self.store.clone(),
        }
    }

    pub fn to_owned(&self) -> Self {
        Session {
            id: self.id.to_owned(),
            expires_at: self.expires_at.to_owned(),
            auto_renew: self.auto_renew,
            store: self.store.to_owned(),
        }
    }
}

pub trait SessionExchange {
    fn initialize_new() -> Option<Session>;
    fn initialize_new_with_id(id: &str) -> Option<Session>;
    fn from_id(id: String) -> Option<Session>;
    fn from_or_new(id: String) -> Option<Session>;
    fn release(id: String);
    fn set_default_session_lifetime(lifetime: Duration);
    fn clean();
    fn clean_up_to(lifetime: SystemTime);
    fn store_size() -> Option<usize>;
    fn start_auto_clean_queue(period: Duration) -> Thread;
}

impl SessionExchange for Session {
    fn initialize_new() -> Option<Self> {
        new_session("")
    }

    fn initialize_new_with_id(id: &str) -> Option<Self> {
        new_session(id)
    }

    fn from_id(id: String) -> Option<Self> {
        if let Ok(store) = STORE.read() {
            if let Some(val) = store.get(&id) {
                if val.expires_at.cmp(&SystemTime::now()) != Ordering::Less {
                    //found the session, return now
                    return Some(val.to_owned());

                } else {
                    //expired, remove it from the store
                    thread::spawn(move || {
                        release(id);
                    });

                    return None;
                }
            }
        }

        None
    }

    fn from_or_new(id: String) -> Option<Self> {
        if let Some(session) = Session::from_id(id) {
            Some(session)
        } else {
            Session::initialize_new()
        }
    }

    fn release(id: String) {
        thread::spawn(move || {
            release(id);
        });
    }

    fn set_default_session_lifetime(lifetime: Duration) {
        thread::spawn(move || {
            if let Ok(mut default_lifetime) = DEFAULT_LIFETIME.write() {
                *default_lifetime = lifetime;
            }
        });
    }

    fn clean() {
        thread::spawn(move || {
            clean_up_to(SystemTime::now());
        });
    }

    fn clean_up_to(lifetime: SystemTime) {
        let now = SystemTime::now();
        let time =
            if lifetime.cmp(&now) != Ordering::Greater {
                now
            } else {
                lifetime
            };

        thread::spawn(move || {
            clean_up_to(time);
        });
    }

    fn store_size() -> Option<usize> {
        if let Ok(store) = STORE.read() {
            Some(store.keys().len())
        } else {
            None
        }
    }

    fn start_auto_clean_queue(period: Duration) -> Thread {
        let sleep_period =
            if period.cmp(&Duration::new(60, 0)) == Ordering::Less {
                Duration::new(60, 0)
            } else {
                period
            };

        let handler: JoinHandle<_> = thread::spawn(move || {
            loop {
                thread::sleep(sleep_period);
                clean_up_to(SystemTime::now());
            }
        }, );

        handler.thread().to_owned()
    }
}

pub trait SessionHandler {
    fn get_id(&self) -> String;
    fn get_value(&self, key: &str) -> Option<String>;
    fn set_value(&mut self, key: &str, val: &str) -> Option<String>;
    fn auto_lifetime_renew(&mut self, auto_renew: bool);
    fn expires_at(&mut self, expires_at: SystemTime);
    fn save(&mut self);
}

impl SessionHandler for Session {
    fn get_id(&self) -> String {
        self.id.to_owned()
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

    // Set the expires system time. This will turn off auto session life time
    // renew if it's set.
    fn expires_at(&mut self, expires_time: SystemTime) {
        if self.auto_renew {
            self.auto_renew = false;
        }

        self.expires_at = expires_time;
    }

    fn save(&mut self) {
        save(self.id.to_owned(), self);
    }
}

pub trait PersistHandler {
    fn from_file(path: &Path);
    fn to_file(&self, path: &Path);
}

impl PersistHandler for Session {
    fn from_file(path: &Path) {
        //TODO: from file
    }

    fn to_file(&self, path: &Path) {
        //TODO: to file
    }
}

fn new_session(id: &str) -> Option<Session> {
    let next_id: String;
    if id.is_empty() {
        next_id = match gen_session_id(16) {
            Some(val) => val,
            None => String::new(),
        };

        if next_id.is_empty() { return None; }
    } else {
        next_id = id.to_owned();
    }

    let session = Session {
        id: next_id,
        expires_at: get_next_expiration(),
        auto_renew: true,
        store: HashMap::new(),
    };

    if let Ok(mut store) = STORE.write() {
        //if key already exists, override to protect session scanning
        store.insert(session.id.to_owned(), session.to_owned());
        Some(session)
    } else {
        None
    }
}

fn gen_session_id(id_size: usize) -> Option<String> {
    let size =
        if id_size < 16 {
            16
        } else {
            id_size
        };

    let mut next_id: String =
        thread_rng().gen_ascii_chars().take(size).collect();

    if let Ok(store) = STORE.read() {
        let now = SystemTime::now();
        let mut count = 1;

        loop {
            if !store.contains_key(&next_id) {
                return Some(next_id);
            }

            if count % 32 == 0 {
                count = 1;
                if now.elapsed().unwrap() > Duration::from_millis(256) {
                    // 256 milli-sec for get a good guess is already too expansive...
                    return None;
                }
            }

            // now take the next guess
            next_id = thread_rng().gen_ascii_chars().take(32).collect();
            count += 1;
        }
    }

    None
}

fn save(id: String, session: &mut Session) -> bool {
    if let Ok(mut store) = STORE.write() {
        if session.auto_renew {
            session.expires_at = get_next_expiration();
        }

        let old_session = store.insert(id, session.to_owned());
        drop(old_session);

        true
    } else {
        false
    }
}

fn get_next_expiration() -> SystemTime {
    if let Ok(default_lifetime) = DEFAULT_LIFETIME.read() {
        SystemTime::now().add(*default_lifetime)
    } else {
        SystemTime::now().add(Duration::new(172800, 0))
    }
}

fn release(id: String) -> bool {
    if let Ok(mut store) = STORE.write() {
        store.remove(&id);
    } else {
        return false;
    }

    true
}

fn clean_up_to(time: SystemTime) {
    let mut stale_sessions: Vec<String> = Vec::new();
    if let Ok(mut store) = STORE.write() {
        for session in store.values() {
            if session.expires_at.cmp(&time) != Ordering::Greater {
                stale_sessions.push(session.id.to_owned());
            }
        }

        println!("Cleaned: {}", stale_sessions.len());

        for id in stale_sessions {
            store.remove(&id);
        }
    }

    println!("Session clean done!");
}
