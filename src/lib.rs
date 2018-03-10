//#![feature(plugin)]
//#![plugin(peg_syntax_ext)]
//#![feature(inclusive_range_syntax)]
#![feature(vec_remove_item)]
#![feature(use_extern_macros)]

extern crate bit_vec;
extern crate byteorder;
#[macro_use]
extern crate log;
#[macro_use]
extern crate nom;
extern crate priority_queue;
extern crate rand;
extern crate sha1;

mod bittorrent;

const HSLEN: usize = 68; // 49 + 19
const PSTRLEN: u8 = 19;
const PSTR: &'static str = "BitTorrent protocol";
const READ_TIMEOUT: u64 = 5;
const MSG_SIZE: usize = 2 ^ 16;

const DL_DIR: &'static str = "test/test";
// Default test file
const TEST_FILE: &'static str = "test/bible.torrent";
