extern crate core;

use bittorrent::{Peer::Peer, Peer::PeerInfo,
    Peer::Action, metainfo, torrent::Torrent, Piece, ParseError};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, SystemTime};
use std::error::Error;
use std::sync::{Arc, Mutex};
use priority_queue::PriorityQueue;
use std::collections::HashMap;
use std::{thread};
use std::io::{ErrorKind, Write, Read};
use std::fmt;
use std::str::from_utf8;
use std::fmt::Display;
use std::cmp::{Ordering};

pub struct PeerManager {
    /*
    This should be a download rate-based PriorityQueue
    Coordinating thread needs to know:
    - Download rate
    - Whether peer is interested
    Must be able to control:
    - Choking/Unchoking
    */
    // Using a priority queue (as opposed to BinaryHeap) 
    // to be able to modify arbitrary elements
    // In either case, can't use the actual peer elements -
    // mutation not allowed
    peers : Arc<Mutex<PriorityQueue<[u8 ; 20], PeerPriority>>>,
    torrents : Arc<Mutex<HashMap<[u8 ; 20], Torrent>>>, // u8 is the Info_hash
    peer_id : [u8 ; 20], // our peer id
}

#[derive(Debug)]
pub struct NetworkError {
    msg: String,
}

impl Error for NetworkError {
    fn description(&self) -> &str {self.msg.as_str()}
    fn cause(&self) -> Option<&Error> {None}
}

impl fmt::Display for NetworkError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.msg)
    }
}

impl From<ParseError> for NetworkError {
    fn from(error: ParseError) -> Self {
        NetworkError{msg: error.to_string()}
    }
}

macro_rules! acquire_torrent_lock {
    ($torrents:ident,$peer:ident,$torrent:ident) => (
        let mut torrents = $torrents.lock().unwrap();
        let hash = $peer.peer_info.info_hash;
        let mut $torrent = torrents.get_mut(&hash).unwrap();
    );
}

impl PeerManager {
    fn new() -> PeerManager {
        PeerManager{
            peers: Arc::new(Mutex::new(PriorityQueue::new())),
            torrents: Arc::new(Mutex::new(HashMap::new())),
            peer_id : Peer::gen_peer_id(),
        }
    } 

    pub fn add_torrent(&mut self, metainfo_path : String) -> Result<(), ParseError>{
        match Torrent::new(metainfo_path) {
            Ok(torrent) => {
                let mut torrents = self.torrents.lock().unwrap();
                let info_hash = torrent.info_hash();
                torrents.insert(info_hash, torrent);
                Ok(())
            },
            Err(err) => Err(err),
        }
    }

    /// Handle incoming clients
    fn handle_client(stream: &TcpStream, torrents: &Arc<Mutex<HashMap<[u8 ; 20], Torrent>>>) -> Result<Peer, NetworkError> {
        let mut buf : [u8 ; 68] = [0;68]; // size of handshake: 19 + 49
        // read handshake and create peer
        match Peer::parse_handshake(&buf) {
            Ok((peer_id, info_hash)) => {
                PeerManager::match_torrent(&info_hash, torrents)?;
                let info = PeerInfo{
                    peer_id: String::from(from_utf8(&peer_id).unwrap()),
                    info_hash: info_hash,
                    ip: stream.peer_addr().unwrap().ip().to_string(),
                    port: stream.peer_addr().unwrap().port() as i64,
                };
                let peer = Peer::new(info);
                Ok(peer)
            },
            Err(err) => Err(NetworkError::from(err)),
        }
    }

    fn match_torrent(info_hash : &[u8 ; 20], torrents: &Arc<Mutex<HashMap<[u8 ; 20], Torrent>>>) -> Result<bool, NetworkError> {
        // Find the matching torrent
        let torrents = torrents.lock().unwrap();
        if torrents.contains_key(info_hash){
            return Ok(true);
        }
        Err(NetworkError{msg: format!("Failed to find torrent with info hash: {:?}", info_hash)})
    }

    fn receive(mut peer : Peer, mut stream : TcpStream,
            torrents : Arc<Mutex<HashMap<[u8 ; 20], Torrent>>>,
            peers : Arc<Mutex<PriorityQueue<[u8 ; 20], PeerPriority>>>) {

        let mut buf = [0; ::MSG_SIZE];
        stream.set_read_timeout(Some(Duration::new(::READ_TIMEOUT, 0)));
        loop {
            match stream.read(&mut buf) {
                Err(e) => {
                    match e.kind() {
                        ErrorKind::WouldBlock => {break;}, // timeout
                        _ => {
                            error!("Error while reading from stream: {}", e)
                        },
                    }
                },
                Ok(n) => {
                    // TODO: proper error handling here
                    match peer.parse_message(&buf).unwrap() {
                        Action::Request(requests) => {
                            acquire_torrent_lock!(torrents, peer, torrent);
                            for req in requests {
                                let pd = torrent.read_block(&req);
                                stream.write(&peer.piece(&pd));
                            }
                        },
                        Action::Write(piece) => {
                            acquire_torrent_lock!(torrents, peer, torrent);
                            torrent.write_block(piece);
                        },
                        Action::InterestedChange => {
                            let mut peers = peers.lock().unwrap();
                            peers.change_priority_by(peer.info_hash(), PeerPriority::flip_interested);
                        }
                        Action::None => {},
                    }
                },
            };
        }
    }

    /*
    Choke: stop sending to this peer
    Choking algo (according to Bittorrent spec):
    - Check every 10 seconds
    - Unchoke 4 peers for which we have the best download rates (who are interested)
    - Peer with better download rate becomes interest: choke worst uploader
    - Maintain one Peer, regardless of download rate
    */
    fn manage_choking(peers : Arc<Mutex<PriorityQueue<[u8 ; 20], PeerPriority>>>) {
        
        let max_uploaders = 5;

        loop {
            let ten_secs = Duration::from_secs(10);
            thread::sleep(ten_secs);

            let peers = peers.lock().unwrap();
            let i = 0; // enumerate or into_sorted_iter not compiling
            for peer in peers.clone().into_sorted_vec() {
                if i < max_uploaders {
                    unimplemented!();
                    // peer.unchoke();
                } else {
                    unimplemented!();
                    //peer.choke();
                }
                i=i+1;
            }
        }
    }

    fn handle(&mut self, port : & 'static str) { // "127.0.0.1:80"
        let torrents = self.torrents.clone();
        let peers = self.peers.clone();

        thread::spawn(move || { // listens for incoming connections
            let listener = TcpListener::bind(port).unwrap();
            for stream in listener.incoming() {
                let torrents = torrents.clone();
                /*
                - Hashmap for looking up sender for a given peer
                - spmc for cancels or reuse hashmap?
                */
                /*let (p,c) = unsafe { 
                    bounded_fast::new(10);
                };*/
                match stream {
                    Ok(stream) => {
                        match PeerManager::handle_client(&stream, &torrents){
                            Ok(peer) => {
                                {
                                    let mut peers = peers.lock().unwrap();
                                    peers.push(peer.peer_info.info_hash.clone(), PeerPriority::new());
                                }
                                let peers = peers.clone();

                                thread::spawn(move || { // manages new connection
                                    PeerManager::receive(peer, stream, torrents, peers);
                                });
                            }
                            Err(err) => {
                                warn!("Dropping peer for improperly formatted handshake: {}", err);
                            }
                        }
                    }
                    Err(e) => { 
                        error!("Connection failed: {}", e); 
                    }
                }
            }
        });
    }
}

#[derive(Debug, Clone)]
struct PeerPriority {
    pub peer_interested: bool,
    pub download_speed: usize,
}

impl PeerPriority {
    pub fn new() -> PeerPriority {
        PeerPriority {
            peer_interested: false,
            download_speed: 0,
        }
    }

    pub fn flip_interested(peer : PeerPriority) -> PeerPriority {
        PeerPriority{
            peer_interested: !peer.peer_interested,
            download_speed: peer.download_speed,
        }
    }
}

/// Ordering for unchoking selection
impl Ord for PeerPriority {
    fn cmp(&self, other: &Self) -> Ordering {
        if (self.peer_interested && other.peer_interested) ||
            (!self.peer_interested && !other.peer_interested) {
            return self.download_speed.cmp(&other.download_speed);
        } else if self.peer_interested && !other.peer_interested {
            return Ordering::Greater;
        } else {
            return Ordering::Less;
        }
    }
}

impl PartialOrd for PeerPriority {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for PeerPriority {
    fn eq(&self, other: &Self) -> bool {
        match self.cmp(other) {
            Ordering::Equal => true,
            _ => false,
        }
    }
}

impl Eq for PeerPriority {}


// 