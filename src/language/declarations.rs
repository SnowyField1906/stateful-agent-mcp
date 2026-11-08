//! Declarations: what a language defines, and which code is tests.

use std::fmt;
use std::path::Path;

use tree_sitter::Node;

/// One kind of definition (`fn`, `struct`, `class`) and what the tools make of the definitions of
/// that kind.
///
/// A language lists its kinds once, in [`Declarations::kinds`], and a [`Definition`] points at its
/// row, so the tools read these properties instead of matching on the name of a kind. A row is
/// built in a `const` with the methods below, which set the field of the same name:
/// `Kind::new("trait").declares_type().owner().interface()`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Kind {
    /// Short name shown in listings and accepted by the `kinds` filter.
    pub name: &'static str,
    /// Other words that name the kind in a request: `function` for `fn`.
    pub aliases: &'static [&'static str],
    /// The definition has a body that can call other code.
    pub function: bool,
    /// The definition declares a type or a trait that other code can name as the type of a value.
    pub declares_type: bool,
    /// The body of the definition holds methods: an `impl`, a trait, a class.
    pub owner: bool,
    /// Other types implement the definition: a trait. A method called through it can reach every
    /// implementation.
    pub interface: bool,
    /// The definition is a module that a path can name: an inline `mod`.
    pub module: bool,
    /// Listed only when a request names the kind: enum variants, fields.
    pub on_request: bool,
}

impl Kind {
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            aliases: &[],
            function: false,
            declares_type: false,
            owner: false,
            interface: false,
            module: false,
            on_request: false,
        }
    }

    pub const fn aka(mut self, aliases: &'static [&'static str]) -> Self {
        self.aliases = aliases;
        self
    }

    pub const fn function(mut self) -> Self {
        self.function = true;
        self
    }

    pub const fn declares_type(mut self) -> Self {
        self.declares_type = true;
        self
    }

    pub const fn owner(mut self) -> Self {
        self.owner = true;
        self
    }

    pub const fn interface(mut self) -> Self {
        self.interface = true;
        self
    }

    pub const fn module(mut self) -> Self {
        self.module = true;
        self
    }

    pub const fn on_request(mut self) -> Self {
        self.on_request = true;
        self
    }

    /// Whether `word` names this kind: its name or one of its aliases, ignoring case.
    pub fn is_named(&self, word: &str) -> bool {
        self.name.eq_ignore_ascii_case(word)
            || self
                .aliases
                .iter()
                .any(|alias| alias.eq_ignore_ascii_case(word))
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name)
    }
}

/// A definition found in a syntax tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Definition {
    pub kind: &'static Kind,
    pub name: String,
    /// For an `impl Trait for Type` block, the trait (`name` is then the type).
    pub trait_name: Option<String>,
    /// The body of the definition is test code: `mod tests`, a `Test*` class.
    pub test: bool,
}

impl Definition {
    pub fn new(kind: &'static Kind, name: impl Into<String>) -> Self {
        Self {
            kind,
            name: name.into(),
            trait_name: None,
            test: false,
        }
    }

    /// The definition implements the trait `trait_name`, if there is one.
    pub fn implementing(mut self, trait_name: Option<String>) -> Self {
        self.trait_name = trait_name;
        self
    }

    /// Whether the body of the definition is test code.
    pub fn holding_tests(mut self, test: bool) -> Self {
        self.test = test;
        self
    }
}

/// What a language defines.
pub trait Declarations {
    /// Every kind of definition the language has, in the order they are named in messages: the
    /// kinds that are always listed first, then the ones listed [`on request`](Kind::on_request).
    fn kinds(&self) -> &'static [&'static Kind];

    /// The definition that `node` introduces, if it introduces one.
    fn definition(&self, node: Node<'_>, source: &str) -> Option<Definition>;

    /// The fields of a definition node that hold the name it introduces. Python also introduces a
    /// name on the `left` of an assignment.
    fn name_fields(&self) -> &'static [&'static str] {
        &["name"]
    }

    /// Whether `name` is written like the name of a type. The default is the convention of most
    /// languages: it starts with a capital.
    fn is_type_name(&self, name: &str) -> bool {
        is_capitalized(name)
    }

    /// Whether the file at `path` is test code. The default knows `tests` and `test` folders and
    /// the file names `tests`, `test_*` and `*_test`; a language with other conventions
    /// overrides it, usually next to [`is_conventional_test_path`].
    fn is_test_path(&self, path: &str) -> bool {
        is_conventional_test_path(path)
    }
}

/// Whether `name` starts with a capital: the common way to write the name of a type.
pub fn is_capitalized(name: &str) -> bool {
    name.starts_with(char::is_uppercase)
}

/// The test file conventions shared by the languages: a `tests` or `test` folder, or a file named
/// `tests`, `test_*` or `*_test`.
pub fn is_conventional_test_path(path: &str) -> bool {
    let path = Path::new(path);
    let in_tests_dir = path
        .components()
        .any(|c| matches!(c.as_os_str().to_str(), Some("tests" | "test")));
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    in_tests_dir || stem == "tests" || stem.starts_with("test_") || stem.ends_with("_test")
}
