//! Bindings of each scope's tools to [`State`](super::State): one file per scope, each a set of
//! `State` methods that call the scope's tool handlers. Scopes know nothing of the server; this
//! is where they meet it.

mod filesystem;
mod langtool;
mod workflow;
