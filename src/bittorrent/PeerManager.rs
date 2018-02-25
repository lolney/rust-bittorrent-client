extern crate core;

use bittorent::{Peer, Action, MetaInfo, Torrent};
//use priority_queue::PriorityQueue;
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, SystemTime};
use std::error::Error;

pub struct PeerManager {
    //peers : PriorityQueue<Peer, u32>,
    torrents : HashMap<[u8 ; 20], Torrent>, // u8 is the Info_hash
}

#[derive(Debug)]
pub struct NetworkError {
    msg: String,
    cause: Option<&Error>,
}

impl Error for NetworkError {
    fn description(&self) -> &str {self.msg.as_str()};
    fn cause(&self) -> Option<&Error> {self.cause}
}

#[derive(PartialEq, Debug, Clone)]
// Should also contain the pieces and the bitfield?
pub struct Torrent {
    metainfo : MetaInfo,
    path: Path,
}

impl Torrent {
    pub fn write_piece(&self, piece : PieceData) {
        self.metainfo.write_piece(path, piece);
    }
}

const PSTRLEN: u8 = 19;
const PSTR = "BitTorrent protocol";
const READ_TIMEOUT= Some(Duration::new(5, 0));

fn parse_handshake(msg : &[u8]) -> Result<([u8 ; 20], [u8 ; 20]), NetworkError>{
    let mut i = 0;

    let pstrlen = msg[i];
    if pstrlen != PSTRLEN {
        Err(NetworkError{msg: format!("Unexpected pstrlen: {}", pstrlen)})
    }

    i = i + 1;
    let pstr = &msg[i .. i + pstrlen];
    if pstr as str != PSTR {
        Err(NetworkError{msg: format!("Unexpected protocol string: {}", pstr)})
    }

    i = i + 8 + pstrlen;
    let info_hash : [u8 ; 20]= &msg[i .. i + 20];

    // Find the matching torrent
    mut bool found = false;
    for x in torrents {
        if x.info_hash() == info_hash {
            found = true; 
            break;
        }
    }
    if !found{
        Err(NetworkError::new(format!("Failed to find torrent with info hash: {}", x.info_hash)))
    }

    i = i + 20;
    let peer_id : [u8 ; 20] = &msg[i .. i + 20];

    return Ok((peer_id, info_hash))
}

impl PeerManager {
    fn new(){
        PeerManager{
            peers: PriorityQueue::new(),
            torrents: HashMap::new()
        }
    } 

    pub fn add_torrent(){
        unimplemented()!;
    }

    fn handle_client(&mut self, stream: TcpStream) -> Peer {
        let mut buf : [u8 : 68]; // size of handshake: 19 + 49
        // read
        match parse_handshake(buf) {
            Ok(peer_id, info_hash) {
                let info = PeerInfo{
                    peer_id: peer_id
                    ip: stream.peer_addr().ip().to_string()
                    port: stream.peer_addr().port() as i64
                }
                let peer = Peer::new(info);
                // self.peers.push(peer, begin.elapsed())
                return Peer;
            }
            Err(err) => {
                warn!("Dropping peer {} for improperly formatted handshake: {}"), stream, err); 
            }
        }

        
    }

    fn handle(&mut self) -> Result<(), NetworkError>{
        let begin = SystemTime::now();

        let (tx, rx) = mpsc::channel<&[u8]>(0);

        thread::spawn(move || { // listens for incoming connections
            let listener = TcpListener::bind("127.0.0.1:80").unwrap();

            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => {
                        let peer = handle_client(stream);

                        let tx = tx.clone();
                        thread::spawn(move || {
                            let mut buf = [0];
                            stream.set_read_timeout(READ_TIMEOUT);
                            loop {
                                match stream.read(&mut buf) {
                                    Err(e) => {
                                        match e.kind() {
                                            io::ErrorKind::WouldBlock => {break;}, // timeout
                                            _ => {
                                                error!("Error while reading from stream: {}", e)
                                            },
                                        }
                                    },
                                    Ok(msg) => {
                                        match peer.parse_message(msg){
                                            Request(pieces) => {
                                                stream.write(&peer.request(pieces));
                                            },
                                            Write(piece) => {
                                                tx.send(piece)
                                            },
                                            None => {},
                                        }
                                    },
                                };
                            }

                        })
                    }
                    Err(e) => { 
                        error!("Connection failed: {}", e); 
                    }
                }
            }
        }

        loop{
            match rx.recv() {
                Ok(piece) => {
                    torrent = self.torrents.get(piece.info_hash);
                    torrent.write_piece(piece);
                }
                Err(error) => {
                    error!("Error retrieving data from handler thread: {}", e);
                }
            }
        }
    }
}