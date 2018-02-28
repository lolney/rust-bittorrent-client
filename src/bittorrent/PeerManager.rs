extern crate core;

use bittorrent::{Peer::Peer, Peer::PeerInfo, Peer::Action, Metainfo, Metainfo::Torrent, Piece, ParseError};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, SystemTime};
use std::error::Error;
use std::sync::{Arc, Mutex};
use std::collections::binary_heap::{BinaryHeap};
use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver};
use std::{thread};
use std::io::{ErrorKind, Write, Read};
use std::fmt;
use std::str::from_utf8;
use std::fmt::Display;

pub struct PeerManager {
    /*
    This should be a download rate-based PriorityQueue
    Coordinating thread needs to know:
    - Download rate
    - Whether peer is interested
    Must be able to control:
    - Choking/Unchoking
    */
    peers : Arc<Mutex<BinaryHeap<Peer>>>,
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

impl PeerManager {
    fn new() -> PeerManager {
        PeerManager{
            peers: Arc::new(Mutex::new(BinaryHeap::new())),
            torrents: Arc::new(Mutex::new(HashMap::new())),
            peer_id : Peer::gen_peer_id(),
        }
    } 

    pub fn add_torrent(&mut self, metainfo_path : String){
        let mut torrents = self.torrents.lock().unwrap();
        let torrent = Torrent::new(metainfo_path);
        let info_hash = torrent.info_hash();
        torrents.insert(info_hash, torrent);
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
                // self.peers.push(peer, begin.elapsed())
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

    fn receive(mut peer : Peer, mut stream : TcpStream, torrents : Arc<Mutex<HashMap<[u8 ; 20], Torrent>>>) {
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
                            let mut torrents = torrents.lock().unwrap();
                            let hash = peer.peer_info.info_hash;
                            let mut torrent = torrents.get_mut(&hash).unwrap();
                            for req in requests {
                                let pd = torrent.read_block(&req);
                                stream.write(&peer.piece(&pd));
                            }
                        },
                        Action::Write(piece) => {
                            let mut torrents = torrents.lock().unwrap();
                            let hash = peer.peer_info.info_hash;
                            let mut torrent = torrents.get_mut(&hash).unwrap();
                            torrent.write_block(piece);
                        },
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

    Handling is currently torrent-agnostic (each piece comes with an info hash),
    So can global just maintain global numbers
    */
    fn manage_choking() {
        unimplemented!();
    }

    fn handle(&mut self, port : &str) { // "127.0.0.1:80"
        let torrents = self.torrents.clone();

        thread::spawn(move || { // listens for incoming connections
            let listener = TcpListener::bind(port).unwrap();
            for stream in listener.incoming() {
                let torrents = torrents.clone();
                match stream {
                    Ok(stream) => {
                        match PeerManager::handle_client(&stream, &torrents){
                            Ok(peer) => {
                                thread::spawn(move || {
                                    PeerManager::receive(peer, stream, torrents);
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

// 