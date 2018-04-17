extern crate bittorrent;
extern crate cursive;
extern crate cursive_table_view;
#[macro_use]
extern crate log;
extern crate rand;

mod client;
mod table;

use client as cl;

fn main() {
    cl::run();
}
