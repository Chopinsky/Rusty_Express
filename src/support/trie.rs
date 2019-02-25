#![allow(dead_code)]

use crate::hashbrown::HashMap;
use crate::regex::Regex;
use crate::core::router::Callback;

#[derive(Debug)]
pub(crate) struct Field {
    name: String,
    is_param: bool,
    validation: Option<Regex>,
}

impl Field {
    pub(crate) fn new(name: String, is_param: bool, validation: Option<Regex>) -> Self {
        Field {
            name,
            is_param,
            validation
        }
    }
}

impl Clone for Field {
    fn clone(&self) -> Self {
        Field {
            name: self.name.clone(),
            is_param: self.is_param,
            validation: self.validation.clone(),
        }
    }
}

impl PartialEq for Field {
    fn eq(&self, other: &Field) -> bool {
        if self.name != other.name
            || self.is_param != other.is_param
            || self.validation.is_some() != other.validation.is_some()
        {
            false
        } else {
            if let Some(ref reg_one) = self.validation {
                if let Some(ref reg_two) = other.validation {
                    return reg_one.to_string() == reg_two.to_string()
                }
            }

            true
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
    fn new(field: Field, callback: Option<Callback>) -> Self {
        Node {
            field,
            callback,
            named_children: HashMap::new(),
            params_children: Vec::new(),
        }
    }

    fn insert(&mut self, mut segments: Vec<Field>, callback: Callback) {
        if segments.is_empty() {
            return;
        }

        let head = segments.remove(0);

        // if already has this child, keep calling insert recursively. Only do this when not
        // a params, otherwise, always create a new branch
        if !head.is_param {
            if let Some(child) = self.named_children.get_mut(&head.name) {
                match segments.len() {
                    0 => {
                        if child.callback.is_some() {
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
                .insert(head.name.clone(), Node::build_new_child(head, segments, callback));

            return;
        }

        self.params_children.push(Node::build_new_child(head, segments, callback));
    }

    fn build_new_child(field: Field, segments: Vec<Field>, callback: Callback) -> Box<Node> {
        match segments.len() {
            0 => {
                Box::new(Node::new(field, Some(callback)))
            },
            _ => {
                let mut node = Node::new(field, None);
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
            callback: self.callback,
            named_children: self.named_children.clone(),
            params_children: self.params_children.clone(),
        }
    }
}

pub(crate) struct RouteTrie {
    root: Node,
}

impl RouteTrie {
    pub(crate) fn initialize() -> Self {
        RouteTrie {
            root: Node::new(
                Field::new(String::from("/"), false, None),
                None
            ),
        }
    }

    #[inline]
    pub(crate) fn is_empty(&self) -> bool {
        self.root.named_children.is_empty() && self.root.params_children.is_empty()
    }

    pub(crate) fn add(&mut self, segments: Vec<Field>, callback: Callback) {
        self.root.insert(segments, callback);
    }

    pub(crate) fn find(
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

            params.push((param_node.field.name.clone(), head.clone()));

            if is_segments_tail {
                return param_node.callback;
            }

            if let Some(cb) = RouteTrie::recursive_find(param_node, &segments[1..], params) {
                return Some(cb);
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
