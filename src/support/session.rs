#![allow(unused_variables)]
#![allow(unused_mut)]
#![allow(dead_code)]

extern crate rand;

use std::collections::HashMap;
use std::cmp::Ordering;
use std::fs::{File};
use std::ops::*;
use std::path::Path;
use std::sync::{Arc, RwLock};
use std::time;
use std::thread;
use std::thread::*;

use chrono::prelude::*;
use chrono::Duration;
use rand::{thread_rng, Rng};

static DELEM_LV_1: char = '\u{0005}';
static DELEM_LV_2: char = '\u{0006}';
static DELEM_LV_3: char = '\u{0007}';
static DELEM_LV_4: char = '\u{0008}';

lazy_static! {
    static ref STORE: Arc<RwLock<HashMap<String, Session>>> = Arc::new(RwLock::new(HashMap::new()));
    static ref DEFAULT_LIFETIME: Arc<RwLock<Duration>> = Arc::new(RwLock::new(Duration::seconds(172800)));
}

pub struct Session {
    id: String,
    expires_at: DateTime<Utc>,
    auto_renewal: bool,
    store: HashMap<String, String>,
}

impl Session {
    pub fn clone(&self) -> Self {
        Session {
            id: self.id.to_owned(),
            expires_at: self.expires_at.clone(),
            auto_renewal: self.auto_renewal,
            store: self.store.clone(),
        }
    }

    pub fn to_owned(&self) -> Self {
        Session {
            id: self.id.to_owned(),
            expires_at: self.expires_at.to_owned(),
            auto_renewal: self.auto_renewal,
            store: self.store.to_owned(),
        }
    }

    fn serialize(&self) -> String {
        let mut result = String::new();
        if self.id.is_empty() { return result; }

        let expires_at = self.expires_at.to_rfc3339();
        result.push_str(&format!("{}{}", self.id.to_owned(), DELEM_LV_2));
        result.push_str(&format!("{}{}", expires_at, DELEM_LV_2));
        result.push_str(&format!("{}{}", self.auto_renewal.to_string(), DELEM_LV_2));

        for (key, val) in self.store.iter() {
            let entry = format!("{}{}{}", *key, DELEM_LV_4, *val);
            result.push_str(&format!("{}{}", entry, DELEM_LV_3));
        }

        result
    }

    fn deserialize(raw: &str, default_expires: DateTime<Utc>) -> Option<Session> {
        //TODO: if expired, skip
        let mut id = String::new();
        let mut expires_at = default_expires.clone();
        let mut auto_renewal = false;
        let mut store = HashMap::new();

        for (index, field) in raw.trim().split(DELEM_LV_2).enumerate() {
            match index {
                0 => id = field.to_owned(),
                1 => {
                    if let Ok(parsed_expiration) = field.parse::<DateTime<Utc>>() {
                        expires_at = parsed_expiration;
                    }
                },
                2 => if field.eq("true") { auto_renewal = true; },
                3 => parse_session_store(store.to_owned(), field),
                _ => { break; },
            }
        }

        if !id.is_empty() {
            return Some(Session {
                id,
                expires_at,
                auto_renewal,
                store,
            });
        }

        None
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
    fn clean_up_to(lifetime: DateTime<Utc>);
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
                if val.expires_at.cmp(&Utc::now()) != Ordering::Less {
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
            clean_up_to(Utc::now());
        });
    }

    fn clean_up_to(lifetime: DateTime<Utc>) {
        let now = Utc::now();
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
            if period.cmp(&Duration::seconds(60)) == Ordering::Less {
                time::Duration::from_secs(60)
            } else {
                time::Duration::from_millis(period.num_milliseconds() as u64)
            };

        let handler: JoinHandle<_> = thread::spawn(move || {
            loop {
                thread::sleep(sleep_period);
                clean_up_to(Utc::now());
            }
        }, );

        handler.thread().to_owned()
    }
}

pub trait SessionHandler {
    fn get_id(&self) -> String;
    fn get_value(&self, key: &str) -> Option<String>;
    fn set_value(&mut self, key: &str, val: &str) -> Option<String>;
    fn auto_lifetime_renew(&mut self, auto_renewal: bool);
    fn expires_at(&mut self, expires_at: DateTime<Utc>);
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

    fn auto_lifetime_renew(&mut self, auto_renewal: bool) {
        self.auto_renewal = auto_renewal;
    }

    // Set the expires system time. This will turn off auto session life time
    // renew if it's set.
    fn expires_at(&mut self, expires_time: DateTime<Utc>) {
        if self.auto_renewal {
            self.auto_renewal = false;
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
        let save_path = path.to_owned();
        thread::spawn(move || {
            let mut file: File =
                if let Ok(dest_file) = File::create(&save_path) {
                    dest_file
                } else {
                    // can't create file, abort saving
                    return;
                };

            if let Ok(store) = STORE.read() {
                let mut session: Vec<u8> = Vec::new();
                for (_, val) in store.iter() {

                }

                //match file.write_all() {}
            }
        });
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
        auto_renewal: true,
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
        let begin = Utc::now();
        let mut count = 1;

        loop {
            if !store.contains_key(&next_id) {
                return Some(next_id);
            }

            if count % 32 == 0 {
                count = 1;
                if Utc::now().signed_duration_since(begin).cmp(&Duration::milliseconds(256)) == Ordering::Greater {
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
        if session.auto_renewal {
            session.expires_at = get_next_expiration();
        }

        let old_session = store.insert(id, session.to_owned());
        drop(old_session);

        true
    } else {
        false
    }
}

fn get_next_expiration() -> DateTime<Utc> {
    if let Ok(default_lifetime) = DEFAULT_LIFETIME.read() {
        Utc::now().add(*default_lifetime)
    } else {
        Utc::now().add(Duration::seconds(172800))
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

fn clean_up_to(time: DateTime<Utc>) {
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

fn parse_session_store(mut store: HashMap<String, String>, field: &str) {
    if field.is_empty() { return; }

    for (_, entry) in field.trim().split(DELEM_LV_3).enumerate() {
        if let Some(pos) = entry.find(DELEM_LV_4) {
            let (key, value): (&str, &str) = entry.split_at(pos);
            if !key.is_empty() {
                store.entry(key.to_owned()).or_insert(value.to_owned());
            }
        }
    }
}
