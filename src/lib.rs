//#![feature(plugin)]
//#![plugin(peg_syntax_ext)]
//#![feature(inclusive_range_syntax)]

#[macro_use]
extern crate nom;
extern crate byteorder;

mod bittorrent;