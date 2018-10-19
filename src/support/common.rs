use std::collections::HashMap;

use super::debug;
use std::io::{BufWriter, Write};
use std::net::TcpStream;

pub trait MapUpdates<T> {
    fn add(&mut self, field: &str, value: T, allow_replace: bool) -> Option<T>;
    fn merge(&mut self, other: &HashMap<String, T>);
}

impl<T> MapUpdates<T> for HashMap<String, T> {
    fn add(&mut self, field: &str, value: T, allow_replace: bool) -> Option<T> {
        if field.is_empty() {
            return None;
        }

        let f = field.to_lowercase();
        if allow_replace {
            //new field, insert into the map
            self.insert(f, value)
        } else {
            //existing field, replace existing value or append depending on the parameter
            self.entry(f).or_insert(value);
            None
        }
    }

    fn merge(&mut self, other: &HashMap<String, T>) {

    }
}

pub fn write_to_buff(buffer: &mut BufWriter<&TcpStream>, content: &[u8]) {
    if let Err(err) = buffer.write(content) {
        debug::print(
            &format!(
                "An error has taken place when writing the response header to the stream: {}",
                err
            ),
            1,
        );
    }
}

pub fn flush_buffer(buffer: &mut BufWriter<&TcpStream>) -> u8 {
    if let Err(err) = buffer.flush() {
        debug::print(
            &format!(
                "An error has taken place when flushing the response to the stream: {}",
                err
            )[..],
            1,
        );

        return 1;
    }

    0
}

pub fn json_stringify(contents: &HashMap<String, String>) -> String {
    let mut res: String = String::from("{");
    let mut is_first = true;

    if !contents.is_empty() {
        for (field, content) in contents.iter() {
            if field.is_empty() {
                continue;
            }

            if !is_first {
                res.push(',');
            } else {
                is_first = false;
            }

            res.push_str(&format!("{}:{}", field, json_format_content(content)));
        }
    }

    res.push('}');
    res
}

pub fn json_flat_stringify(contents: &HashMap<String, Vec<String>>) -> String {
    let mut res: String = String::from("{");
    let mut is_first = true;

    if !contents.is_empty() {
        for (field, content) in contents.iter() {
            if field.is_empty() {
                continue;
            }

            //TODO: flag the vec
            if !is_first {
                res.push(',');
            } else {
                is_first = false;
            }


            res.push_str(&format!("{}:{}", field, json_format_content(content)));
        }
    }

    res.push('}');
    res
}

fn json_format_content(content: &String) -> String {
    let len = content.len();
    match len {
        0 => String::new(),
        _ => content.to_owned(),
    }
}
