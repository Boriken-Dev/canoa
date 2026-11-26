//! The names this add-in carries, from the brand file it was built with (`BRAND_FILE`, else the copy `canoa` holds). It cannot depend
//! on `canoa`, so it reads the same file in its build script, and each name must match what the host takes from its own.

/// What the product is called.
pub const NAME: &str = env!("CANOA_BRAND_NAME");
/// The command line's name: what a message tells the person to run.
pub const PROGRAM: &str = env!("CANOA_BRAND_PROGRAM");
/// The configuration file's name.
pub const CONFIG_FILE: &str = env!("CANOA_BRAND_CONFIG_FILE");
/// The folder under the per-user data folder where the add-in keeps its log and its browser's data.
pub const FOLDER: &str = env!("CANOA_BRAND_FOLDER");
/// The home under the person's own folder.
pub const HOME_DIR: &str = env!("CANOA_BRAND_HOME_DIR");
/// Set to keep a message box from opening, which a test does.
pub const NO_DIALOG: &str = concat!(env!("CANOA_RESOLVED_ENV_PREFIX"), "_OFFICE_NO_DIALOG");
/// The file the add-in's log goes to, when it is not the default.
pub const LOG: &str = concat!(env!("CANOA_RESOLVED_ENV_PREFIX"), "_OFFICE_LOG");
/// The folder the pane's browser keeps its data in, when it is not the default.
pub const DATA: &str = concat!(env!("CANOA_RESOLVED_ENV_PREFIX"), "_OFFICE_DATA");
