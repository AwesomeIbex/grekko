#![feature(option_result_contains)]
#![feature(in_band_lifetimes)]

#[macro_use]
extern crate cached;

use grekko::start_server;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

pub mod geocoding;
pub mod osrm_service;
pub mod redis_manager;
pub mod request;
pub mod solver;
pub mod mapbox;

#[tokio::main]
async fn main() {
    let socket = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 3030);
    start_server(socket).await;
}
