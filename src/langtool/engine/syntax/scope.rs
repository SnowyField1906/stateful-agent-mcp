//! The definitions that enclose a symbol, a call or a reference.
//!
//! One value serves symbols, references and the call graph: it keeps the kind of every enclosing
//! definition for the logic (is this a method, is this test code) and renders as the qualified
//! path that every tool accepts back as a symbol (`east::Token`).

use std::fmt;

use super::spec::Definition;

/// A definition that encloses other code: `mod east`, `impl Token<'a>`, `class A`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub kind: &'static str,
    pub name: String,
    /// The trait an `impl` block implements: `Syntax` for `impl Syntax for Token`.
    pub trait_name: Option<String>,
}

impl Frame {
    pub fn new(kind: &'static str, name: &str) -> Self {
        Self {
            kind,
            name: name.to_string(),
            trait_name: None,
        }
    }

    /// The frame that the definition `def` opens.
    pub fn of(def: &Definition) -> Self {
        Self {
            kind: def.kind,
            name: def.name.clone(),
            trait_name: def.trait_name.clone(),
        }
    }

    /// Whether a method in this frame can be called through a receiver typed as the trait
    /// `name`: the trait itself and every `impl` of it.
    pub fn implements(&self, name: &str) -> bool {
        (self.kind == "trait" && self.bare_name() == name)
            || self.trait_name.as_deref() == Some(name)
    }

    /// The name without generic parameters: `Token` for `Token<'a>`.
    pub fn bare_name(&self) -> &str {
        self.name.split('<').next().unwrap_or(&self.name).trim()
    }

    /// Whether the frame is a type or trait whose body holds methods.
    pub fn is_owner(&self) -> bool {
        matches!(self.kind, "impl" | "trait" | "class")
    }

    /// Whether the frame holds test code: `mod tests`, `mod test` or a `Test*` class.
    pub fn is_test(&self) -> bool {
        is_test_scope(self.kind, &self.name)
    }
}

/// Whether a definition of `kind` named `name` holds test code: `mod tests`, `mod test` or a
/// `Test*` class.
pub fn is_test_scope(kind: &str, name: &str) -> bool {
    (kind == "mod" && matches!(name, "tests" | "test"))
        || (kind == "class" && name.starts_with("Test"))
}

/// The enclosing definitions of a position, outermost first. Empty at the top level.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Lineage(Vec<Frame>);

impl Lineage {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Whether `prefix` is this lineage or its outer part: `a::b::c` starts with `a::b`.
    pub fn starts_with(&self, prefix: &Lineage) -> bool {
        self.0.starts_with(&prefix.0)
    }

    pub fn frames(&self) -> &[Frame] {
        &self.0
    }

    /// The names of the enclosing definitions, outermost first.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(|frame| frame.name.as_str())
    }

    /// The innermost enclosing definition.
    pub fn innermost(&self) -> Option<&Frame> {
        self.0.last()
    }

    /// The type or trait this position is a member of: the innermost frame when it is an owner.
    pub fn owner(&self) -> Option<&Frame> {
        self.innermost().filter(|frame| frame.is_owner())
    }

    /// Whether any enclosing definition holds test code.
    pub fn in_test(&self) -> bool {
        self.0.iter().any(Frame::is_test)
    }

    pub fn push(&mut self, frame: Frame) {
        self.0.push(frame);
    }

    pub fn pop(&mut self) {
        self.0.pop();
    }

    /// This lineage inside one more definition.
    pub fn entered(&self, frame: Frame) -> Self {
        let mut inner = self.clone();
        inner.push(frame);
        inner
    }
}

/// The qualified path, `east::Token::parse`: the names of the frames joined by `::`. The
/// selector syntax accepts it for every language.
impl fmt::Display for Lineage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, name) in self.names().enumerate() {
            if i > 0 {
                f.write_str("::")?;
            }
            f.write_str(name)?;
        }
        Ok(())
    }
}
