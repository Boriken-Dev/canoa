//! The identities the registry and Office know the add-in by.

use windows::core::GUID;

/// The add-in object Office creates: the class `PROG_ID` names.
pub const CLSID_ADDIN: GUID = GUID::from_u128(0xF1780EA2_B255_41D0_9014_97A28C29EB01);

/// The control a task pane hosts: the class `PANE_PROG_ID` names.
pub const CLSID_PANE: GUID = GUID::from_u128(0x9A886EBD_07B1_4695_B5AF_C082D892E989);

/// The add-in's ProgID, and the name of its key under each application's `Addins` key.
pub const PROG_ID: &str = "dev.boriken.canoa";

/// The ProgID Office is given to create the task pane's control.
pub const PANE_PROG_ID: &str = "dev.boriken.canoa.pane";
