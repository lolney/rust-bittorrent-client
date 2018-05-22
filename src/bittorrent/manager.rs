extern crate core;

use bit_vec::BitVec;
use bittorrent::{peer::Action, peer::Peer, peer::PeerInfo, torrent,
                 torrent_runtime::TorrentRuntime, tracker::Tracker, Hash, ParseError, Piece};
use log::error;
use priority_queue::PriorityQueue;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::io::Error as IOError;
use std::io::{ErrorKind, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::ops::SubAssign;
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use std::sync::mpsc;
use std::sync::mpsc::SendError;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use std::usize::MAX;
use tokio_core::reactor::Core;
use tokio_timer::Interval;

use futures::prelude::Stream;

/// This module contains the main point of interaction with the library, `Manager`.
/// Calling `Manager::new().handle()` will spawn the `Controller` thread,
/// which chooses which pieces to request and which peers to upload to.
/// Each connection has a `Worker` task, which handles communication with a `Peer`,
/// reads and writes to disk, and sends relevant updates to the Controller.

pub struct Manager {
    torrents: Arc<Mutex<HashMap<Hash, TorrentRuntime>>>, // u8 is the Info_hash
    peer_id: Hash,                                       // our peer id
    npeers: Arc<AtomicUsize>, // TODO: can this just be in the controller?
    manager_send: Option<Sender<NewPeerMsg>>, // Must be initialized when Controller is created
    port: u16,                // Port we're listening on
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
    ($torrents:ident, $peer:ident, $torrent:ident) => {
        let mut torrents = $torrents.lock().expect("Torrents mutex poisoned");
        let hash = $peer.peer_info.info_hash;
        let mut $torrent = match torrents.get_mut(&hash) {
            Some(v) => Ok(v),
            None => Err(parse_error!("Torrent has been removed ")),
        }?;
    };
}

/// Instead of locking the peers data structure
/// and the priority queue on every access,
/// just send updates as needed.
/// We just lock for reads and writes, then,
/// which would be expensive to send anyway
#[derive(Debug, Clone, PartialEq)]
enum PeerUpdate {
    DownloadSpeed(usize),
    InterestedChange,
    ChokingChange,
    Have(u32),
    Downloaded(u32),
    Bitfield(BitVec),
    Disconnect,
}

impl Message for PeerUpdate {
    fn get_disconnected() -> Self {
        PeerUpdate::Disconnect
    }

    fn get_empty() -> Self {
        PeerUpdate::Disconnect
    }
}

#[derive(Debug, Clone, PartialEq)]
enum ManagerUpdate {
    Choke,
    Unchoke,
    Request(Vec<u8>),
    Have(u32),
    Disconnect,
    None,
}

impl Message for ManagerUpdate {
    fn get_disconnected() -> Self {
        ManagerUpdate::Disconnect
    }

    fn get_empty() -> Self {
        ManagerUpdate::None
    }
}

#[derive(PartialEq, Eq, Debug, Clone, Copy, Serialize, Deserialize)]
pub enum Status {
    Paused,
    Running,
    Complete,
}

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct Info {
    pub info_hash: Hash,
    pub name: String,
    pub status: Status,
    pub progress: f32,
    pub up: usize,
    pub down: usize,
    pub npeers: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InfoMsg {
    All(Vec<Info>),
    One(Info),
    Disconnect,
}

impl Message for InfoMsg {
    fn get_disconnected() -> Self {
        InfoMsg::Disconnect
    }

    fn get_empty() -> Self {
        InfoMsg::Disconnect
    }
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

#[derive(Debug, Clone)]
pub enum ClientMsg {
    Pause(Hash),
    Resume(Hash),
    Remove(Hash),
    Disconnect,
}

impl Message for ClientMsg {
    fn get_disconnected() -> Self {
        ClientMsg::Disconnect
    }

    fn get_empty() -> Self {
        ClientMsg::Disconnect
    }
}

pub struct NewPeerMsg {
    peer_id: Hash,
    info_hash: Hash,
    comm: BidirectionalChannel<ManagerUpdate, PeerUpdate>,
    priority: PeerPriority,
}

pub trait Message {
    fn get_disconnected() -> Self;
    fn get_empty() -> Self;
}

#[derive(Debug)]
pub struct BidirectionalChannel<S, R>
where
    S: Message,
    R: Message,
{
    send: mpsc::Sender<S>,
    recv: mpsc::Receiver<R>,
}

impl<S, R> BidirectionalChannel<S, R>
where
    S: Message,
    R: Message,
{
    pub fn create() -> (BidirectionalChannel<R, S>, BidirectionalChannel<S, R>) {
        let (s_s, s_r) = mpsc::channel();
        let (r_s, r_r) = mpsc::channel();

        (
            BidirectionalChannel {
                send: s_s,
                recv: r_r,
            },
            BidirectionalChannel {
                send: r_s,
                recv: s_r,
            },
        )
    }

    pub fn send(&self, update: S) -> Result<(), SendError<S>> {
        self.send.send(update)
    }

    pub fn recv(&self) -> R {
        match self.recv.try_recv() {
            Ok(update) => update,
            Err(err) => match err {
                mpsc::TryRecvError::Empty => R::get_empty(),
                mpsc::TryRecvError::Disconnected => R::get_disconnected(),
            },
        }
    }

    pub fn try_iter(&self) -> mpsc::TryIter<R> {
        self.recv.try_iter()
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

    /// Add a completed torrent specified by file `metainfo_path`,
    /// where the file/directory mentioned in that file is present in `download_path`
    pub fn add_completed_torrent(
        &mut self,
        metainfo_path: &str,
        download_path: &str,
    ) -> Result<(), ParseError> {
        match TorrentRuntime::new(metainfo_path, download_path, true) {
            Ok(torrent) => self._add_torrent(torrent),
            Err(err) => Err(err),
        }
    }

    /// Add a new torrent from file `metainfo_path`, downloading to `download_path`
    pub fn add_torrent(
        &mut self,
        metainfo_path: &str,
        download_path: &str,
    ) -> Result<(), ParseError> {
        match TorrentRuntime::new(metainfo_path, download_path, false) {
            Ok(torrent) => {
                torrent.create_files()?;
                self._add_torrent(torrent)
            }
            Err(err) => Err(err),
        }
    }

    fn _add_torrent(&mut self, torrent: TorrentRuntime) -> Result<(), ParseError> {
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
        torrents: &Arc<Mutex<HashMap<Hash, TorrentRuntime>>>,
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
                let torrents = torrents.lock().expect("Torrents mutex poisoned");
                let torrent = torrents.get(&info.info_hash).unwrap();
                let peer = Peer::new(info, torrent.npieces());
                Ok(peer)
            }
        }
    }

    fn match_torrent(
        info_hash: &Hash,
        torrents: &Arc<Mutex<HashMap<Hash, TorrentRuntime>>>,
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

    fn update_priority_all(peer: &Peer, torrent: &mut TorrentRuntime, by: isize) {
        for (i, has) in peer.bitfield.iter().enumerate() {
            if has {
                torrent.update_priority(i, by);
            }
        }
    }

    /// Writes to error log and to tcpstream
    /// Exchanges messages with the manager
    fn receive(
        mut peer: Peer,
        mut stream: TcpStream,
        torrents: Arc<Mutex<HashMap<Hash, TorrentRuntime>>>,
        comm: BidirectionalChannel<PeerUpdate, ManagerUpdate>,
    ) -> Result<(), ParseError> {
        let mut buf = [0; ::MSG_SIZE];
        let mut download_size = 0;

        stream
            .set_read_timeout(Some(Duration::new(::READ_TIMEOUT, 0)))
            .unwrap_or_else(|e| error!("Failed to set read timeout on stream: {}", e));

        stream
            .set_nonblocking(true)
            .expect("set_nonblocking call failed");

        // Begin by sending our bitfield
        {
            acquire_torrent_lock!(torrents, peer, torrent);
            if let Err(e) = stream.write(&Peer::bitfield(&torrent.bitfield())) {
                error!("Error sending initial bitfield to peer {}", peer.peer_id());
            }
        }

        loop {
            // match incoming messages from the peer
            match stream.read(&mut buf) {
                Err(e) => {
                    match e.kind() {
                        ErrorKind::WouldBlock => {
                            /*
                            // for each field in bitfield, update_priority by -1
                            if peer.peer_choking {
                                acquire_torrent_lock!(torrents, peer, torrent);
                                Manager::update_priority_all(&peer, &mut torrent, -1);
                            }

                            comm.send(PeerUpdate::Disconnect);
                            return;*/
                        } // timeout
                        _ => error!("Error while reading from stream: {}", e),
                    }
                }
                Ok(n) => match peer.parse_message(&buf[0..n]) {
                    Ok(action) => match action {
                        Action::Request(requests) => {
                            info!("Fullfilling request for peer {}", peer.peer_id());
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
                            info!(
                                "Writing piece from peer {}: {:?}",
                                peer.peer_id(),
                                piece.piece
                            );
                            download_size = download_size + piece.piece.length;
                            acquire_torrent_lock!(torrents, peer, torrent);
                            match torrent.write_block(&piece) {
                                Ok(()) => {
                                    comm.send(PeerUpdate::Downloaded(piece.piece.index));
                                }
                                Err(e) => error!("Error while writing piece to file: {}", e),
                            }
                        }
                        Action::InterestedChange => {
                            trace!(
                                "Peer {} interested status now: {:?}",
                                peer.peer_id(),
                                peer.peer_interested
                            );
                            comm.send(PeerUpdate::InterestedChange);
                        }
                        Action::ChokingChange => {
                            info!("Received choking change");
                            acquire_torrent_lock!(torrents, peer, torrent);
                            if peer.peer_choking {
                                Manager::update_priority_all(&peer, &mut torrent, -1);
                            } else {
                                Manager::update_priority_all(&peer, &mut torrent, 1);
                            }
                            comm.send(PeerUpdate::ChokingChange);
                        }
                        Action::Have(index) => {
                            if peer.peer_choking {
                                acquire_torrent_lock!(torrents, peer, torrent);
                                torrent.update_priority(index as usize, 1);
                            }
                            comm.send(PeerUpdate::Have(index));
                        }
                        Action::Bitfield(bitfield) => {
                            if peer.peer_choking {
                                acquire_torrent_lock!(torrents, peer, torrent);
                                Manager::update_priority_all(&peer, &mut torrent, 1);
                            }
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
                    info!("Sending request");
                    stream.write(req.as_slice());
                }
                ManagerUpdate::Choke => {
                    if !peer.am_choking {
                        stream.write(peer.choke(true).as_slice());
                    }
                }
                ManagerUpdate::Unchoke => {
                    if peer.am_choking {
                        info!("Unchoking peer {}", peer.peer_id());
                        stream.write(peer.choke(false).as_slice());
                    }
                }
                ManagerUpdate::Disconnect => {
                    info!(
                        "Manager has ordered disconnect from peer {}",
                        peer.peer_id()
                    );
                    return Ok(());
                }
                ManagerUpdate::Have(index) => {
                    stream.write(Peer::have(index).as_slice());
                }
                ManagerUpdate::None => {}
            }
        }
    }

    fn register_peer(
        peer: &Peer,
        npeers: Arc<AtomicUsize>,
        manager_send: &Sender<NewPeerMsg>,
    ) -> BidirectionalChannel<PeerUpdate, ManagerUpdate> {
        npeers.fetch_add(1, AtomicOrdering::SeqCst);
        let (manager_comm, peer_comm) = BidirectionalChannel::create();

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
        torrents: Arc<Mutex<HashMap<Hash, TorrentRuntime>>>,
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
                                if let Err(e) = Manager::receive(peer, stream, torrents, peer_comm)
                                {
                                    warn!("Peer shut down with error: {}", e);
                                }
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
    pub fn handle(&mut self) -> BidirectionalChannel<ClientMsg, InfoMsg> {
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
        let (info_send, info_recv) = BidirectionalChannel::create();
        thread::spawn(move || {
            Controller::new(manager_recv, torrents, npeers, info_send).run_loop();
        });

        return info_recv;
    }
}

macro_rules! torrents {
    ($self:ident) => {
        $self.torrents.lock().expect("Torrents mutex poisoned");
    };
}

/// Removes the peer specified by `peer_id`.
/// This is a macro to avoid annoying the borrow checker.
macro_rules! remove_peer {
    ($self:ident, $peer_id:ident) => {
        $self.npeers.fetch_sub(1, AtomicOrdering::SeqCst);
        remove(&mut $self.peer_priority, $peer_id);
        $self.bitfields.remove($peer_id);
        $self.peers.remove($peer_id);
    };
}

struct Controller {
    recv: mpsc::Receiver<NewPeerMsg>,
    torrents: Arc<Mutex<HashMap<Hash, TorrentRuntime>>>,
    npeers: Arc<AtomicUsize>,
    client_comm: BidirectionalChannel<InfoMsg, ClientMsg>,
    peer_priority: PriorityQueue<Hash, PeerPriority>,
    peers: HashMap<Hash, BidirectionalChannel<ManagerUpdate, PeerUpdate>>,
    bitfields: HashMap<Hash, BitVec>,
}

struct Timers<F>
where
    F: Fn() -> bool,
{
    timers: Vec<Timer<F>>,
    base: Duration,
}

// TODO: macro
// Input: (function and frequency) pairs
// Generate code
/*
{

}
*/
impl<F> Timers<F>
where
    F: Fn() -> bool,
{
    fn new(base: Duration) -> Timers<F> {
        Timers {
            timers: Vec::new(),
            base: base,
        }
    }

    pub fn add(&mut self, duration: Duration, task: F) -> Result<(), &'static str> {
        if duration.as_secs() % self.base.as_secs() != 0 {
            Err("Duration must be a multiple of base")
        } else {
            self.timers.push(Timer::new(duration, task));
            Ok(())
        }
    }

    pub fn run_loop(&mut self) {
        loop {
            thread::sleep(self.base);
            if self.tick() {
                break;
            }
        }
    }

    fn tick(&mut self) -> bool {
        for timer in self.timers.iter_mut() {
            if timer.tick(self.base) {
                return true;
            }
        }
        return false;
    }
}

struct Timer<F>
where
    F: Fn() -> bool,
{
    base: Duration,
    remaining: Duration,
    task: F,
}

impl<F> Timer<F>
where
    F: Fn() -> bool,
{
    fn new(duration: Duration, task: F) -> Timer<F> {
        Timer {
            base: duration,
            remaining: duration,
            task: task,
        }
    }

    pub fn tick(&mut self, delta: Duration) -> bool {
        self.remaining.sub_assign(self.base);

        if self.remaining <= Duration::from_secs(0) {
            self.remaining = self.base;
            return (self.task)();
        } else {
            return false;
        }
    }
}

impl Controller {
    pub fn new(
        recv: mpsc::Receiver<NewPeerMsg>,
        torrents: Arc<Mutex<HashMap<Hash, TorrentRuntime>>>,
        npeers: Arc<AtomicUsize>,
        client_comm: BidirectionalChannel<InfoMsg, ClientMsg>,
    ) -> Controller {
        Controller {
            recv: recv,
            torrents: torrents,
            npeers: npeers,
            client_comm: client_comm,
            peer_priority: PriorityQueue::new(),
            peers: HashMap::new(),
            bitfields: HashMap::new(),
        }
    }

    // TODO: This is a strong candidate to be replaced by a task system;
    // Task: returns Option<Update>
    pub fn run_loop(&mut self) {
        let mut start = Instant::now();
        let ten_secs = Duration::from_secs(2);

        self.choke();
        loop {
            if self.recv_client() {
                break;
            }

            /// Send updates at 1-second interval
            self.send_update();

            /// Receive messages sent when a new peer is added
            self.try_recv_new_peer();

            self.check_messages();

            // Update choking on 10-sec intervals
            if start.elapsed() >= ten_secs {
                self.choke();
                self.make_requests();
                start = Instant::now();
            }
        }
    }

    fn filter_priority<F>(&self, filter: F) -> Vec<Hash>
    where
        F: FnMut(&(&Hash, &PeerPriority)) -> bool,
    {
        self.peer_priority
            .iter()
            .filter(filter)
            .map(|(id, _)| id.clone())
            .collect()
    }

    fn recv_client(&mut self) -> bool {
        for msg in self.client_comm.try_iter() {
            match msg {
                ClientMsg::Pause(info_hash) => {
                    let mut torrents = torrents!(self);
                    match torrents.get_mut(&info_hash) {
                        Some(ref mut torrent) => {
                            if torrent.pause(true) {
                                info!("Paused torrent")
                            } else {
                                error!("Torrent already paused");
                            }
                        }
                        None => {
                            error!("Received request to pause torrent that does not exist");
                        }
                    }
                }
                ClientMsg::Resume(info_hash) => {
                    let mut torrents = torrents!(self);
                    match torrents.get_mut(&info_hash) {
                        Some(ref mut torrent) => {
                            if torrent.pause(false) {
                                info!("Resumed torrent")
                            } else {
                                error!("Torrent already running");
                            }
                        }
                        None => {
                            error!("Received request to resume torrent that does not exist");
                        }
                    }
                }
                ClientMsg::Remove(info_hash) => {
                    let mut torrents = torrents!(self);
                    if let Some(_) = torrents.remove(&info_hash) {
                        let ids = self.filter_priority(|&(_, peer)| peer.info_hash == info_hash);
                        for peer_id in ids.iter() {
                            self.peers
                                .get_mut(peer_id)
                                .expect("Peer should be in peers list")
                                .send(ManagerUpdate::Disconnect);
                            remove_peer!(self, peer_id);
                        }
                    } else {
                        error!("Tried to remove torrent that does not exist");
                    }
                }
                ClientMsg::Disconnect => {
                    return true;
                }
            }
        }
        return false;
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
        for (id, _priority) in self.peer_priority.clone().into_sorted_iter() {
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
        for (info_hash, mut torrent) in torrents
            .iter_mut()
            .filter(|t| t.1.status() == Status::Running)
        {
            // Select eligible peers
            let peers_iter: Vec<Hash> = self.peer_priority
                .iter()
                .filter(|&(k, v)| !v.peer_choking && v.info_hash == *info_hash)
                .map(|(k, v)| k.clone())
                .collect(); // collect to be able to reuse

            while torrent.nrequests() < ::REQUESTS_LIMIT {
                match torrent.select_piece() {
                    Some(piece) => {
                        for id in peers_iter.iter() {
                            if self.bitfields
                                .get(id)
                                .expect("Peer in priority queue but not bitfields")
                                .get(piece.index as usize)
                                .expect("Torrent bitfield and controller bitfields don't match")
                            {
                                // Request piece
                                let comm = self.peers
                                    .get(id)
                                    .expect("Peer in priority queue but not comms map");
                                comm.send(ManagerUpdate::Request(Peer::request(&piece)))
                                    .unwrap();
                                info!("Requesting piece {:?} from peer {}", piece, id);
                                break;
                            }
                        }
                    }
                    None => {
                        trace!("Exhausted possible requests for torrent {}", torrent.name());
                        break;
                    }
                }
            }
        }
    }

    fn send_update(&self) {
        let mut torrents = torrents!(self);
        let out = torrents
            .iter_mut()
            .map(|(info_hash, t)| self.create_info(info_hash, t))
            .collect();
        if let Err(e) = self.client_comm.send(InfoMsg::All(out)) {
            error!("Info update failed with error: {}", e);
        }
    }

    fn try_recv_new_peer(&mut self) {
        loop {
            match self.recv.try_recv() {
                Ok(newpeer) => {
                    let mut torrents = torrents!(self);
                    let mut torrent = torrents.get(&newpeer.info_hash).unwrap();

                    self.peer_priority.push(newpeer.peer_id, newpeer.priority);
                    self.peers.insert(newpeer.peer_id, newpeer.comm);
                    self.bitfields
                        .insert(newpeer.peer_id, BitVec::from_elem(torrent.npieces(), false));
                }
                Err(err) => match err {
                    mpsc::TryRecvError::Empty => {
                        return;
                    }
                    mpsc::TryRecvError::Disconnected => {
                        info!("TCP listener thread has shut down. Controller now shutting down.");
                        return;
                    }
                },
            }
        }
    }

    /// Check messages from peers
    // A message can arrive between the point where we check the queue
    // and determine which pieces to request, but it's probably not critical
    // that this information is 100% up to date -
    // this determination will probably not happen whenever possible
    fn check_messages(&mut self) {
        // Can't have mut reference to self in closure
        let mut messages = Vec::new();

        for (peer_id, mut comm) in self.peers.iter_mut() {
            let mut iter = comm.try_iter();
            loop {
                match iter.next() {
                    Some(msg) => {
                        messages.push((peer_id.clone(), msg.clone()));
                    }
                    None => {
                        break;
                    }
                }
            }
        }

        for (peer_id, msg) in messages {
            self.process(&peer_id, &msg);
        }
    }

    /// Process msg, assuming inputs are valid
    fn process(&mut self, peer_id: &Hash, msg: &PeerUpdate) {
        match msg {
            &PeerUpdate::DownloadSpeed(speed) => {
                self.peer_priority
                    .change_priority_by(peer_id, |priority| priority.set_download(speed));
            }
            &PeerUpdate::InterestedChange => {
                self.peer_priority
                    .change_priority_by(peer_id, PeerPriority::flip_interested);
            }
            &PeerUpdate::ChokingChange => {
                self.peer_priority
                    .change_priority_by(peer_id, PeerPriority::flip_choking);
            }
            &PeerUpdate::Have(piece_index) => {
                let mut bitfield = self.bitfields.get_mut(&peer_id).unwrap();
                bitfield.set(piece_index as usize, true);
            }
            &PeerUpdate::Downloaded(index) => {
                for (_, comm) in self.peers.iter_mut() {
                    comm.send(ManagerUpdate::Have(index));
                }
                let mut torrents = torrents!(self);
                let info_hash = self.peer_priority.get(peer_id).unwrap().1.info_hash;
                let mut torrent = torrents.get_mut(&info_hash).unwrap();
                let remaining = torrent.remaining();
                info!("{} bytes remaining", remaining);
                if remaining == 0 {
                    info!("Torrent \"{}\" complete", torrent.name());
                    match torrent.set_complete() {
                        Ok(()) => {
                            self.client_comm
                                .send(InfoMsg::One(self.create_info(&info_hash, &mut torrent)));
                        }
                        Err(e) => {
                            error!("Error while setting torrent as complete: {}", e);
                        }
                    }
                }
            }
            &PeerUpdate::Bitfield(ref _bitfield) => {
                let mut bitfield = self.bitfields.get_mut(&peer_id).unwrap();
                if bitfield.len() == _bitfield.len() {
                    bitfield.union(_bitfield);
                } else {
                    panic!(
                        "PeerUpdate sent invalid bitfield: {}, {}",
                        bitfield.len(),
                        _bitfield.len()
                    );
                }
            }
            &PeerUpdate::Disconnect => {
                remove_peer!(self, peer_id);
            }
        }
    }

    fn create_info(&self, info_hash: &Hash, torrent: &mut TorrentRuntime) -> Info {
        Info {
            info_hash: info_hash.clone(),
            name: torrent.name().to_string(),
            status: torrent.status(),
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
    use bittorrent::tracker::tests::{default_tracker, run_server};
    use bittorrent::tracker::TrackerResponse;
    use env_logger;

    /// Allows access to all these variables without copy-pasting the setup every time
    macro_rules! controller_setup {
        (
            $controller:ident, $manager_send:ident, $npeers:ident, $info_hash:ident, $npieces:ident
        ) => {
            controller_setup!(
                $controller,
                $manager_send,
                $npeers,
                $info_hash,
                $npieces,
                client,
                torrents
            );
        };
        (
            $controller:ident,
            $manager_send:ident,
            $npeers:ident,
            $info_hash:ident,
            $npieces:ident,
            $client:ident,
            $torrents:ident
        ) => {
            let ($manager_send, manager_recv) = mpsc::channel();
            let $torrents = Arc::new(Mutex::new(HashMap::new()));
            let $npeers = Arc::new(AtomicUsize::new(0));
            let (info_send, $client) = BidirectionalChannel::create();

            let mut $controller =
                Controller::new(manager_recv, $torrents.clone(), $npeers.clone(), info_send);

            // Add torrent
            let torrent = TorrentRuntime::new(::TEST_FILE, ::DL_DIR, false).unwrap();
            let $npieces = torrent.npieces();

            let $info_hash = torrent.info_hash().clone();
            let npieces = torrent.npieces();
            {
                let mut ts = $torrents.lock().unwrap();
                ts.insert(torrent.info_hash(), torrent);
            }
        };
    }

    macro_rules! get_torrent {
        ($torrents:ident, $info_hash:expr, $torrent:ident) => {
            let mut ts = $torrents.lock().unwrap();
            let mut $torrent = ts.get($info_hash).unwrap();
        };
    }

    fn create_manager(port: u16) -> (Manager, BidirectionalChannel<ClientMsg, InfoMsg>) {
        let mut manager = Manager::new().port(port);
        let comm = manager.handle();
        return (manager, comm);
    }

    #[test]
    fn test_client_msgs() {
        controller_setup!(
            controller,
            manager_send,
            npeers,
            info_hash,
            npieces,
            client,
            torrents
        );
        // Send pause; make sure torrent is paused
        client.send(ClientMsg::Pause(info_hash));
        controller.recv_client();
        {
            get_torrent!(torrents, &info_hash, torrent);
            assert_eq!(torrent.status(), Status::Paused);
        }
        // Send resume; make sure it's resumed
        client.send(ClientMsg::Resume(info_hash));
        controller.recv_client();
        {
            get_torrent!(torrents, &info_hash, torrent);
            assert_eq!(torrent.status(), Status::Running);
        }
        // Send resume again; still resumed
        client.send(ClientMsg::Resume(info_hash));
        controller.recv_client();
        {
            get_torrent!(torrents, &info_hash, torrent);
            assert_eq!(torrent.status(), Status::Running);
        }
        // Remove it
        client.send(ClientMsg::Remove(info_hash));
        controller.recv_client();
        {
            let mut ts = torrents.lock().unwrap();
            assert_eq!(ts.get(&info_hash), None);
        }
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
        seeder.add_completed_torrent(::TEST_FILE, &format!("{}/{}", ::READ_DIR, "valid_torrent"));

        let (mut leecher, leecher_rx) = create_manager(1777);
        leecher.add_torrent(
            ::TEST_FILE,
            &format!("{}/{}", ::DL_DIR, "test_send_receive"),
        );

        loop {
            match leecher_rx.recv() {
                InfoMsg::One(info) => match info.status {
                    Status::Complete => {
                        leecher_rx.send(ClientMsg::Disconnect);
                        seeder_rx.send(ClientMsg::Disconnect);
                        return;
                    }
                    _ => (),
                },
                _ => (),
            }
        }
    }

    // Create n seeding clients
    // Create n leeching clients

    // Create seeder
    // Create leecher. Suspend leecher, then reload

    fn gen_peer(port: i64, info_hash: Hash) -> Peer {
        Peer::new(
            PeerInfo {
                peer_id: Peer::gen_peer_id(),
                info_hash: info_hash,
                ip: "127.0.0.1".to_string(),
                port: port,
            },
            1,
        )
    }

    #[test]
    fn test_peer_updates() {
        controller_setup!(controller, manager_send, npeers, info_hash, npieces);
        // Simulate new peer
        let peer = gen_peer(3001, info_hash);
        let peer_comm = Manager::register_peer(&peer, npeers.clone(), &manager_send);

        controller.try_recv_new_peer();
        assert_eq!(controller.peer_priority.len(), 1);
        assert_eq!(controller.peers.len(), 1);
        /* end setup */

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

    fn test_worker() {}

    #[test]
    fn test_make_requests() {
        // Setup
        controller_setup!(controller, manager_send, npeers, info_hash, npieces);
        // Connect some peers (mock)
        let peer = gen_peer(3002, info_hash);
        let peer_comm = Manager::register_peer(&peer, npeers.clone(), &manager_send);
        let peer_2 = gen_peer(3003, info_hash);
        let peer_comm_2 = Manager::register_peer(&peer_2, npeers.clone(), &manager_send);
        controller.try_recv_new_peer();

        // Some of those peers stop choking
        assert_eq!(controller.bitfields[peer_2.peer_id()][0], false);
        peer_comm_2.send(PeerUpdate::ChokingChange);
        peer_comm_2.send(PeerUpdate::Have(0));
        controller.check_messages();
        assert_eq!(controller.bitfields[peer_2.peer_id()][0], true);

        // Run make_requests()
        {
            let mut ts = controller.torrents.lock().unwrap();
            let mut torrent = ts.get_mut(&info_hash).unwrap();
            torrent.update_priority(0, 1);
        }
        controller.make_requests();
        // Make sure those peers get requests
        assert_eq!(peer_comm.recv(), ManagerUpdate::None);
        if let ManagerUpdate::Request(req) = peer_comm_2.recv() {
            assert!(true);
        } else {
            assert!(false);
        }
        assert_eq!(peer_comm_2.recv(), ManagerUpdate::None);
    }

    fn test_update_priority() {
        // Try receiving some `have`s and getting some disconnects with mock peer
        // Make sure priority is updated
    }

}
