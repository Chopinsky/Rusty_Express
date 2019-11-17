use crate::core::router::RouteHandler;
use crate::hashbrown::HashMap;
use crate::regex::Regex;

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
            validation,
        }
    }
}

impl PartialEq for Field {
    fn eq(&self, other: &Field) -> bool {
        if self.name != other.name
            || self.is_param != other.is_param
            || self.validation.is_some() != other.validation.is_some()
        {
            return false;
        }

        if let Some(ref reg_one) = self.validation {
            if let Some(ref reg_two) = other.validation {
                return reg_one.to_string() == reg_two.to_string();
            }
        }

        true
    }
}

struct Node {
    field: Field,
    handler: RouteHandler,
    //    callback: Option<Callback>,
    //    location: Option<PathBuf>,
    named_children: HashMap<String, Node>,
    params_children: Vec<Node>,
}

impl Node {
    fn new(field: Field, handler: RouteHandler) -> Self {
        Node {
            field,
            handler,
            //            callback,
            //            location,
            named_children: HashMap::new(),
            params_children: Vec::new(),
        }
    }

    fn insert(&mut self, mut segments: Vec<Field>, handler: RouteHandler) {
        debug_assert!(handler.is_some());

        let head = match segments.pop() {
            Some(seg) => seg,
            None => return,
        };

        // if already has this child, keep calling insert recursively. Only do this when not
        // a params, otherwise, always create a new branch
        if !head.is_param {
            if let Some(child) = self.named_children.get_mut(&head.name) {
                if segments.is_empty() {
                    // done, update the node
                    if child.handler.is_some() {
                        panic!("Key collision!");
                    }

                    child.handler = handler;
                } else {
                    // recursive insert to the child
                    child.insert(segments, handler);
                }

                return;
            }

            self.named_children.insert(
                head.name.clone(),
                Node::build_new_child(head, segments, handler),
            );

            return;
        }

        self.params_children
            .push(Node::build_new_child(head, segments, handler));
    }

    fn build_new_child(field: Field, segments: Vec<Field>, handler: RouteHandler) -> Node {
        match segments.len() {
            0 => {
                // leaf node
                Node::new(field, handler)
            }
            _ => {
                // branch node
                let mut node = Node::new(field, RouteHandler::default());
                node.insert(segments, handler);
                node
            }
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
                RouteHandler::default(),
            ),
        }
    }

    #[inline]
    pub(crate) fn is_empty(&self) -> bool {
        self.root.named_children.is_empty() && self.root.params_children.is_empty()
    }

    #[inline]
    pub(crate) fn add(&mut self, segments: Vec<Field>, handler: RouteHandler) {
        self.root.insert(segments, handler);
    }

    pub(crate) fn find(
        route_head: &RouteTrie,
        segments: &[String],
        params: &mut HashMap<String, String>,
    ) -> RouteHandler {
        RouteTrie::recursive_find(&route_head.root, segments, params)
    }

    fn recursive_find(
        root: &Node,
        segments: &[String],
        params: &mut HashMap<String, String>,
    ) -> RouteHandler {
        if segments.is_empty() {
            return RouteHandler::default();
        }

        let head = &segments[0];
        let is_tail = segments.len() <= 1;

        if let Some(child) = root.named_children.get(head) {
            if is_tail {
                return child.handler.clone();
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

            let result = if is_tail {
                //                RouteHandler::new(param_node.callback, param_node.location.clone())
                param_node.handler.clone()
            } else {
                RouteTrie::recursive_find(param_node, &segments[1..], params)
            };

            if result.is_some() {
                params
                    .entry(param_node.field.name.clone())
                    .or_insert(head.to_owned());
                return result;
            }
        }

        RouteHandler::default()
    }
}
