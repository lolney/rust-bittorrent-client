extern crate bittorrent;
extern crate cursive;
extern crate cursive_table_view;
extern crate rand;

pub mod client;

use client::client as c;

fn main() {
    c::run();
}
