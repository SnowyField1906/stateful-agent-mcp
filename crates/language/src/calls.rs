//! Calls: what a call expression calls and how it was written.

use tree_sitter::Node;

/// How a call was written, which decides the definitions it can reach.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CallKind {
    /// Nothing is known about the target beyond its name.
    Any,
    /// `foo()`: a function that is not a method.
    Free,
    /// A method of the enclosing type: `self.foo()`, `Self::foo()`.
    Own,
    /// A method on a receiver whose type the language adapter inferred.
    MethodOf(String),
    /// A method on a receiver typed as a trait object or `impl Trait`: the trait's declaration
    /// and every implementation of it.
    Dyn(String),
    /// `Type::foo()` or `module::foo()`: the last segment before the name.
    Path(String),
    /// `value.foo()` where the type of `value` is unknown: possible method targets.
    Method,
}

/// A call to a function by name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallSite {
    pub name: String,
    pub kind: CallKind,
    /// The first segment of the path or dotted chain the target was written with: `std` for
    /// `std::fs::read(..)`, `fs` for `fs::read(..)`, `os` for `os.path.join(..)`. `None` for a
    /// bare name or a method on a value.
    pub root: Option<String>,
}

/// The calls of a language.
pub trait Calls {
    /// Whether a type can have methods that none of its own definitions holds: methods inherited
    /// from a base class. A call on such a type that finds no method among those of the type
    /// proves nothing about where it goes. The default is `true`, so that a language that does not
    /// say is never taken for one without inheritance; Rust, whose types get methods from `impl`
    /// blocks and traits only, says `false`.
    fn types_inherit_methods(&self) -> bool {
        true
    }

    /// The name a call expression calls (`foo` for `foo(1)`, `bar` for `x.bar()`), or `None`
    /// when `node` is not a call or its target is not a plain name.
    fn call_name(&self, node: Node<'_>, source: &str) -> Option<String>;

    /// The call that `node` makes with the way it was written. Calls that are not calls of
    /// functions, such as tuple-struct constructors, give `None`. The default knows only the name.
    fn call_site(&self, node: Node<'_>, source: &str) -> Option<CallSite> {
        self.call_name(node, source).map(|name| CallSite {
            name,
            kind: CallKind::Any,
            root: None,
        })
    }
}
