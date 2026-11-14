#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]

extern crate alloc;

mod driver;
mod kind;
mod provider;
pub mod serve;
pub mod settings;
pub mod threads;

#[doc(hidden)]
pub use alloc::vec::Vec as __Vec;
pub use guatiao;
pub use guatiao_channel::{Channel, ChannelSink, ChannelSinkVtable, ChannelVtable};

pub use driver::{Accepted, Acceptor, Driver, DriverError, Refusal};
pub use kind::{
    DEFAULT_DIAL_TIMEOUT_MS, DialRequest, ListenRequest, NonDefault, Transport, TransportAcceptor,
    TransportAcceptorVtable, TransportInfo, TransportListener, TransportListenerVtable,
    TransportVtable,
};

#[cfg(feature = "pipe")]
#[cfg_attr(docsrs, doc(cfg(feature = "pipe")))]
pub mod pipe;

#[cfg(feature = "websocket")]
#[cfg_attr(docsrs, doc(cfg(feature = "websocket")))]
pub mod websocket;

#[cfg(feature = "contract")]
#[cfg_attr(docsrs, doc(cfg(feature = "contract")))]
pub mod contract;
