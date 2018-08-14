#![allow(dead_code)]

use core::router::Callback;
use std::collections::HashMap;

pub struct TrieNode {
    field: String,
    callback: Option<Callback>,
    named_children: HashMap<String, Box<TrieNode>>,
    params_children: Vec<Box<TrieNode>>,
}

impl TrieNode {
    fn new(field: &str, callback: Option<Callback>) -> Self {
        TrieNode {
            field: field.to_owned(),
            callback,
            named_children: HashMap::new(),
            params_children: Vec::new(),
        }
    }

    pub fn insert(&mut self, mut segments: Vec<String>, callback: Callback) {
        if segments.is_empty() {
            return;
        }

        let head = segments.remove(0);
        let (current, is_param) = match head.starts_with(':') {
            true => (&head[1..], true),
            false => (&head[..], false),
        };

        // if already has this child, keep calling insert recursively. Only do this when not
        // a params, otherwise, always create a new branch
        if !is_param {
            if let Some(child) = self.named_children.get_mut(current) {
                match segments.len() {
                    0 => {
                        if let Some(_) = child.callback {
                            panic!("Key collision!");
                        }

                        child.callback = Some(callback);
                    }
                    _ => {
                        child.insert(segments, callback);
                    }
                }

                return;
            }

            let new_child = match segments.len() {
                0 => TrieNode::new(current, Some(callback)),
                _ => {
                    let mut temp = TrieNode::new(current, None);
                    temp.insert(segments, callback);
                    temp
                }
            };

            self.named_children
                .insert(head.to_owned(), Box::new(new_child));

            return;
        }

        let new_child = match segments.len() {
            0 => TrieNode::new(current, Some(callback)),
            _ => {
                let mut node = TrieNode::new(current, None);
                node.insert(segments, callback);
                node
            }
        };

        self.params_children.push(Box::new(new_child));
    }
}

impl Clone for TrieNode {
    fn clone(&self) -> Self {
        TrieNode {
            field: self.field.clone(),
            callback: self.callback.clone(),
            named_children: self.named_children.clone(),
            params_children: self.params_children.clone(),
        }
    }
}

//TODO: add params validations

pub struct RouteTrie {
    pub root: TrieNode,
}

impl RouteTrie {
    pub fn initialize() -> Self {
        RouteTrie {
            root: TrieNode::new("/", None),
        }
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.root.named_children.is_empty() && self.root.params_children.is_empty()
    }

    pub fn add(&mut self, segments: Vec<String>, callback: Callback) {
        self.root.insert(segments, callback);
    }

    pub fn find(
        root: &TrieNode,
        segments: &[String],
        params: &mut Vec<(String, String)>,
    ) -> Option<Callback> {

        if segments.is_empty() {
            return None;
        }

        let head = &segments[0];
        let is_segments_tail = segments.len() <= 1;

        if let Some(child) = root.named_children.get(head) {
            if is_segments_tail {
                return child.callback;
            }

            return RouteTrie::find(&child, &segments[1..], params);
        }

        for param_node in root.params_children.iter() {
            if param_node.field.is_empty() {
                continue;
            }

            params.push((param_node.field.to_owned(), head.to_owned()));

            if is_segments_tail {
                if let Some(callback) = param_node.callback {
                    return Some(callback);
                }
            } else {
                if let Some(callback) = RouteTrie::find(param_node, &segments[1..], params) {
                    return Some(callback);
                }
            }

            params.pop();
        }

        None
    }
}

impl Clone for RouteTrie {
    fn clone(&self) -> Self {
        RouteTrie {
            root: self.root.clone(),
        }
    }
}
