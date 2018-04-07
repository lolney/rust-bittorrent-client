extern crate core;

use bittorrent::{metainfo, torrent, Hash, ParseError, Piece, peer::Action, peer::Peer,
                 peer::PeerInfo, torrent::Torrent, tracker::Tracker};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::time::{Duration, Instant, SystemTime};
use std::error::Error;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use std::sync::mpsc;
use std::sync::mpsc::Sender;
use priority_queue::PriorityQueue;
use std::collections::HashMap;
use std::thread;
use std::io::{ErrorKind, Read, Write};
use std::fmt;
use std::cmp::Ordering;
use bit_vec::BitVec;
use std::usize::MAX;
use log::error;
use std::io::Error as IOError;
use tokio_core::reactor::{Core, Handle};

use futures::prelude::{async, Future, Sink, Stream};

/// This module contains the main point of interaction with the library, `Manager`.
/// Calling `Manager::new().handle()` will spawn the `Controller` thread,
/// which chooses which pieces to request and which peers to upload to.
/// Each connection has a `Worker` task, which handles communication with a `Peer`,
/// reads and writes to disk, and sends relevant updates to the Controller.

pub struct Manager {
    /*
    This should be a download rate-based PriorityQueue
    Coordinating thread needs to know:
    - Download rate
    - Whether peer is interested
    Must be able to control:
    - Choking/Unchoking
    */
    torrents: Arc<Mutex<HashMap<Hash, Torrent>>>, // u8 is the Info_hash
    peer_id: Hash,                                // our peer id
    npeers: Arc<AtomicUsize>,                     // TODO: can this just be in the controller?
    manager_send: Option<Sender<NewPeerMsg>>,     // Must be initialized when Controller is created
    port: u16,
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
        let mut torrents = $torrents.lock().expect("Torrents mutex poisoned");
        let hash = $peer.peer_info.info_hash;
        let mut $torrent = torrents.get_mut(&hash).expect("Expected torrent missing");
    );
}

/// Instead of locking the peers data structure
/// and the priority queue on every access,
/// just send updates as needed.
/// We just lock for reads and writes, then,
/// which would be expensive to send anyway
#[derive(Debug, Clone)]
enum PeerUpdate {
    DownloadSpeed(usize),
    InterestedChange,
    ChokingChange,
    Have(u32),
    Bitfield(BitVec),
    Disconnect,
}

/// Note: assumes data has been validated by Peer
impl PeerUpdate {
    fn process(
        self,
        peer_id: &Hash,
        peers: &mut PriorityQueue<Hash, PeerPriority>,
        bitfield: &mut BitVec,
    ) -> bool {
        match self {
            PeerUpdate::DownloadSpeed(speed) => {
                peers.change_priority_by(peer_id, |priority| priority.set_download(speed));
                false
            }
            PeerUpdate::InterestedChange => {
                peers.change_priority_by(peer_id, PeerPriority::flip_interested);
                false
            }
            PeerUpdate::ChokingChange => {
                peers.change_priority_by(peer_id, PeerPriority::flip_choking);
                false
            }
            PeerUpdate::Have(piece_index) => {
                bitfield.set(piece_index as usize, true);
                false
            }
            PeerUpdate::Bitfield(_bitfield) => {
                if bitfield.len() == 0 {
                    bitfield.clone_from(&_bitfield);
                } else if bitfield.len() == _bitfield.len() {
                    bitfield.union(&_bitfield);
                } else {
                    panic!("PeerUpdate sent invalid bitfield");
                }
                false
            }
            PeerUpdate::Disconnect => true,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Paused,
    Running,
    Complete,
}

#[derive(Debug, Clone)]
pub struct Info {
    pub info_hash: Hash,
    pub name: String,
    pub status: Status,
    pub progress: f32,
    pub up: usize,
    pub down: usize,
    pub npeers: usize,
}

#[derive(Debug, Clone)]
pub enum InfoMsg {
    All(Vec<Info>),
    One(Info),
}

impl Ord for Status {
    fn cmp(&self, other: &Self) -> Ordering {
        let to_int = |s| match s {
            Paused => 0,
            Running => 1,
            Complete => 2,
        };
        to_int(self).cmp(&to_int(other))
    }
}

impl PartialOrd for Status {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

pub struct NewPeerMsg {
    peer_id: Hash,
    info_hash: Hash,
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

enum Handshake {
    Initiator { peer_id: Hash, info_hash: Hash },
    Receiver(Hash),
}

/// Used by the choking manager to determine priority for
/// choking or requests
#[derive(Debug, Clone)]
struct PeerPriority {
    info_hash: Hash,
    peer_interested: bool,
    download_speed: usize,
    peer_choking: bool,
}

impl PeerPriority {
    pub fn new(info_hash: Hash) -> PeerPriority {
        PeerPriority {
            info_hash: info_hash,
            peer_interested: false,
            download_speed: 0,
            peer_choking: true,
        }
    }

    pub fn flip_interested(peer: PeerPriority) -> PeerPriority {
        PeerPriority {
            info_hash: peer.info_hash,
            peer_interested: !peer.peer_interested,
            download_speed: peer.download_speed,
            peer_choking: peer.peer_choking,
        }
    }

    pub fn flip_choking(peer: PeerPriority) -> PeerPriority {
        PeerPriority {
            info_hash: peer.info_hash,
            peer_interested: peer.peer_interested,
            download_speed: peer.download_speed,
            peer_choking: !peer.peer_choking,
        }
    }

    pub fn set_download(self, download_speed: usize) -> PeerPriority {
        PeerPriority {
            info_hash: self.info_hash,
            peer_interested: self.peer_interested,
            download_speed: download_speed,
            peer_choking: self.peer_choking,
        }
    }

    pub fn set_max(self) -> PeerPriority {
        PeerPriority {
            info_hash: self.info_hash,
            peer_interested: true,
            download_speed: MAX,
            peer_choking: self.peer_choking,
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

fn remove(pq: &mut PriorityQueue<Hash, PeerPriority>, id: &Hash) {
    pq.change_priority_by(id, |priority| priority.set_max());
    pq.pop();
}

impl Manager {
    pub fn new() -> Manager {
        Manager {
            torrents: Arc::new(Mutex::new(HashMap::new())),
            peer_id: Peer::gen_peer_id(),
            npeers: Arc::new(AtomicUsize::new(0)),
            manager_send: None,
            port: ::PORT_NUM,
        }
    }

    /// Used in a builder pattern to set the listening port
    pub fn port(mut self, port: u16) -> Manager {
        self.port = port;
        self
    }

    /// Resume torrent from file
    pub fn resume_torrent(&mut self) {
        // deserialize
        // _add_torrent(torrent)
        unimplemented!();
    }

    /// Add a new torrent from file `metainfo_path`, downloading
    pub fn add_torrent(
        &mut self,
        metainfo_path: &str,
        download_path: &str,
    ) -> Result<(), ParseError> {
        match Torrent::new(metainfo_path.to_string(), download_path.to_string()) {
            Ok(torrent) => self._add_torrent(torrent),
            Err(err) => Err(err),
        }
    }

    fn _add_torrent(&mut self, torrent: Torrent) -> Result<(), ParseError> {
        let mut core = Core::new()?;
        let info_hash = torrent.info_hash();
        let stream = Tracker::get_peers(
            core.handle(),
            info_hash,
            self.peer_id,
            torrent.trackers(),
            self.port.clone(),
        )?;

        {
            let mut torrents = self.torrents.lock().expect("Torrents mutex poisoned");

            torrents.insert(info_hash, torrent);
        }

        let torrents = self.torrents.clone();
        let npeers = self.npeers.clone();
        let channel = self.manager_send
            .clone()
            .expect("Handle not called before adding torrent");
        let peer_id = self.peer_id;
        let port = self.port;

        core.run(stream.for_each(|ips| {
            info!("Tracker response received with {} ips", ips.len());
            for ip in ips {
                debug!("Received IP address from tracker: {}", ip);
                if ip.port() == port {
                    warn!("Avoiding connection with self: {}", port);
                    continue;
                }
                let channel = channel.clone();
                let torrents = torrents.clone();
                let npeers = npeers.clone();

                Manager::connect(
                    TcpStream::connect(ip),
                    torrents,
                    npeers,
                    channel,
                    Handshake::Initiator {
                        peer_id: peer_id.clone(),
                        info_hash: info_hash,
                    },
                );
            }
            Ok(())
        })).map_err(|e| {
            error!("Error while announcing to tracker: {:?}", e);
            e
        })
    }

    /// Handle incoming clients
    fn handle_client(
        stream: &mut TcpStream,
        torrents: &Arc<Mutex<HashMap<Hash, Torrent>>>,
        handshake: Handshake,
    ) -> Result<Peer, ParseError> {
        if let Handshake::Initiator { peer_id, info_hash } = handshake {
            stream.write(&Peer::handshake(&peer_id, &info_hash));
        }
        let mut buf: [u8; ::HSLEN] = [0; ::HSLEN];
        // read handshake and create peer
        match stream.read(&mut buf) {
            Err(e) => Err(ParseError::new_cause(
                "Error while waiting for handshake",
                e,
            )),
            Ok(n) => {
                let (their_peer_id, their_info_hash) = Peer::parse_handshake(&buf)?;
                match handshake {
                    Handshake::Initiator { peer_id, info_hash } => {
                        if info_hash != their_info_hash {
                            return Err(parse_error!(
                                "Info hash in handshake ({}) does not match expected ({})",
                                their_info_hash,
                                info_hash
                            ));
                        }
                    }
                    Handshake::Receiver(peer_id) => {
                        Manager::match_torrent(&their_info_hash, torrents)?;
                        stream.write(&Peer::handshake(&peer_id, &their_info_hash));
                    }
                }
                let info = PeerInfo {
                    peer_id: their_peer_id,
                    info_hash: their_info_hash,
                    ip: stream
                        .peer_addr()
                        .expect("Expected connection to have IP")
                        .ip()
                        .to_string(),
                    port: stream
                        .peer_addr()
                        .expect("Expected connection to have port")
                        .port() as i64,
                };
                let peer = Peer::new(info);
                Ok(peer)
            }
        }
    }

    fn match_torrent(
        info_hash: &Hash,
        torrents: &Arc<Mutex<HashMap<Hash, Torrent>>>,
    ) -> Result<bool, ParseError> {
        // Find the matching torrent
        let torrents = torrents.lock().expect("Torrents mutex poisoned");
        if torrents.contains_key(info_hash) {
            return Ok(true);
        }
        Err(parse_error!(
            "Failed to find torrent with info hash: {}",
            info_hash
        ))
    }

    /// Writes to error log and to tcpstream
    /// Exchanges messages with the manager
    fn receive(
        mut peer: Peer,
        mut stream: TcpStream,
        torrents: Arc<Mutex<HashMap<Hash, Torrent>>>,
        comm: PeerComm,
    ) {
        let mut buf = [0; ::MSG_SIZE];
        let mut download_size = 0;
        let start = Instant::now(); // TODO: consider abstracting this away?
                                    // macro for (task, period)
        let ten_secs = Duration::from_secs(10);

        stream
            .set_read_timeout(Some(Duration::new(::READ_TIMEOUT, 0)))
            .unwrap_or_else(|e| error!("Failed to set read timeout on stream: {}", e));

        loop {
            // match incoming messages from the peer
            match stream.read(&mut buf) {
                Err(e) => {
                    match e.kind() {
                        ErrorKind::WouldBlock => {
                            comm.send(PeerUpdate::Disconnect);
                            return;
                        } // timeout
                        _ => error!("Error while reading from stream: {}", e),
                    }
                }
                Ok(n) => match peer.parse_message(&buf) {
                    Ok(action) => match (action) {
                        Action::Request(requests) => {
                            acquire_torrent_lock!(torrents, peer, torrent);
                            for req in requests {
                                match torrent.read_block(&req) {
                                    Ok(ref pd) => {
                                        trace!(
                                            "Reading piece for peer {}: {:?}",
                                            peer.peer_id(),
                                            pd
                                        );
                                        stream.write(Peer::piece(&pd).as_slice());
                                    }
                                    Err(err) => {
                                        error!(
                                            "Error while sending block to peer {}: {}",
                                            peer.peer_id(),
                                            err
                                        );
                                    }
                                }
                            }
                        }
                        Action::Write(piece) => {
                            trace!("Writing piece from peer {}: {:?}", peer.peer_id(), piece);
                            download_size = download_size + piece.piece.length;
                            acquire_torrent_lock!(torrents, peer, torrent);
                            torrent.write_block(&piece);
                        }
                        Action::InterestedChange => {
                            trace!(
                                "Peer {} interested status now: {:?}",
                                peer.peer_id(),
                                peer.peer_interested
                            );
                            comm.send(PeerUpdate::InterestedChange);
                        }
                        Action::Have(index) => {
                            comm.send(PeerUpdate::Have(index));
                        }
                        Action::Bitfield(bitfield) => {
                            comm.send(PeerUpdate::Bitfield(bitfield));
                        }
                        Action::None => {}
                    },
                    Err(e) => {
                        error!(
                            "Error while parsing incoming message from peer {}: {}",
                            peer.peer_id(),
                            e
                        );
                    }
                },
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
                    info!(
                        "Manager has ordered disconnect from peer {}",
                        peer.peer_id()
                    );
                    return;
                }
                ManagerUpdate::None => {}
            }
        }
    }

    fn register_peer(
        peer: &Peer,
        npeers: Arc<AtomicUsize>,
        manager_send: &Sender<NewPeerMsg>,
    ) -> PeerComm {
        npeers.fetch_add(1, AtomicOrdering::SeqCst);
        let (manager_comm, peer_comm) = PeerComm::create();

        manager_send
            .send(NewPeerMsg {
                peer_id: peer.peer_id().clone(),
                info_hash: peer.info_hash().clone(),
                comm: manager_comm,
                priority: PeerPriority::new(peer.info_hash().clone()),
            })
            .or_else(|err| {
                error!("Failed to send message for new peer: {}", err);
                Err(err)
            });
        return peer_comm;
    }

    fn connect(
        stream: Result<TcpStream, IOError>,
        torrents: Arc<Mutex<HashMap<Hash, Torrent>>>,
        npeers: Arc<AtomicUsize>,
        manager_send: Sender<NewPeerMsg>,
        handshake: Handshake,
    ) {
        match stream {
            Ok(mut stream) => {
                if npeers.load(AtomicOrdering::SeqCst) >= ::MAX_PEERS {
                    stream.shutdown(Shutdown::Both);
                    warn!(
                        "Max numbers of peers {} reached; rejecting new connection",
                        ::MAX_PEERS
                    );
                } else {
                    match Manager::handle_client(&mut stream, &torrents, handshake) {
                        Ok(peer) => {
                            info!("New peer connected: {}", peer.peer_id());
                            let peer_comm = Manager::register_peer(&peer, npeers, &manager_send);

                            thread::spawn(move || {
                                // manages new connection
                                Manager::receive(peer, stream, torrents, peer_comm);
                            });
                        }
                        Err(err) => {
                            stream.shutdown(Shutdown::Both);
                            warn!("Dropping peer for improperly formatted handshake: {}", err);
                        }
                    }
                }
            }
            Err(e) => {
                error!("Connection failed: {}", e);
            }
        }
    }

    /// Entry point. Only meant to be called once.
    /// Returns channel for receiving progress updates.
    pub fn handle(&mut self) -> mpsc::Receiver<InfoMsg> {
        let torrents = self.torrents.clone();

        let (manager_send, manager_recv) = mpsc::channel();
        self.manager_send = Some(manager_send.clone());

        let npeers = self.npeers.clone();
        let peer_id = self.peer_id.clone();
        let port = self.port.clone();

        thread::spawn(move || {
            // listens for incoming connections
            let listener = TcpListener::bind(format!("127.0.0.1:{}", port))
                .expect("Failed to bind listening port");
            info!(
                "Listening on port {} for new peers as peer {}",
                port, peer_id
            );
            listener.incoming().for_each(|stream| {
                let torrents = torrents.clone();
                Manager::connect(
                    stream,
                    torrents,
                    npeers.clone(),
                    manager_send.clone(),
                    Handshake::Receiver(peer_id),
                );
            });
        });

        let torrents = self.torrents.clone();
        let npeers = self.npeers.clone();
        let (info_send, info_recv) = mpsc::channel();
        thread::spawn(move || {
            Controller::new(manager_recv, torrents, npeers, info_send).run_loop();
        });

        return info_recv;
    }
}

macro_rules! torrents {
    ($self: ident) => {
        $self.torrents.lock().expect("Torrents mutex poisoned");
    }
}

struct Controller {
    recv: mpsc::Receiver<NewPeerMsg>,
    torrents: Arc<Mutex<HashMap<Hash, Torrent>>>,
    npeers: Arc<AtomicUsize>,
    send: mpsc::Sender<InfoMsg>,
    peer_priority: PriorityQueue<Hash, PeerPriority>,
    peers: HashMap<Hash, ManagerComm>,
    bitfields: HashMap<Hash, BitVec>,
}

impl Controller {
    pub fn new(
        recv: mpsc::Receiver<NewPeerMsg>,
        torrents: Arc<Mutex<HashMap<Hash, Torrent>>>,
        npeers: Arc<AtomicUsize>,
        send: mpsc::Sender<InfoMsg>,
    ) -> Controller {
        Controller {
            recv: recv,
            torrents: torrents,
            npeers: npeers,
            send: send,
            peer_priority: PriorityQueue::new(),
            peers: HashMap::new(),
            bitfields: HashMap::new(),
        }
    }

    // TODO: This is a strong candidate to be replaced by a task system;
    // Task: returns Option<Update>
    pub fn run_loop(&mut self) {
        let start = Instant::now();
        let ten_secs = Duration::from_secs(10);

        loop {
            /// Send updates at 1-second interval
            self.send_update();

            /// Receive messages sent when a new peer is added
            self.try_recv_new_peer();

            self.check_messages();

            // Update choking on 10-sec intervals
            if start.elapsed() >= ten_secs {
                self.choke();
            }

            self.make_requests();
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
    fn choke(&mut self) {
        let max_uploaders = 5; // TODO: config
        let mut i = 0;
        for (id, priority) in self.peer_priority.clone().into_sorted_iter() {
            let comm = self.peers
                .get(&id)
                .expect("Peer in priority queue but not comms map");
            if i < max_uploaders {
                comm.send(ManagerUpdate::Unchoke);
            } else {
                comm.send(ManagerUpdate::Choke);
            }
            i = i + 1;
        }
    }

    fn make_requests(&mut self) {
        let mut torrents = torrents!(self);
        torrents.retain(|info_hash, ref mut torrent: &mut Torrent| {
            let mut peers_iter = self.peer_priority
                .iter()
                .filter(|&(k, v)| !v.peer_choking && v.info_hash == *info_hash);
            let mut peer = peers_iter.next();

            while peer.is_some() && torrent.nrequests < ::REQUESTS_LIMIT {
                match torrent.select_piece() {
                    Some(piece) => {
                        // Request piece
                        torrent.inc_nrequests();
                        let (id, v) = peer.unwrap();
                        let comm = self.peers
                            .get(id)
                            .expect("Peer in priority queue but not comms map");
                        comm.send(ManagerUpdate::Request(Peer::request(&piece)));
                        peer = peers_iter.next();
                    }
                    None => {
                        info!("Torrent \"{}\" complete", torrent.name());
                        self.send.send(InfoMsg::One(self.create_info(
                            info_hash,
                            torrent,
                            Status::Complete,
                        )));
                        return false;
                    }
                }
            }
            true
        });
    }

    fn send_update(&self) {
        let mut torrents = torrents!(self);
        let out = torrents
            .iter_mut()
            .map(|(info_hash, t)| self.create_info(info_hash, t, Status::Running))
            .collect();
        self.send
            .send(InfoMsg::All(out))
            .map_err(|e| error!("Info update failed with error: {:?}", e));
    }

    fn try_recv_new_peer(&mut self) {
        match self.recv.try_recv() {
            Ok(newpeer) => {
                self.peer_priority.push(newpeer.peer_id, newpeer.priority);
                self.peers.insert(newpeer.peer_id, newpeer.comm);
            }
            Err(err) => match err {
                mpsc::TryRecvError::Empty => {}
                mpsc::TryRecvError::Disconnected => {
                    info!("TCP listener thread has shut down. Controller now shutting down.");
                    return;
                }
            },
        }
    }

    /// Check messages from peers
    // A message can arrive between the point where we check the queue
    // and determine which pieces to request, but it's probably not critical
    // that this information is 100% up to date -
    // this determination will probably not happen whenever possible
    fn check_messages(&mut self) {
        // Can't have mut reference to self in closure
        let bitfields = &mut self.bitfields;
        let peer_priority = &mut self.peer_priority;
        let npeers = &mut self.npeers;
        self.peers.retain(|peer_id, comm| {
            let bitfield = bitfields.entry(peer_id.clone()).or_insert(BitVec::new());
            let disconnected =
                Controller::process_queue(&mut comm.try_iter(), &peer_id, bitfield, peer_priority);
            if disconnected {
                npeers.fetch_sub(1, AtomicOrdering::SeqCst);
                remove(peer_priority, peer_id);
            }
            !disconnected
        });
    }

    /// return true if disconnected; false otherwise
    fn process_queue(
        iter: &mut mpsc::TryIter<PeerUpdate>,
        peer_id: &Hash,
        bitfield: &mut BitVec,
        peer_priority: &mut PriorityQueue<Hash, PeerPriority>,
    ) -> bool {
        match iter.next() {
            Some(msg) => {
                let res = msg.process(peer_id, peer_priority, bitfield);
                res || Controller::process_queue(iter, peer_id, bitfield, peer_priority)
            }
            None => false,
        }
    }

    fn create_info(&self, info_hash: &Hash, torrent: &mut Torrent, status: Status) -> Info {
        Info {
            info_hash: info_hash.clone(),
            name: torrent.name().to_string(),
            status: status,
            progress: torrent.downloaded() as f32 / torrent.size() as f32,
            up: torrent.upload_rate(),
            down: torrent.download_rate(),
            npeers: self.npeers.load(AtomicOrdering::SeqCst),
        }
    }
}

#[cfg(test)]
mod tests {
    use bittorrent::manager::*;
    use env_logger;
    use bittorrent::tracker::tests::{default_tracker, run_server};
    use bittorrent::tracker::TrackerResponse;

    fn create_manager(port: u16) -> (Manager, mpsc::Receiver<InfoMsg>) {
        let mut manager = Manager::new().port(port);
        let receiver = manager.handle();
        return (manager, receiver);
    }

    #[test]
    fn test_send_receive() {
        let _ = env_logger::init();
        thread::spawn(move || {
            fn tmp() -> Vec<TrackerResponse> {
                default_tracker(&1776)
            }
            tmp;
            tests::run_server("127.0.0.1:3000", &tmp);
        });

        let (mut seeder, seeder_rx) = create_manager(1776);
        seeder.add_torrent(::TEST_FILE, ::DL_DIR);

        let (mut leecher, leecher_rx) = create_manager(1777);
        leecher.add_torrent(
            ::TEST_FILE,
            &format!("{}/{}", ::DL_DIR, "test_send_receive"),
        );

        loop {
            match leecher_rx.recv() {
                Ok(InfoMsg::One(info)) => match info.status {
                    Status::Complete => return,
                    _ => (),
                },
                _ => (),
            }
        }
    }

    #[test]
    fn test_controller() {
        // Create controller
        let (manager_send, manager_recv) = mpsc::channel();
        let torrents = Arc::new(Mutex::new(HashMap::new()));
        let npeers = Arc::new(AtomicUsize::new(0));
        let (info_send, info_recv) = mpsc::channel();

        let mut controller =
            Controller::new(manager_recv, torrents.clone(), npeers.clone(), info_send);

        // Add torrent
        let torrent = Torrent::new(::TEST_FILE.to_string(), ::DL_DIR.to_string()).unwrap();
        let npieces = torrent.npieces();

        // Simulate new peer
        let peer = Peer::new(PeerInfo {
            peer_id: Peer::gen_peer_id(),
            info_hash: torrent.info_hash(),
            ip: "127.0.0.1".to_string(),
            port: 3001,
        });
        let peer_comm = Manager::register_peer(&peer, npeers.clone(), &manager_send);

        {
            let mut ts = torrents.lock().unwrap();
            ts.insert(torrent.info_hash(), torrent);
        }

        controller.try_recv_new_peer();
        assert_eq!(controller.peer_priority.len(), 1);
        assert_eq!(controller.peers.len(), 1);

        // Send peer updates
        peer_comm.send(PeerUpdate::DownloadSpeed(100));
        peer_comm.send(PeerUpdate::InterestedChange);
        peer_comm.send(PeerUpdate::ChokingChange);
        peer_comm.send(PeerUpdate::Bitfield(BitVec::from_elem(npieces, true)));
        //peer_comm.send(PeerUpdate::Have(0));

        /*
        peer_interested: false,
        download_speed: 0,
        peer_choking: true,*/
        // Check that updates have been reflected
        {
            controller.check_messages();
            let top = controller.peer_priority.peek().unwrap();
            assert_eq!(top.1.peer_interested, true);
            assert_eq!(top.1.peer_choking, false);
            assert_eq!(top.1.download_speed, 100);
            assert_eq!(controller.bitfields[top.0][0], true);

            assert_eq!(controller.peer_priority.len(), 1);
            assert_eq!(controller.peers.len(), 1);
        }

        // Disconnect
        peer_comm.send(PeerUpdate::Disconnect);
        controller.check_messages();
        assert_eq!(controller.peer_priority.len(), 0);
        assert_eq!(controller.peers.len(), 0);
    }

    // Tests for worker?

}
