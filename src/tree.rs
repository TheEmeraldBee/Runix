/// Labeled entry in a display tree
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Node {
    pub label: String,
    pub children: Vec<Node>,
}

impl Node {
    /// Returns a node with no children
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            children: Vec::new(),
        }
    }

    pub fn push(&mut self, child: Node) {
        self.children.push(child);
    }

    /// Returns the tree as text
    pub fn render(&self) -> String {
        let mut out = format!("{}\n", self.label);
        render_children(&self.children, "", &mut out);
        out
    }
}

fn render_children(children: &[Node], prefix: &str, out: &mut String) {
    for (i, child) in children.iter().enumerate() {
        let last = i + 1 == children.len();
        let (branch, next) = if last {
            ("└── ", "    ")
        } else {
            ("├── ", "│   ")
        };
        out.push_str(&format!("{prefix}{branch}{}\n", child.label));
        render_children(&child.children, &format!("{prefix}{next}"), out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_nested_tree() {
        let mut root = Node::new("plan");
        let mut pacman = Node::new("pacman");
        pacman.push(Node::new("+ fish"));
        pacman.push(Node::new("- nano"));
        root.push(pacman);
        root.push(Node::new("files"));
        assert_eq!(
            root.render(),
            "plan\n├── pacman\n│   ├── + fish\n│   └── - nano\n└── files\n"
        );
    }
}
