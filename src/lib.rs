//#![feature(plugin)]
//#![plugin(peg_syntax_ext)]
//#![feature(inclusive_range_syntax)]
#![feature(vec_remove_item)]
#![feature(use_extern_macros)]

extern crate bit_vec;
extern crate byteorder;
#[macro_use]
extern crate log;
extern crate priority_queue;
extern crate rand;
extern crate sha1;

extern crate futures;
extern crate hyper;
extern crate tokio_core;
extern crate url;

mod bittorrent;

const HSLEN: usize = 68; // 49 + 19
const PSTRLEN: u8 = 19;
const PSTR: &'static str = "BitTorrent protocol";
const READ_TIMEOUT: u64 = 120; // in seconds
const MSG_SIZE: usize = 2 ^ 16;
/// Recommended message size
const REC_SIZE: usize = 2 ^ 14;

const DL_DIR: &'static str = "test/test";
/// Default test file
const TEST_FILE: &'static str = "test/bible.torrent";
/// Max outstanding requests per torrent
const REQUESTS_LIMIT: usize = 1;
const MAX_PEERS: usize = 30;
const PORT_NUM: usize = 6881;
