#![forbid(unsafe_code)]

use std::net::IpAddr;

pub mod api;
pub mod client;
pub mod media;
pub mod models;
mod parsing;
pub mod search;
pub mod server;
pub mod service;
pub mod storage;

pub(crate) fn is_proxyable_ip(address: IpAddr) -> bool {
	!address.is_multicast() && !bogon::ip_addr_is_bogon(address)
}
