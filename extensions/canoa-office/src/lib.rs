#![doc = include_str!("../README.md")]
#![cfg(windows)]

mod addin;
pub mod automation;
pub mod context;
mod exports;
pub mod host;
mod icon;
mod ids;
pub mod interfaces;
mod log;
mod owner;
mod pane;
pub mod relation;
mod ribbon;
pub mod surface;
pub mod webview;

pub use addin::{AddIn, pane_width};
pub use exports::{DllCanUnloadNow, DllGetClassObject};
pub use icon::{MARK_PNG, mark_picture, picture_from_png};
pub use ids::{CLSID_ADDIN, CLSID_PANE, PANE_PROG_ID, PROG_ID};
pub use pane::PaneControl;
pub use ribbon::ribbon_xml;
