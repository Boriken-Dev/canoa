//! The profile items that ship with the host as scripts: who the session's programs say they are, and the network its values ask
//! for (the proxy itself stays the host's). They are providers of the `profile_item` kind in a library of their own, written with
//! the Canoa SDK and registered the way an extension's library is, so the host treats them as it treats any item an extension adds.

use coqui_acp_api::guatiao::library::Registry;

use crate::scripts::Bundle;

/// The libraries of the items that ship in the host, with the SDK modules they import.
static BUNDLE: Bundle = Bundle {
    name: "profile-items",
    libraries: &["profile_items"],
    files: &[
        (
            "profile_items/items.js",
            include_str!("../../extensions/profile_items/items.js"),
        ),
        (
            "profile_items/package.json",
            include_str!("../../extensions/profile_items/package.json"),
        ),
        (
            "profile_items/canoa_profile_items.guatiao",
            include_str!("../../extensions/profile_items/canoa_profile_items.guatiao"),
        ),
        (
            "profile_items/sdk/profile.js",
            include_str!("../../extensions/sdk/profile.js"),
        ),
        (
            "profile_items/sdk/kinds.js",
            include_str!("../../extensions/sdk/kinds.js"),
        ),
        (
            "profile_items/sdk/model.js",
            include_str!("../../extensions/sdk/model.js"),
        ),
    ],
};

/// A registry of the items that ship in the host as scripts, read by the source of JavaScript built into the program.
///
/// # Errors
///
/// Why they cannot be offered: their files, or the source.
pub(super) fn registry() -> Result<Registry, String> {
    BUNDLE.registry(&crate::scripts::root())
}
