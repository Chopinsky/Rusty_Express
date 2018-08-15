#![allow(dead_code)]

use core::router::{Callback, REST};
use regex::Regex;
use std::collections::HashMap;

struct Field {
    name: String,
    validation: Option<Regex>,
}

impl Clone for Field {
    fn clone(&self) -> Self {
        Field {
            name: self.name.clone(),
            validation: self.validation.clone(),
        }
    }
}

struct Node {
    field: Field,
    callback: Option<Callback>,
    named_children: HashMap<String, Box<Node>>,
    params_children: Vec<Box<Node>>,
}

impl Node {
    fn new(field: &str, validation: Option<Regex>, callback: Option<Callback>) -> Self {
        Node {
            field: Field {
                name: field.to_owned(),
                validation,
            },
            callback,
            named_children: HashMap::new(),
            params_children: Vec::new(),
        }
    }

    fn insert(&mut self, mut segments: Vec<String>, callback: Callback) {
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

            self.named_children
                .insert(head.to_owned(), Node::build_new_child(current, segments, callback));

            return;
        }

        self.params_children.push(Node::build_new_child(current, segments, callback));
    }

    fn build_new_child(name: &str, segments: Vec<String>, callback: Callback) -> Box<Node> {
        match segments.len() {
            0 => {
                Box::new(Node::new(name, None, Some(callback)))
            },
            _ => {
                let mut node = Node::new(name, None, None);
                node.insert(segments, callback);
                Box::new(node)
            }
        }
    }
}

impl Clone for Node {
    fn clone(&self) -> Self {
        Node {
            field: self.field.clone(),
            callback: self.callback.clone(),
            named_children: self.named_children.clone(),
            params_children: self.params_children.clone(),
        }
    }
}

//TODO: add params validations

pub struct RouteTrie {
    root: Node,
}

impl RouteTrie {
    pub fn initialize() -> Self {
        RouteTrie {
            root: Node::new("/", None, None),
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
        route_head: &RouteTrie,
        segments: &[String],
        params: &mut Vec<(String, String)>,
    ) -> Option<Callback>
    {
        RouteTrie::recursive_find(&route_head.root, segments, params)
    }

    fn recursive_find (
        root: &Node,
        segments: &[String],
        params: &mut Vec<(String, String)>,
    ) -> Option<Callback>
    {
        if segments.is_empty() {
            return None;
        }

        let head = &segments[0];
        let is_segments_tail = segments.len() <= 1;

        if let Some(child) = root.named_children.get(head) {
            if is_segments_tail {
                return child.callback;
            }

            return RouteTrie::recursive_find(&child, &segments[1..], params);
        }

        for param_node in root.params_children.iter() {
            if param_node.field.name.is_empty() {
                continue;
            }

            if let Some(ref reg) = param_node.field.validation {
                if !reg.is_match(head) {
                    continue;
                }
            }

            params.push((param_node.field.name.to_owned(), head.to_owned()));

            if is_segments_tail {
                if let Some(callback) = param_node.callback {
                    return Some(callback);
                }
            } else {
                if let Some(callback) = RouteTrie::recursive_find(param_node, &segments[1..], params) {
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
