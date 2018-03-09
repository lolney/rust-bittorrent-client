extern crate core;

use bittorrent::{metainfo, ParseError, Peer::Action, Peer::Peer, Peer::PeerInfo, Piece,
                 torrent::Torrent};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant, SystemTime};
use std::error::Error;
use std::sync::{Arc, Mutex};
use std::sync::mpsc;
use priority_queue::PriorityQueue;
use std::collections::HashMap;
use std::thread;
use std::io::{ErrorKind, Read, Write};
use std::fmt;
use std::str::from_utf8;
use std::fmt::Display;
use std::cmp::Ordering;
use bit_vec::BitVec;
use std::usize::MAX;
use log::error;

pub struct PeerManager {
    /*
    This should be a download rate-based PriorityQueue
    Coordinating thread needs to know:
    - Download rate
    - Whether peer is interested
    Must be able to control:
    - Choking/Unchoking
    */
    torrents: Arc<Mutex<HashMap<[u8; 20], Torrent>>>, // u8 is the Info_hash
    peer_id: [u8; 20],                                // our peer id
}

#[derive(Debug)]
pub struct NetworkError {
    msg: String,
}

impl Error for NetworkError {
    fn description(&self) -> &str {
        self.msg.as_str()
    }
    fn cause(&self) -> Option<&Error> {
        None
    }
}

impl fmt::Display for NetworkError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.msg)
    }
}

impl From<ParseError> for NetworkError {
    fn from(error: ParseError) -> Self {
        NetworkError {
            msg: error.to_string(),
        }
    }
}

macro_rules! acquire_torrent_lock {
    ($torrents:ident,$peer:ident,$torrent:ident) => (
        let mut torrents = $torrents.lock().unwrap();
        let hash = $peer.peer_info.info_hash;
        let mut $torrent = torrents.get_mut(&hash).unwrap();
    );
}

/// Instead of locking the peers data structure
/// and the priority queue on every access,
/// just send updates as needed.
/// We just lock for reads and writes, then,
/// which would be expensive to send anyway
#[derive(Debug)]
enum PeerUpdate {
    DownloadSpeed(usize),
    InterestedChange,
    Have(u32),
    Bitfield(BitVec),
    Disconnect,
}

impl PeerUpdate {
    fn process(
        self,
        peer_id: &[u8; 20],
        peers: &mut PriorityQueue<[u8; 20], PeerPriority>,
        bitfield: &mut BitVec,
    ) -> bool {
        match self {
            PeerUpdate::DownloadSpeed(speed) => {
                peers.change_priority_by(peer_id, |priority| priority.set_download(speed));
                true
            }
            PeerUpdate::InterestedChange => {
                peers.change_priority_by(peer_id, PeerPriority::flip_interested);
                true
            }
            PeerUpdate::Have(piece_index) => {
                bitfield.set(piece_index as usize, true);
                true
            }
            PeerUpdate::Bitfield(_bitfield) => {
                bitfield.union(&_bitfield);
                true
            }
            PeerUpdate::Disconnect => {
                remove(peers, peer_id);
                false
            }
        }
    }
}

enum ManagerUpdate {
    Choke,
    Unchoke,
    Request(Vec<u8>),
    Disconnect,
    None,
}

struct NewPeerMsg {
    peer_id: [u8; 20],
    comm: ManagerComm,
    priority: PeerPriority,
}

struct ManagerComm {
    manager_update_send: mpsc::Sender<ManagerUpdate>,
    // could instead have only a single copy of this and clone the sender:
    peer_update_recv: mpsc::Receiver<PeerUpdate>,
}

struct PeerComm {
    peer_update_send: mpsc::Sender<PeerUpdate>,
    manager_update_recv: mpsc::Receiver<ManagerUpdate>,
}

impl PeerComm {
    fn create() -> (ManagerComm, PeerComm) {
        let (peer_update_send, peer_update_recv) = mpsc::channel();
        let (manager_update_send, manager_update_recv) = mpsc::channel();

        (
            ManagerComm {
                manager_update_send: manager_update_send,
                peer_update_recv: peer_update_recv,
            },
            PeerComm {
                peer_update_send: peer_update_send,
                manager_update_recv: manager_update_recv,
            },
        )
    }

    fn recv(&self) -> ManagerUpdate {
        match self.manager_update_recv.try_recv() {
            Ok(update) => update,
            Err(err) => match err {
                mpsc::TryRecvError::Empty => ManagerUpdate::None,
                mpsc::TryRecvError::Disconnected => ManagerUpdate::Disconnect,
            },
        }
    }

    fn send(&self, update: PeerUpdate) {
        self.peer_update_send.send(update);
    }
}

impl ManagerComm {
    fn try_iter(&self) -> mpsc::TryIter<PeerUpdate> {
        self.peer_update_recv.try_iter()
    }

    fn send(&self, update: ManagerUpdate) {
        self.manager_update_send.send(update);
    }
}

/// Used by the choking manager to determine priority for
/// choking or requests
#[derive(Debug, Clone)]
struct PeerPriority {
    peer_interested: bool,
    download_speed: usize,
}

impl PeerPriority {
    pub fn new() -> PeerPriority {
        PeerPriority {
            peer_interested: false,
            download_speed: 0,
        }
    }

    pub fn flip_interested(peer: PeerPriority) -> PeerPriority {
        PeerPriority {
            peer_interested: !peer.peer_interested,
            download_speed: peer.download_speed,
        }
    }

    pub fn set_download(self, download_speed: usize) -> PeerPriority {
        PeerPriority {
            peer_interested: self.peer_interested,
            download_speed: download_speed,
        }
    }

    pub fn set_max(self) -> PeerPriority {
        PeerPriority {
            peer_interested: true,
            download_speed: MAX,
        }
    }
}

/// Ordering for unchoking selection
impl Ord for PeerPriority {
    fn cmp(&self, other: &Self) -> Ordering {
        if (self.peer_interested && other.peer_interested)
            || (!self.peer_interested && !other.peer_interested)
        {
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

fn remove(pq: &mut PriorityQueue<[u8; 20], PeerPriority>, id: &[u8; 20]) {
    pq.change_priority_by(id, |priority| priority.set_max());
    pq.pop();
}

impl PeerManager {
    fn new() -> PeerManager {
        PeerManager {
            torrents: Arc::new(Mutex::new(HashMap::new())),
            peer_id: Peer::gen_peer_id(),
        }
    }

    pub fn add_torrent(&mut self, metainfo_path: String) -> Result<(), ParseError> {
        match Torrent::new(metainfo_path) {
            Ok(torrent) => {
                let mut torrents = self.torrents.lock().unwrap();
                let info_hash = torrent.info_hash();
                torrents.insert(info_hash, torrent);
                Ok(())
            }
            Err(err) => Err(err),
        }
    }

    /// Handle incoming clients
    fn handle_client(
        stream: &TcpStream,
        torrents: &Arc<Mutex<HashMap<[u8; 20], Torrent>>>,
    ) -> Result<Peer, NetworkError> {
        let mut buf: [u8; 68] = [0; 68]; // size of handshake: 19 + 49
                                         // read handshake and create peer
        match Peer::parse_handshake(&buf) {
            Ok((peer_id, info_hash)) => {
                PeerManager::match_torrent(&info_hash, torrents)?;
                let info = PeerInfo {
                    peer_id: peer_id,
                    info_hash: info_hash,
                    ip: stream.peer_addr().unwrap().ip().to_string(),
                    port: stream.peer_addr().unwrap().port() as i64,
                };
                let peer = Peer::new(info);
                Ok(peer)
            }
            Err(err) => Err(NetworkError::from(err)),
        }
    }

    fn match_torrent(
        info_hash: &[u8; 20],
        torrents: &Arc<Mutex<HashMap<[u8; 20], Torrent>>>,
    ) -> Result<bool, NetworkError> {
        // Find the matching torrent
        let torrents = torrents.lock().unwrap();
        if torrents.contains_key(info_hash) {
            return Ok(true);
        }
        Err(NetworkError {
            msg: format!("Failed to find torrent with info hash: {:?}", info_hash),
        })
    }

    /// Writes to error log and to tcpstream
    /// Exchanges messages with the manager
    fn receive(
        mut peer: Peer,
        mut stream: TcpStream,
        torrents: Arc<Mutex<HashMap<[u8; 20], Torrent>>>,
        comm: PeerComm,
    ) {
        let mut buf = [0; ::MSG_SIZE];
        let mut download_size = 0;
        let start = Instant::now(); // TODO: consider abstracting this away?
                                    // macro for (task, period)
        let ten_secs = Duration::from_secs(10);

        stream.set_read_timeout(Some(Duration::new(::READ_TIMEOUT, 0)));

        loop {
            // match incoming messages from the peer
            match stream.read(&mut buf) {
                Err(e) => {
                    match e.kind() {
                        ErrorKind::WouldBlock => {
                            break;
                        } // timeout
                        _ => error!("Error while reading from stream: {}", e),
                    }
                }
                Ok(n) => {
                    match peer.parse_message(&buf) {
                        Ok(action) => match (action) {
                            Action::Request(requests) => {
                                acquire_torrent_lock!(torrents, peer, torrent);
                                for req in requests {
                                    match torrent.read_block(&req) {
                                        Ok(ref pd) => {
                                            stream.write(Peer::piece(&pd).as_slice());
                                        }
                                        Err(err) => {
                                            error!("Error while reading block to send to peer {:?}: {}",
                                                peer.peer_id(), err);
                                        }
                                    }
                                }
                            }
                            Action::Write(piece) => {
                                download_size = download_size + piece.piece.length;
                                acquire_torrent_lock!(torrents, peer, torrent);
                                torrent.write_block(&piece);
                            }
                            Action::InterestedChange => {
                                comm.send(PeerUpdate::InterestedChange);
                            }
                            Action::None => {}
                        },
                        Err(e) => {
                            error!(
                                "Error while parsing incoming message from peer {:?}: {}",
                                peer.peer_id(),
                                e
                            );
                        }
                    }
                }
            };

            // match inc commands from the manager
            match comm.recv() {
                ManagerUpdate::Request(req) => {
                    stream.write(req.as_slice());
                }
                ManagerUpdate::Choke => {
                    stream.write(peer.choke(true).as_slice());
                }
                ManagerUpdate::Unchoke => {
                    stream.write(peer.choke(false).as_slice());
                }
                ManagerUpdate::Disconnect => {
                    return;
                }
                ManagerUpdate::None => {}
            }
        }
    }

    fn process_queue(
        iter: &mut mpsc::TryIter<PeerUpdate>,
        peer_priority: &mut PriorityQueue<[u8; 20], PeerPriority>,
        bitfield: &mut BitVec,
        peer_id: &[u8; 20],
    ) -> bool {
        match iter.next() {
            Some(msg) => {
                msg.process(peer_id, peer_priority, bitfield);
                PeerManager::process_queue(iter, peer_priority, bitfield, peer_id)
            }
            None => true,
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
    fn manage_choking(recv: mpsc::Receiver<NewPeerMsg>) {
        let max_uploaders = 5;
        let mut peer_priority: PriorityQueue<[u8; 20], PeerPriority> = PriorityQueue::new();
        let mut peers: HashMap<[u8; 20], ManagerComm> = HashMap::new();
        let mut bitfields: HashMap<[u8; 20], BitVec> = HashMap::new();

        let start = Instant::now();
        let ten_secs = Duration::from_secs(10);

        loop {
            // TODO: consider sleep here
            // Messages sent when a new peer is added
            match recv.try_recv() {
                Ok(newpeer) => {
                    peer_priority.push(newpeer.peer_id, newpeer.priority);
                    peers.insert(newpeer.peer_id, newpeer.comm);
                }
                Err(err) => match (err) {
                    mpsc::TryRecvError::Empty => {}
                    mpsc::TryRecvError::Disconnected => return,
                },
            }

            // A message can arrive between the point where we check the queue
            // and determine which pieces to request, but it's probably not critical
            // that this information is 100% up to date -
            // this determination will probably not happen whenever possible
            peers.retain(|peer_id, comm| {
                let mut bitfield = bitfields.entry(peer_id.clone()).or_insert(BitVec::new());
                PeerManager::process_queue(
                    &mut comm.try_iter(),
                    &mut peer_priority,
                    bitfield,
                    &peer_id,
                )
            });

            if start.elapsed() >= ten_secs {
                let mut i = 0;
                for (id, priority) in peer_priority.clone().into_sorted_iter() {
                    let comm = peers.get(&id).unwrap();
                    if i < max_uploaders {
                        comm.send(ManagerUpdate::Unchoke);
                    } else {
                        comm.send(ManagerUpdate::Choke);
                    }
                    i = i + 1;
                }
            }
        }
    }

    fn handle(&mut self, port: &'static str) {
        // "127.0.0.1:80"
        let torrents = self.torrents.clone();

        let (manager_send, manager_recv) = mpsc::channel();

        thread::spawn(move || {
            // listens for incoming connections
            let listener = TcpListener::bind(port).unwrap();
            for stream in listener.incoming() {
                let torrents = torrents.clone();
                /*
                - Hashmap for looking up sender for a given peer
                - Doesn't need to be sent to connection threads
                - Peers must be accessble to the torrent object,
                - which decides which peer to request from based
                on their download rate
                */
                /*let (p,c) = unsafe { 
                    bounded_fast::new(10);
                };*/
                match stream {
                    Ok(stream) => {
                        match PeerManager::handle_client(&stream, &torrents) {
                            Ok(peer) => {
                                let (manager_comm, peer_comm) = PeerComm::create();

                                manager_send.send(NewPeerMsg {
                                    peer_id: peer.info_hash().clone(),
                                    comm: manager_comm,
                                    priority: PeerPriority::new(),
                                });

                                thread::spawn(move || {
                                    // manages new connection
                                    PeerManager::receive(peer, stream, torrents, peer_comm);
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

        // will need to be moved into own thread
        PeerManager::manage_choking(manager_recv);
    }
}
