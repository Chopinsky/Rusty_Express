use std::io::{BufWriter, Write};
use std::ptr;
use std::sync::atomic;

use crate::core::stream::Stream;
use crate::hashbrown::HashMap;
use crate::support::debug::{self, InfoLevel};

pub trait MapUpdates<T> {
    fn add(&mut self, field: &str, value: T, allow_replace: bool, allow_case: bool) -> Option<T>;
}

impl<T> MapUpdates<T> for HashMap<String, T> {
    fn add(&mut self, key: &str, value: T, enable_replace: bool, normalize_key: bool) -> Option<T> {
        if key.is_empty() {
            return None;
        }

        let f = if !normalize_key {
            key.to_lowercase()
        } else {
            key.to_owned()
        };

        if enable_replace {
            //new field, insert into the map
            self.insert(f, value)
        } else {
            //existing field, replace existing value or append depending on the parameter
            self.entry(f).or_insert(value);
            None
        }
    }
}

pub trait VecExt<T> {
    fn swap_reset(&mut self) -> Vec<T>;
    fn swap_reserve(&mut self, cap: usize) -> Vec<T>;
}

impl<T> VecExt<T> for Vec<T> {
    /// Swapping the `Vec<T>` pointer out so it can be used elsewhere, without moving the ownership
    /// of self. The implementation effectively exchange the ownership of the underlying vector, such
    /// that we don't have to move the ownership of the container variable.
    fn swap_reset(&mut self) -> Vec<T> {
        // create the new vector to hold new content
        let mut next = Vec::new();

        // swap the content of the `Vec<T>` struct instead of its underlying vec data
        swap_vec_ptr(self, &mut next);

        // now `next` holds the content originally pointed to by `self`
        next
    }

    fn swap_reserve(&mut self, cap: usize) -> Vec<T> {
        // create the new vector with a predetermined capacity to hold new content
        let mut next = Vec::with_capacity(cap);

        // swap the content of the `Vec<T>` struct instead of its underlying vec data
        swap_vec_ptr(self, &mut next);

        // now `next` holds the content originally pointed to by `self`
        next
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

impl LineBreakUtil for Vec<u8> {
    fn append_line_break(&mut self) {
        if self.capacity() - self.len() < 2 {
            self.reserve(2);
        }

        self.push(13);
        self.push(10);
    }
}

pub(crate) fn write_to_buff(buffer: &mut BufWriter<&mut Stream>, content: &[u8]) {
    if buffer.write(content).is_err() {
        debug::print(
            "An error has taken place when writing the response header to the stream",
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

fn swap_vec_ptr<T>(src: &mut Vec<T>, tgt: &mut Vec<T>) {
    // obtain the raw pointers
    let p: *mut Vec<T> = src;
    let pn: *mut Vec<T> = tgt;

    // swap the pointers: this is essentially the impl from ptr::swap_nonoverlapping for struct
    // size smaller than 32. Since Vec<T> has a constant size of 24, we don't need to check the
    // mem size again and again as it's a known branch to execute.
    unsafe {
        let temp = ptr::read(p);
        ptr::copy_nonoverlapping(pn, p, 1);
        ptr::write(pn, temp);
    }
}

#[cfg(test)]
mod route_test {
    use super::VecExt;

    #[test]
    fn vec_swap_reset() {
        let mut src = vec![1, 2, 3, 4, 5];
        let tgt = src.swap_reset();
        src.extend_from_slice(&[2, 3, 4, 5, 6]);

        assert_eq!(src, vec![2, 3, 4, 5, 6]);
        assert_eq!(tgt, vec![1, 2, 3, 4, 5]);
    }
}
