//#![feature(plugin)]
//#![plugin(peg_syntax_ext)]
//#![feature(inclusive_range_syntax)]
#![feature(vec_remove_item)] 

#[macro_use]
extern crate nom;
#[macro_use]
extern crate log;
extern crate byteorder;
extern crate bit_vec;
extern crate sha1;

mod bittorrent;