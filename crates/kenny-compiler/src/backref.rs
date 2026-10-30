//! Which methods own a `$~` of their own. In CRuby `$~` (and `$1`, `$'` ...) is local to the
//! method frame: a match inside a called method doesn't change the caller's, and a method
//! starts with `$~` nil. Kenny keeps one `$~` per thread, so a method that may set it (or reads
//! it) saves the caller's on entry, starts from nil, and puts it back on the way out
//! (`kn_match_save` / `kn_match_restore`). Methods that don't touch it pay nothing; the test
//! is by the calls' names (and, for the ambiguous ones like `[]`, a regexp-like argument),
//! so a method matching only through `send` or a regexp held in a plain local isn't covered.

use ruby_prism::*;

/// Whether the method body (its blocks included, nested defs and classes not) may set or
/// reads the last match.
pub(crate) fn uses_backref(nodes: &[&Node]) -> bool {
    let mut v = V { found: false };
    for n in nodes {
        v.visit(n);
        if v.found {
            return true;
        }
    }
    false
}

struct V {
    found: bool,
}

/// A regexp literal, or what is likely one: an instance variable or an ALL_CAPS constant.
fn regexpish(n: &Node, loose: bool) -> bool {
    if n.as_regular_expression_node().is_some() || n.as_interpolated_regular_expression_node().is_some() {
        return true;
    }
    let caps = |name: &[u8]| name.iter().any(|c| c.is_ascii_uppercase()) && !name.iter().any(|c| c.is_ascii_lowercase());
    if let Some(c) = n.as_constant_read_node() {
        return caps(c.name().as_slice());
    }
    if let Some(c) = n.as_constant_path_node() {
        return c.name().is_some_and(|x| caps(x.as_slice()));
    }
    loose && (n.as_instance_variable_read_node().is_some() || n.as_class_variable_read_node().is_some())
}

impl<'pr> Visit<'pr> for V {
    fn visit_def_node(&mut self, _: &DefNode<'pr>) {}
    fn visit_class_node(&mut self, _: &ClassNode<'pr>) {}
    fn visit_module_node(&mut self, _: &ModuleNode<'pr>) {}
    fn visit_singleton_class_node(&mut self, _: &SingletonClassNode<'pr>) {}
    fn visit_back_reference_read_node(&mut self, _: &BackReferenceReadNode<'pr>) {
        self.found = true;
    }
    fn visit_numbered_reference_read_node(&mut self, _: &NumberedReferenceReadNode<'pr>) {
        self.found = true;
    }
    fn visit_match_write_node(&mut self, _: &MatchWriteNode<'pr>) {
        self.found = true;
    }
    fn visit_match_last_line_node(&mut self, _: &MatchLastLineNode<'pr>) {
        self.found = true;
    }
    fn visit_global_variable_read_node(&mut self, node: &GlobalVariableReadNode<'pr>) {
        if node.name().as_slice() == b"$~" {
            self.found = true;
        }
    }
    fn visit_global_variable_write_node(&mut self, node: &GlobalVariableWriteNode<'pr>) {
        if node.name().as_slice() == b"$~" {
            self.found = true;
        }
        visit_global_variable_write_node(self, node);
    }
    fn visit_when_node(&mut self, node: &WhenNode<'pr>) {
        if node.conditions().iter().any(|c| regexpish(&c, true)) {
            self.found = true;
            return;
        }
        visit_when_node(self, node);
    }
    fn visit_call_node(&mut self, node: &CallNode<'pr>) {
        if self.found {
            return;
        }
        let args: Vec<Node> = node.arguments().map(|a| a.arguments().iter().collect()).unwrap_or_default();
        let any_arg = |loose: bool| args.iter().any(|a| regexpish(a, loose));
        let hit = match node.name().as_slice() {
            b"=~" | b"!~" | b"match" | b"sub" | b"gsub" | b"sub!" | b"gsub!" | b"scan" | b"last_match" => true,
            b"===" => node.receiver().is_some_and(|r| regexpish(&r, true)),
            b"split" | b"start_with?" | b"index" | b"rindex" | b"byteindex" | b"byterindex" | b"slice" | b"slice!" | b"partition" | b"rpartition" => any_arg(true),
            b"[]" => any_arg(false),
            _ => false,
        };
        if hit {
            self.found = true;
            return;
        }
        visit_call_node(self, node);
    }
}
