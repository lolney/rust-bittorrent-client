//#![feature(plugin)]
//#![plugin(peg_syntax_ext)]
//#![feature(inclusive_range_syntax)]
#![feature(vec_remove_item)] 

#[macro_use]
extern crate nom;
extern crate byteorder;
extern crate bit_vec;

mod bittorrent;