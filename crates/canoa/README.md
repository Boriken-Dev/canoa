# canoa

The protocol-neutral Rust registry and authorization primitives for Canoa.

Status: early scaffold. `Registry` is in-memory and the `verified` flag is supplied by a trusted caller. Neither a network handshake nor operating-system principal validation is implemented yet. Do not treat this crate alone as a security boundary.
