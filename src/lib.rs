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
extern crate rand;
//extern crate priority_queue;

mod bittorrent;

const HSLEN : usize = 68; // 49 + 19
const PSTRLEN : u8 = 19;
const PSTR : &'static str = "BitTorrent protocol";
const READ_TIMEOUT : u64 = 5;
const MSG_SIZE : usize = 2^16;