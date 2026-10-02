//! Client for the SSAP websocket API on LG webOS TVs.
//!
//! LG never documented SSAP. Payloads here are checked against a real
//! OLED65B46LA (webOS 9.2.4), with aiowebostv as the reference for the rest.

mod client;
mod tls;
mod types;
mod wol;

pub use client::{Error, Result, TurnOff, Tv};
pub use tls::{CertFingerprint, CertPolicy, InvalidFingerprint};
pub use types::{
    AppId, Button, ClientKey, EmptyClientKey, Input, InputId, InvalidVolume, PowerState, Volume, VolumeStatus,
};
pub use wol::{InvalidMac, MacAddr, wake};
