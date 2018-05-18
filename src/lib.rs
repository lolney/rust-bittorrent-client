//#![feature(plugin)]
//#![plugin(peg_syntax_ext)]
//#![feature(inclusive_range_syntax)]
#![feature(vec_remove_item)]
#![feature(use_extern_macros)]
#![feature(iterator_try_fold)]
#![feature(try_trait)]
#![feature(proc_macro, conservative_impl_trait, generators)]
#![allow(dead_code)]
#![feature(collections)]

extern crate bit_vec;
extern crate byteorder;
extern crate env_logger;
#[macro_use]
extern crate log;

extern crate priority_queue;
extern crate rand;
extern crate sha1;

extern crate futures_await as futures;
extern crate hyper;
extern crate tokio_core;
extern crate tokio_timer;
extern crate url;

extern crate serde;
#[macro_use]
extern crate serde_derive;

#[macro_use]
pub mod bittorrent;

const HSLEN: usize = 68; // 49 + 19
const PSTRLEN: u8 = 19;
const PSTR: &'static str = "BitTorrent protocol";
const READ_TIMEOUT: u64 = 120; // in seconds
const MSG_SIZE: usize = 1 << 16;
/// Recommended message size
const REC_SIZE: usize = 1 << 14;

const DL_DIR: &'static str = "test/test_write";
const READ_DIR: &'static str = "test/test_read";
/// Default test file
const TEST_FILE: &'static str = "test/torrent.torrent";
const TEST_BIBLE: &'static str = "test/bible.torrent";
/// Max outstanding requests per torrent
const REQUESTS_LIMIT: usize = 1;
const MAX_PEERS: usize = 30;
const PORT_NUM: u16 = 6881;
