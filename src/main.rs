extern crate bittorrent;
extern crate cursive;
extern crate cursive_table_view;

pub mod client;

use client::client as c;

fn main() {
    c::run();
}
