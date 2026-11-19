//! What the tests of the C ABI share.
#![allow(dead_code, reason = "each test uses part of it")]

/// Keeps every credential the host stores in its memory (`CANOA_CREDENTIAL_STORE`) and out of the operating system's store, which
/// nothing here is about. Done once, before the first host starts.
pub fn memory_credentials() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        // SAFETY: std's own readers of the environment take the lock this takes; nothing else here reads it from C.
        unsafe { std::env::set_var("CANOA_CREDENTIAL_STORE", "memory") };
    });
}
