/* Tracker communication handled here
*/
use bittorrent::Peer::{Peer};

struct Trackers {
    urls: Vec<Tracker>,
}

#[derive(Debug)]
struct Tracker {
    tracker_id: String,
    peers: Vec<Peer>,
    interval: i64,
    min_interval: Option<i64>,
    complete: i64,
    incomplete: i64,
}

impl Tracker {
    pub fn announce() {

    }

    /// Queries the state of a given torrent
    pub fn scrape() {

    }
}