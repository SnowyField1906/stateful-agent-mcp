//! Hot-reloading stdio proxy for `stateful-agent`: binary fingerprinting and staging, the JSON-RPC
//! router, schema diffing and the worker process the router supervises. `src/main.rs` only wires
//! them to stdin and stdout.

pub mod binary;
pub mod gateway;
pub mod router;
pub mod schema;
pub mod worker;
