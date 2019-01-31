use std::io::{BufWriter, Write};
use std::net::TcpStream;

use crate::debug::{self, InfoLevel};
use crate::hashbrown::HashMap;

pub trait MapUpdates<T> {
    fn add(&mut self, field: &str, value: T, allow_replace: bool) -> Option<T>;
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
}

pub trait VecExtension {
    fn flat(&self) -> String;
}


impl VecExtension for Vec<String> {
    fn flat(&self) -> String {
        let mut result = String::new();

        for content in self.iter() {
            result.push_str(content);
        }

        result
    }
}

pub fn write_to_buff(buffer: &mut BufWriter<&TcpStream>, content: &[u8]) {
    if let Err(err) = buffer.write(content) {
        debug::print(
            &format!(
                "An error has taken place when writing the response header to the stream: {}",
                err
            ),
            InfoLevel::Warning,
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
            InfoLevel::Warning,
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

            res.push_str(&format!("{}:{}", field, json_format_content(&vec!(content.to_owned()))));
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

fn json_format_content(content: &Vec<String>) -> String {
    let len = content.len();
    match len {
        0 => String::new(),
        1 => content[0].to_owned(),
        _ => {
            let mut base = String::from("[");

            for i in 0..len {
                base.push_str(&content[i]);

                if i != len - 1 {
                    base.push(',');
                }
            }

            base.push(']');
            base
        },
    }
}
