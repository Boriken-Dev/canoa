//! The names this launcher carries, from the brand file it was built with (`BRAND_FILE`, else the copy `canoa` holds). It cannot
//! depend on `canoa`, so it reads the same file in its build script, and each name must match what the host and the app take from
//! theirs.

/// What the product is called.
pub const NAME: &str = env!("CANOA_BRAND_NAME");
/// The command line's name: what this launcher's messages are prefixed with.
pub const PROGRAM: &str = env!("CANOA_BRAND_PROGRAM");
/// The folder the unpacked copies live in, under the per-user data folder.
pub const FOLDER: &str = env!("CANOA_BRAND_FOLDER");
/// The program the launcher starts from the unpacked folder.
pub const APP_PROGRAM: &str = concat!(env!("CANOA_BRAND_EXECUTABLE"), ".exe");
/// The list of configuration files the app reads, which the launcher makes absolute.
pub const CONFIG: &str = concat!(env!("CANOA_RESOLVED_ENV_PREFIX"), "_CONFIG");
/// The folder that counts as beside the program: the launcher's own.
pub const INSTALLED_DIR: &str = concat!(env!("CANOA_RESOLVED_ENV_PREFIX"), "_INSTALLED_DIR");
/// Set to keep the launcher from opening a message box when it fails.
pub const NO_DIALOG: &str = concat!(env!("CANOA_RESOLVED_ENV_PREFIX"), "_LAUNCH_NO_DIALOG");
