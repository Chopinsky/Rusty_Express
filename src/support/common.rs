use std::io::{BufWriter, Write};
use std::sync::atomic;

use crate::core::stream::Stream;
use crate::hashbrown::HashMap;
use crate::support::debug::{self, InfoLevel};

pub trait MapUpdates<T> {
    fn add(&mut self, field: &str, value: T, allow_replace: bool, allow_case: bool) -> Option<T>;
}

impl<T> MapUpdates<T> for HashMap<String, T> {
    fn add(&mut self, field: &str, value: T, allow_replace: bool, allow_case: bool) -> Option<T> {
        if field.is_empty() {
            return None;
        }

        let f = if !allow_case {
            field.to_lowercase()
        } else {
            field.to_owned()
        };

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

pub trait LineBreakUtil {
    fn append_line_break(&mut self);
}

impl LineBreakUtil for String {
    fn append_line_break(&mut self) {
        if self.capacity() - self.len() < 2 {
            self.reserve_exact(2);
        }

        self.push('\r');
        self.push('\n');
    }
}

pub(crate) fn write_to_buff(buffer: &mut BufWriter<&mut Stream>, content: &[u8]) {
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

pub(crate) fn write_line_break(buffer: &mut BufWriter<&mut Stream>) {
    let _ = buffer.write(&[13, 10]);
}

pub(crate) fn flush_buffer(buffer: &mut BufWriter<&mut Stream>) -> u8 {
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

#[inline(always)]
pub(crate) fn cpu_relax(count: usize) {
    for _ in 0..(1 << count) {
        atomic::spin_loop_hint()
    }
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

            res.push_str(field);
            res.push_str(":");
            res.push_str(content);
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

            res.push_str(&field);
            res.push_str(":");
            res.push_str(&json_format_content(content.as_slice()));
        }
    }

    res.push('}');
    res
}

fn json_format_content(content: &[String]) -> String {
    let len = content.len();
    match len {
        0 => String::new(),
        1 => content[0].to_owned(),
        _ => {
            let mut base = String::from("[");

            (0..len).for_each(|idx| {
                base.push_str(&content[idx]);

                if idx != len - 1 {
                    base.push(',');
                }
            });

            base.push(']');
            base
        }
    }
}
