#![allow(dead_code)]

use std::cell::RefCell;
use std::collections::HashMap;
use core::router::Callback;

pub struct TrieNode {
    field: String,
    is_param_field: bool,
    callback: Option<Callback>,
    children: HashMap<String, RefCell<TrieNode>>,
}

impl TrieNode {
    fn new(field: &str, callback: Option<Callback>, is_param_field: bool) -> Self {
        TrieNode {
            field: field.to_owned(),
            is_param_field,
            callback,
            children: HashMap::new(),
        }
    }

    pub fn insert(&mut self, mut segments: Vec<String>, callback: Callback) {
        if segments.is_empty() { return; }

        let head = segments.remove(0);
        let (current, is_param) = match head.starts_with(':') {
            true => (&head[1..], true),
            false => (&head[..], false),
        };

        // if already has this child, keep calling insert recursively. Only do this when not
        // a params, otherwise, always create a new branch
        if !is_param {
            if let Some(child) = self.children.get(current) {
                match segments.len() {
                    0 => (*child.borrow_mut()).callback = Some(callback),
                    _ => (*child.borrow_mut()).insert(segments, callback),
                }

                return;
            }
        }

        let new_child = match segments.len() {
            0 => {
                TrieNode::new(current, Some(callback), is_param)
            },
            _ => {
                let mut node = TrieNode::new(current, None, is_param);
                node.insert(segments, callback);
                node
            },
        };

        self.children.insert(current.to_owned(), RefCell::new(new_child));
    }

    //TODO: comprise to radix-trie?
}

pub struct RouteTrie {
    root: TrieNode
}

impl RouteTrie {
    #[inline]
    pub fn initialize() -> Self {
        RouteTrie {
            root: TrieNode::new("/", None, false),
        }
    }
}