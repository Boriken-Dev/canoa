#![doc = include_str!("../README.md")]
#![cfg(windows)]

mod addin;
mod exports;
mod ids;
pub mod interfaces;
mod pane;
mod ribbon;

pub use addin::AddIn;
pub use exports::{DllCanUnloadNow, DllGetClassObject};
pub use ids::{CLSID_ADDIN, CLSID_PANE, PANE_PROG_ID, PROG_ID};
pub use pane::PaneControl;
pub use ribbon::ribbon_xml;
