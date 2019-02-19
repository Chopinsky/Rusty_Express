use chrono::prelude::*;
use std::cmp::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(PartialEq, Eq, Hash, Clone)]
pub enum KeyPrefix {
    Secure,
    Host,
}

pub struct Cookie {
    key: String,
    value: String,
    key_prefix: Option<KeyPrefix>,
    expires: Option<SystemTime>,
    max_age: Option<u32>,
    domain: String,
    path: String,
    secure: bool,
    http_only: bool,
}

impl Cookie {
    pub fn new(key: &str, value: &str) -> Self {
        Cookie {
            key: key.to_owned(),
            value: value.to_owned(),
            key_prefix: None,
            expires: None,
            max_age: None,
            domain: String::new(),
            path: String::new(),
            secure: false,
            http_only: false,
        }
    }

    pub fn set_key_prefix(&mut self, prefix: Option<KeyPrefix>) {
        self.key_prefix = match prefix {
            Some(KeyPrefix::Secure) => {
                self.secure = true;
                prefix
            }
            Some(KeyPrefix::Host) => {
                if !self.domain.is_empty() {
                    self.domain.clear();
                }
                self.path = String::from("/");
                self.secure = true;
                prefix
            }
            _ => prefix,
        };
    }

    pub fn set_expires(&mut self, expires_at: Option<SystemTime>) {
        self.expires = match expires_at {
            Some(time) if time.cmp(&SystemTime::now()) != Ordering::Greater => None,
            _ => expires_at,
        };
    }

    pub fn set_max_age(&mut self, max_age: Option<u32>) {
        self.max_age = max_age;
    }

    pub fn set_path(&mut self, path: &str) {
        self.path = match self.key_prefix {
            Some(KeyPrefix::Host) => String::new(),
            _ if path.is_empty() => String::new(),
            _ => {
                if path.starts_with("/") {
                    path.to_owned()
                } else {
                    panic!("Cookie path must start with '/'");
                }
            }
        };
    }

    pub fn set_domain(&mut self, domain: &str) {
        self.domain = match self.key_prefix {
            Some(KeyPrefix::Host) => String::new(),
            _ => domain.to_owned(),
        }
    }

    pub fn set_secure_attr(&mut self, is_secure: bool) {
        self.secure = match self.key_prefix {
            Some(KeyPrefix::Host) | Some(KeyPrefix::Secure) => true,
            _ => is_secure,
        };
    }

    pub fn set_http_only_attr(&mut self, http_only: bool) {
        self.http_only = http_only;
    }

    pub fn update_session_key(&mut self, key: &str) {
        if key.is_empty() {
            panic!("Session key must have a value!");
        }
        self.key = key.to_owned();
    }

    pub fn update_session_value(&mut self, value: &str) {
        if value.is_empty() {
            panic!("Session key must have a value!");
        }
        self.value = value.to_owned();
    }

    pub fn is_valid(&self) -> bool {
        (!self.key.is_empty()) && (!self.value.is_empty())
    }

    pub fn get_cookie_key(&self) -> String {
        self.key.to_owned()
    }

    pub fn get_cookie_value(&self) -> String {
        self.value.to_owned()
    }
}

impl ToString for Cookie {
    fn to_string(&self) -> String {
        if self.key.is_empty() || self.value.is_empty() {
            return String::new();
        }

        let mut cookie = match self.key_prefix {
            Some(KeyPrefix::Secure) => ["__Secure-", &self.key[..], "=", &self.value[..], ";"].join(""),
            Some(KeyPrefix::Host) => ["__Host-", &self.key[..], "=", &self.value[..], ";"].join(""),
            _ => [&self.key[..], "=", &self.value[..], ";"].join(""),
        };

        match self.expires {
            Some(time) => {
                let dt =
                    system_to_utc(time)
                        .format("%a, %e %b %Y %T GMT")
                        .to_string();

                cookie.reserve_exact(10 + dt.len());
                cookie.push_str(" Expires=");
                cookie.push_str(&dt);
                cookie.push(';');
            }
            _ => { /* Nothing */ }
        }

        match self.max_age {
            Some(age) if age > 0 => {
                let a = age.to_string();

                cookie.reserve_exact(10 + a.len());
                cookie.push_str(" Max-Age=");
                cookie.push_str(&a);
                cookie.push(';');
            }
            _ => { /* Nothing */ }
        }

        if !self.domain.is_empty() {
            cookie.reserve_exact(9 + self.domain.len());

            cookie.push_str(" Domain=");
            cookie.push_str(&self.domain);
            cookie.push(';');
        }

        if !self.path.is_empty() {
            cookie.reserve_exact(7 + self.path.len());

            cookie.push_str(" Path=");
            cookie.push_str(&self.path);
            cookie.push(';');
        }

        if self.secure {
            cookie.push_str(" Secure;");
        }

        if self.http_only {
            cookie.push_str(" HttpOnly;");
        }

        cookie
    }
}

impl Clone for Cookie {
    fn clone(&self) -> Self {
        Cookie {
            key: self.key.clone(),
            value: self.value.clone(),
            key_prefix: self.key_prefix.clone(),
            expires: self.expires.clone(),
            max_age: self.max_age.clone(),
            domain: self.domain.clone(),
            path: self.path.clone(),
            secure: self.secure,
            http_only: self.http_only,
        }
    }
}

fn system_to_utc(t: SystemTime) -> DateTime<Utc> {
    let (sec, n_sec) = match t.duration_since(UNIX_EPOCH) {
        Ok(dur) => (dur.as_secs() as i64, dur.subsec_nanos()),
        Err(e) => {
            let dur = e.duration();
            let (sec, n_sec) = (dur.as_secs() as i64, dur.subsec_nanos());
            if n_sec == 0 {
                (-sec, 0)
            } else {
                (-sec - 1, 1_000_000_000 - n_sec)
            }
        }
    };

    Utc.timestamp(sec, n_sec)
}
