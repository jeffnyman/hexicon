//! How the VM identifies itself to the programs it runs.
//!
//! Hexicon reports its own name and version, not those of the reference
//! T3 VM (`mjr-T3`, version 3.1.3). Programs see these through
//! `vmId`, `vmVersion`, and `vmBanner`; Hexad shows them in response to
//! a VERSION command.

/// The VM's identifier, as `vmId` returns it.
pub const ID: &str = "hexicon";

/// The VM's version, as text.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The VM's version as `vmVersion` returns it: the major number in the
/// high 16 bits, then the minor number and the patch number in 8 bits
/// each. Libraries take the number apart this way to display it, so the
/// layout is fixed even though the numbering is Hexicon's own.
pub fn version_number() -> i32 {
    let part = |text: &str| text.parse::<i32>().unwrap_or(0);
    (part(env!("CARGO_PKG_VERSION_MAJOR")) << 16)
        | ((part(env!("CARGO_PKG_VERSION_MINOR")) & 0xff) << 8)
        | (part(env!("CARGO_PKG_VERSION_PATCH")) & 0xff)
}

/// The VM's banner, as `vmBanner` returns it.
pub fn banner() -> String {
    format!("Hexicon VM {VERSION}")
}
