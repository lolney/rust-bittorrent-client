extern crate core;

use bittorent::Peer;
use priority_queue::PriorityQueue;

struct PeerManager {
    peers : PriorityQueue<Peer, i32>
}

impl PeerManager {
    fn new(){
        peers = PriorityQueue::new()
    }

    fn handle(){
        // Network code
        // Update priority
        // Handle messages
        unimplemeneted!();
    }
}