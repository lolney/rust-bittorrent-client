extern crate core;

use bit_vec::BitVec;

use bittorrent::{
    metainfo::MetaInfo, peer::Action, peer::Peer, peer::PeerInfo, timers::Timers, torrent,
    torrent_runtime::TorrentRuntime, tracker::Tracker, Hash, ParseError, Piece,
};
use log::error;
use priority_queue::PriorityQueue;
use std::cmp::Ord;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::io::Error as IOError;
use std::io::{ErrorKind, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use std::sync::mpsc;
use std::sync::mpsc::SendError;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use std::usize::MAX;
use tokio_core::reactor::Core;

use futures::prelude::Stream;

/// This module contains the main point of interaction with the library, `Manager`.
/// Calling `Manager::new().handle()` will spawn the `Controller` thread,
/// which chooses which pieces to request and which peers to upload to.
/// Each connection has a `Worker` task, which handles communication with a `Peer`,
/// reads and writes to disk, and sends relevant updates to the Controller.
pub struct Manager {
    torrent_state: Arc<TorrentState>,
    manager_send: Option<Sender<NewPeerMsg>>, // Must be initialized when Controller is created
    port: u16,                                // Port we're listening on
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
    ($state:ident, $peer:ident, $torrent:ident) => {
        let mut torrents = $state.torrents.lock().expect("Torrents mutex poisoned");
        let hash = $peer.peer_info.info_hash;
        let mut $torrent = match torrents.get_mut(&hash) {
            Some(v) => Ok(v),
            None => Err(parse_error!("Torrent has been removed ")),
        }?;
    };
}

macro_rules! torrents {
    ($self:ident) => {
        $self
            .torrent_state
            .torrents
            .lock()
            .expect("Torrents mutex poisoned");
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

impl Ord for Info {
    fn cmp(&self, other: &Info) -> Ordering {
        self.info_hash.cmp(&other.info_hash)
    }
}

impl PartialOrd for Info {
    fn partial_cmp(&self, other: &Info) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Eq for Info {}

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

struct TorrentState {
    torrents: Mutex<HashMap<Hash, TorrentRuntime>>, // u8 is the Info_hash
    peer_id: Hash,                                  // our peer id
    npeers: AtomicUsize,
}

impl TorrentState {
    fn new() -> TorrentState {
        TorrentState {
            torrents: Mutex::new(HashMap::new()),
            peer_id: Peer::gen_peer_id(),
            npeers: AtomicUsize::new(0),
        }
    }
}

struct TrackerArgs {
    torrent_state: Arc<TorrentState>,
    port: u16,
    channel: Sender<NewPeerMsg>,
    torrent: MetaInfo,
}

impl TrackerArgs {
    pub fn query_tracker_on_torrents(&self) {
        for torrent in torrents!(self).iter() {
            Manager::query_tracker(self);
        }
    }
}

impl Manager {
    pub fn new() -> Manager {
        Manager {
            torrent_state: Arc::new(TorrentState::new()),
            manager_send: None,
            port: ::PORT_NUM,
        }
    }

    pub fn peer_id(&self) -> Hash {
        self.torrent_state.peer_id
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
            Ok(torrent) => {
                let metainfo = self.insert_torrent(torrent);
                Manager::query_tracker(&self.tracker_args(metainfo))
            }
            Err(err) => Err(err),
        }
    }

    /// Add a new torrent from file `metainfo_path`, downloading to `download_path`.
    /// Note: will not be reflected in the Controller until it's scheduled to check for new NewPeerMsgs
    pub fn add_torrent(
        &mut self,
        metainfo_path: &str,
        download_path: &str,
    ) -> Result<(), ParseError> {
        match TorrentRuntime::new(metainfo_path, download_path, false) {
            Ok(torrent) => {
                torrent.create_files()?;
                let metainfo = self.insert_torrent(torrent);
                Manager::query_tracker(&self.tracker_args(metainfo))
            }
            Err(err) => Err(err),
        }
    }

    fn insert_torrent(&mut self, torrent: TorrentRuntime) -> MetaInfo {
        let metainfo = torrent.metainfo();
        {
            let mut torrents = torrents!(self);
            let info_hash = torrent.info_hash();
            torrents.insert(info_hash, torrent);
        }
        info!(
            "Added torrent {}. Info hash: {}",
            metainfo.name(),
            metainfo.info_hash()
        );
        metainfo
    }

    fn tracker_args(&self, torrent: MetaInfo) -> TrackerArgs {
        let torrent_state = self.torrent_state.clone();
        let port = self.port;
        let channel = self
            .manager_send
            .clone()
            .expect("Handle not called on manager");
        TrackerArgs {
            torrent_state: torrent_state,
            port: port,
            channel: channel,
            torrent: torrent,
        }
    }

    fn query_tracker(args: &TrackerArgs) -> Result<(), ParseError> {
        let mut core = Core::new()?;

        let info_hash = args.torrent.info_hash();
        let stream = Tracker::get_peers(
            core.handle(),
            info_hash,
            args.torrent_state.peer_id,
            args.torrent.trackers(),
            args.port,
        )?;

        core.run(stream.for_each(|ips| {
            info!("Tracker response received with {} ips", ips.len());
            for ip in ips {
                debug!("Received IP address from tracker: {}", ip);
                if ip.port() == args.port {
                    warn!("Avoiding connection with self: {}", args.port);
                    continue;
                }
                let channel = args.channel.clone();

                ConnectionHandler::connect(
                    TcpStream::connect(ip),
                    args.torrent_state.clone(),
                    channel,
                    Handshake::Initiator {
                        peer_id: args.torrent_state.peer_id,
                        info_hash: info_hash,
                    },
                );
            }
            Ok(())
        }))
        .map_err(|e| {
            error!("Error while announcing to tracker: {:?}", e);
            e
        })
    }

    /* TODO: query tracker on interval
    fn repeat_query_tracker(&self) {
        let tracker_args = self.tracker_args();
        thread::spawn(move || {
            let mut wheel = Timers::new(Duration::new(10, 0), tracker_args);
            add_timers!(wheel, TrackerArgs, (10, query_tracker_on_torrents));
            wheel.run_loop();
        });
    }*/

    /// Entry point. Only meant to be called once.
    /// Returns channel for receiving progress updates.
    pub fn handle(&mut self) -> BidirectionalChannel<ClientMsg, InfoMsg> {
        let (manager_send, manager_recv) = mpsc::channel();
        self.manager_send = Some(manager_send.clone());

        let torrent_state_1 = self.torrent_state.clone();
        let port = self.port.clone();

        thread::spawn(move || {
            // listens for incoming connections
            let listener = TcpListener::bind(format!("127.0.0.1:{}", port))
                .expect(&format!("Failed to bind listening port: {}", port));
            info!(
                "Listening on port {} for new peers as peer {}",
                port, torrent_state_1.peer_id
            );
            listener.incoming().for_each(|stream| {
                ConnectionHandler::connect(
                    stream,
                    torrent_state_1.clone(),
                    manager_send.clone(),
                    Handshake::Receiver(torrent_state_1.peer_id.clone()),
                );
            });
        });

        //self.repeat_query_tracker();

        let torrent_state_2 = self.torrent_state.clone();
        let (info_send, info_recv) = BidirectionalChannel::create();
        thread::spawn(move || {
            info!("Starting server");
            Controller::new(manager_recv, torrent_state_2, info_send).run_loop();
        });

        return info_recv;
    }
}

struct ConnectionHandler {
    torrent_state: Arc<TorrentState>,
}

impl ConnectionHandler {
    /// Entry point
    pub fn connect(
        stream: Result<TcpStream, IOError>,
        torrent_state: Arc<TorrentState>,
        manager_send: Sender<NewPeerMsg>,
        handshake: Handshake,
    ) {
        let handler = ConnectionHandler {
            torrent_state: torrent_state,
        };
        handler.do_connect(stream, manager_send, handshake);
    }

    /// Handle incoming clients
    fn handle_client(
        &self,
        stream: &mut TcpStream,
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
                        self.match_torrent(&their_info_hash)?;
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
                let torrents = torrents!(self);
                let torrent = torrents.get(&info.info_hash).unwrap();
                let peer = Peer::new(info, torrent.npieces());
                Ok(peer)
            }
        }
    }

    fn match_torrent(&self, info_hash: &Hash) -> Result<bool, ParseError> {
        // Find the matching torrent
        let torrents = torrents!(self);
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
        torrent_state: Arc<TorrentState>,
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
            acquire_torrent_lock!(torrent_state, peer, torrent);
            if let Err(e) = stream.write(&Peer::bitfield(&torrent.bitfield())) {
                error!("Error sending initial bitfield to peer {}", peer.peer_id());
            }
        }

        loop {
            // Alternatives:
            // Use mio for cross-platform epoll-like scheduling
            //      - Least change from the current model, but not ideal
            // Use tokio for task management:
            // tokio::spawn(reader.for_each(|frame| {process(frame)}));
            //      - Can also move channels to futures::sync::mpsc::channel
            thread::sleep(Duration::from_micros(20000));

            // match incoming messages from the peer
            match stream.read(&mut buf) {
                Err(e) => match e.kind() {
                    ErrorKind::WouldBlock => {}
                    _ => error!("Error while reading from stream: {}", e),
                },
                Ok(n) => match peer.parse_message(&buf[0..n]) {
                    Ok(action) => match action {
                        Action::EOF => {
                            warn!(
                                "Received empty message from {}; disconnecting",
                                peer.peer_id()
                            );
                            // for each field in bitfield, update_priority by 1
                            if peer.peer_choking {
                                acquire_torrent_lock!(torrent_state, peer, torrent);
                                ConnectionHandler::update_priority_all(&peer, &mut torrent, -1);
                            }
                            comm.send(PeerUpdate::Disconnect).unwrap();
                            return Ok(());
                        }
                        Action::Request(requests) => {
                            info!("Fullfilling request for peer {}", peer.peer_id());
                            acquire_torrent_lock!(torrent_state, peer, torrent);
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
                            acquire_torrent_lock!(torrent_state, peer, torrent);
                            match torrent.write_block(&piece) {
                                Ok(()) => {
                                    comm.send(PeerUpdate::Downloaded(piece.piece.index));
                                }
                                Err(e) => error!("Error while writing piece to file: {}", e),
                            }
                        }
                        Action::InterestedChange => {
                            info!(
                                "Peer {} interested status now: {:?}",
                                peer.peer_id(),
                                !peer.peer_interested
                            );
                            comm.send(PeerUpdate::InterestedChange);
                        }
                        Action::ChokingChange => {
                            info!(
                                "Peer {} choking us now: {:?}",
                                peer.peer_id(),
                                peer.peer_choking
                            );
                            acquire_torrent_lock!(torrent_state, peer, torrent);
                            if peer.peer_choking {
                                ConnectionHandler::update_priority_all(&peer, &mut torrent, -1);
                            } else {
                                ConnectionHandler::update_priority_all(&peer, &mut torrent, 1);
                            }
                            comm.send(PeerUpdate::ChokingChange);
                        }
                        Action::Have(index) => {
                            if peer.peer_choking {
                                acquire_torrent_lock!(torrent_state, peer, torrent);
                                torrent.update_priority(index as usize, 1);
                            }
                            comm.send(PeerUpdate::Have(index));
                        }
                        Action::Bitfield(bitfield) => {
                            info!("Received bitfield from {}", peer.peer_id());
                            if !peer.peer_choking {
                                acquire_torrent_lock!(torrent_state, peer, torrent);
                                ConnectionHandler::update_priority_all(&peer, &mut torrent, 1);
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
                        info!("Choking peer {}", peer.peer_id());
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
                        "Disconnected from manager. Shutting down peer {}",
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
        &self,
        peer: &Peer,
        manager_send: &Sender<NewPeerMsg>,
    ) -> BidirectionalChannel<PeerUpdate, ManagerUpdate> {
        self.torrent_state
            .npeers
            .fetch_add(1, AtomicOrdering::SeqCst);
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

    fn do_connect(
        &self,
        stream: Result<TcpStream, IOError>,
        manager_send: Sender<NewPeerMsg>,
        handshake: Handshake,
    ) {
        match stream {
            Ok(mut stream) => {
                if self.torrent_state.npeers.load(AtomicOrdering::SeqCst) > ::MAX_PEERS {
                    stream.shutdown(Shutdown::Both);
                    warn!(
                        "Max numbers of peers {} reached; rejecting new connection",
                        ::MAX_PEERS
                    );
                } else {
                    match self.handle_client(&mut stream, handshake) {
                        Ok(peer) => {
                            info!("New peer connected: {}", peer.peer_id());
                            let peer_comm = self.register_peer(&peer, &manager_send);
                            let torrent_state = self.torrent_state.clone();
                            thread::spawn(move || {
                                // manages new connection
                                if let Err(e) = ConnectionHandler::receive(
                                    peer,
                                    stream,
                                    torrent_state,
                                    peer_comm,
                                ) {
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
}

/// Removes the peer specified by `peer_id`.
/// This is a macro to avoid annoying the borrow checker.
macro_rules! remove_peer {
    ($self:ident, $peer_id:ident) => {
        $self
            .torrent_state
            .npeers
            .fetch_sub(1, AtomicOrdering::SeqCst);
        remove(&mut $self.peer_priority, $peer_id);
        $self.bitfields.remove($peer_id);
        $self.peers.remove($peer_id);
        if $self.peers.len() == 0 {
            info!("All peers have disconnected");
        }
    };
}

macro_rules! add_timers {
    ($wheel:ident, $type:ident, $(($secs:expr, $func:ident)),*) => (
        $(
            $wheel.add(Duration::new($secs, 0), {fn anon(a: &mut $type) -> bool {
                a.$func();
                false
            }; anon}).unwrap();
        )*
    );
}

macro_rules! add_terminating_timers {
    ($wheel:ident, $type:ident, $(($secs:expr, $func:ident)),*) => (
        $(
            $wheel.add(Duration::new($secs, 0), {fn anon(a: &mut $type) -> bool {
                a.$func()
            }; anon}).unwrap();
        )*
    );
}

struct Controller {
    recv: mpsc::Receiver<NewPeerMsg>,
    torrent_state: Arc<TorrentState>,
    client_comm: BidirectionalChannel<InfoMsg, ClientMsg>,
    peer_priority: PriorityQueue<Hash, PeerPriority>,
    peers: HashMap<Hash, BidirectionalChannel<ManagerUpdate, PeerUpdate>>,
    bitfields: HashMap<Hash, BitVec>,
}

impl Controller {
    pub fn new(
        recv: mpsc::Receiver<NewPeerMsg>,
        state: Arc<TorrentState>,
        client_comm: BidirectionalChannel<InfoMsg, ClientMsg>,
    ) -> Controller {
        Controller {
            recv: recv,
            torrent_state: state,
            client_comm: client_comm,
            peer_priority: PriorityQueue::new(),
            peers: HashMap::new(),
            bitfields: HashMap::new(),
        }
    }

    pub fn run_loop(&mut self) {
        self.choke();
        let mut wheel = Timers::new(Duration::new(1, 0), self);

        add_terminating_timers!(wheel, Controller, (1, recv_client), (1, try_recv_new_peer));

        add_timers!(
            wheel,
            Controller,
            (1, send_update),
            (1, check_messages),
            (5, choke),
            (5, make_requests)
        );
        wheel.run_loop();
        info!("Shutting down");
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
                        error!("Tried to remove torrent that does not exist: {}", info_hash);
                    }
                }
                ClientMsg::Disconnect => {
                    info!("Received disconnect order");
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
            let comm = self
                .peers
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
            let peers_iter: Vec<Hash> = self
                .peer_priority
                .iter()
                .filter(|&(k, v)| !v.peer_choking && v.info_hash == *info_hash)
                .map(|(k, v)| k.clone())
                .collect(); // collect to be able to reuse

            while torrent.nrequests() < ::REQUESTS_LIMIT {
                match torrent.select_piece() {
                    Some(piece) => {
                        for id in peers_iter.iter() {
                            if self
                                .bitfields
                                .get(id)
                                .expect("Peer in priority queue but not bitfields")
                                .get(piece.index as usize)
                                .expect("Torrent bitfield and controller bitfields don't match")
                            {
                                // Request piece
                                let comm = self
                                    .peers
                                    .get(id)
                                    .expect("Peer in priority queue but not comms map");
                                comm.send(ManagerUpdate::Request(Peer::request(&piece)))
                                    .unwrap();
                                info!("Requesting piece {:?} from peer {}", piece, id);
                                break;
                            }
                            info!("No peer has this piece");
                        }
                    }
                    None => {
                        info!("Exhausted possible requests for torrent {}", torrent.name());
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

    fn try_recv_new_peer(&mut self) -> bool {
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
                        return false;
                    }
                    mpsc::TryRecvError::Disconnected => {
                        info!("TCP listener thread has shut down. Controller now shutting down.");
                        return true;
                    }
                },
            }
        }
        return false;
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
            npeers: self.torrent_state.npeers.load(AtomicOrdering::SeqCst),
        }
    }
}

#[cfg(test)]
mod tests {
    use bittorrent::manager::*;
    use bittorrent::tracker::tests::{default_tracker, run_server};
    use bittorrent::tracker::TrackerResponse;
    use env_logger;

    use std::thread::sleep;
    use std::time::Duration;

    /// Allows access to all these variables in repeated setup
    macro_rules! controller_setup {
        (
            $controller:ident,
            $manager_send:ident,
            $torrent_state:ident,
            $info_hash:ident,
            $npieces:ident
        ) => {
            controller_setup!(
                $controller,
                $manager_send,
                $torrent_state,
                $info_hash,
                $npieces,
                client
            );
        };
        (
            $controller:ident,
            $manager_send:ident,
            $torrent_state:ident,
            $info_hash:ident,
            $npieces:ident,
            $client:ident
        ) => {
            let ($manager_send, manager_recv) = mpsc::channel();
            let $torrent_state = Arc::new(TorrentState::new());
            let (info_send, $client) = BidirectionalChannel::create();

            let mut $controller = Controller::new(manager_recv, $torrent_state.clone(), info_send);

            // Add torrent
            let torrent = TorrentRuntime::new(::TEST_FILE, ::DL_DIR, false).unwrap();
            let $npieces = torrent.npieces();

            let $info_hash = torrent.info_hash().clone();
            let npieces = torrent.npieces();
            {
                let mut torrents = $torrent_state.torrents.lock().unwrap();
                torrents.insert(torrent.info_hash(), torrent);
            }
        };
    }

    macro_rules! get_torrent {
        ($torrent_state:ident, $info_hash:expr, $torrent:ident) => {
            let mut ts = $torrent_state.torrents.lock().unwrap();
            let mut $torrent = ts.get_mut($info_hash).unwrap();
        };
    }

    fn create_manager(port: u16) -> (Manager, BidirectionalChannel<ClientMsg, InfoMsg>) {
        let mut manager = Manager::new().port(port);
        let comm = manager.handle();
        return (manager, comm);
    }

    fn add_torrents(
        ports: &[u16],
        metainfo_path: &str,
        dl_path: &str,
        completed: bool,
    ) -> Vec<BidirectionalChannel<ClientMsg, InfoMsg>> {
        ports
            .iter()
            .map(|port| {
                let (mut manager, rx) = create_manager(*port);
                if !completed {
                    manager
                        .add_torrent(metainfo_path, &format!("{}/{}", dl_path, port))
                        .unwrap();
                } else {
                    manager
                        .add_completed_torrent(metainfo_path, dl_path)
                        .unwrap();
                };

                rx
            })
            .collect()
    }

    #[test]
    fn test_client_msgs() {
        controller_setup!(
            controller,
            manager_send,
            torrent_state,
            info_hash,
            npieces,
            client
        );
        // Send pause; make sure torrent is paused
        client.send(ClientMsg::Pause(info_hash));
        controller.recv_client();
        {
            get_torrent!(torrent_state, &info_hash, torrent);
            assert_eq!(torrent.status(), Status::Paused);
        }
        // Send resume; make sure it's resumed
        client.send(ClientMsg::Resume(info_hash));
        controller.recv_client();
        {
            get_torrent!(torrent_state, &info_hash, torrent);
            assert_eq!(torrent.status(), Status::Running);
        }
        // Send resume again; still resumed
        client.send(ClientMsg::Resume(info_hash));
        controller.recv_client();
        {
            get_torrent!(torrent_state, &info_hash, torrent);
            assert_eq!(torrent.status(), Status::Running);
        }
        // Remove it
        client.send(ClientMsg::Remove(info_hash));
        controller.recv_client();
        {
            let mut ts = torrent_state.torrents.lock().unwrap();
            assert_eq!(ts.get(&info_hash), None);
        }
    }

    /// Template for integration tests involving some number of seeders (starting completed)
    /// and leechers (starting empty).
    /// This is a macro becuase the function based to default_tracker must be static
    macro_rules! test_send_receive {
        ($n_seeders:expr, $n_leechers:expr) => {
            /// TODO: Needed to run in parallel:
            /// - Have global logger setup
            /// - Allocate ports globally
            /// - Create a separate tracker address for each - either set in torrent
            /// metainfo or create new temp file
            let _ = env_logger::init();
            let seeder_ports: Vec<u16> = (1700..1700 + $n_seeders as u16).collect();
            let leecher_ports: Vec<u16> = (1751..1751 + $n_leechers as u16).collect();

            thread::spawn(move || {
                fn tmp() -> TrackerResponse {
                    let seeder_vec: Vec<usize> = (1700..1700 + $n_seeders).collect();
                    default_tracker(&seeder_vec)
                };
                tests::run_server("127.0.0.1:3000", &tmp);
            });
            // Should be sufficient to let the server start
            sleep(Duration::new(0, 5000));

            let seeder_rxs = add_torrents(
                &seeder_ports,
                ::TEST_FILE,
                &format!("{}/{}", ::READ_DIR, "valid_torrent"),
                true,
            );

            let mut leecher_rxs = add_torrents(
                &leecher_ports,
                ::TEST_FILE,
                &format!("{}/{}", ::DL_DIR, "test_send_receive"),
                false,
            );

            while leecher_rxs.len() > 0 {
                leecher_rxs.retain(|leecher_rx| match leecher_rx.recv() {
                    InfoMsg::One(info) => match info.status {
                        Status::Complete => {
                            leecher_rx.send(ClientMsg::Disconnect).unwrap();
                            false
                        }
                        _ => true,
                    },
                    _ => true,
                });
            }

            for seeder_rx in seeder_rxs {
                seeder_rx.send(ClientMsg::Disconnect).unwrap();
            }
        };
    }

    #[test]
    fn test_many_leechers() {
        test_send_receive!(1, ::MAX_PEERS);
    }

    #[test]
    fn test_some_seeders_and_leechers() {
        test_send_receive!(10, 10);
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
        controller_setup!(controller, manager_send, ts, info_hash, npieces);
        // Simulate new peer
        let peer = gen_peer(3001, info_hash);
        let handler = ConnectionHandler {
            torrent_state: ts.clone(),
        };
        let peer_comm = handler.register_peer(&peer, &manager_send);

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

        // Priority of all pieces should be zero
        {
            let mut state = &controller.torrent_state;
            get_torrent!(state, &info_hash, torrent);
            let piece = torrent.select_piece();
            assert!(piece.is_none());
        }
    }

    #[test]
    fn test_make_requests() {
        // Setup
        controller_setup!(controller, manager_send, ts, info_hash, npieces);
        let handler = ConnectionHandler {
            torrent_state: ts.clone(),
        };
        // Connect some peers (mock)
        let peer = gen_peer(3002, info_hash);
        let peer_comm = handler.register_peer(&peer, &manager_send);
        let peer_2 = gen_peer(3003, info_hash);
        let peer_comm_2 = handler.register_peer(&peer_2, &manager_send);
        controller.try_recv_new_peer();

        // Some of those peers stop choking
        assert_eq!(controller.bitfields[peer_2.peer_id()][0], false);
        peer_comm_2.send(PeerUpdate::ChokingChange);
        peer_comm_2.send(PeerUpdate::Have(0));
        controller.check_messages();
        assert_eq!(controller.bitfields[peer_2.peer_id()][0], true);

        // Run make_requests()
        {
            let mut torrents = controller.torrent_state.torrents.lock().unwrap();
            let mut torrent = torrents.get_mut(&info_hash).unwrap();
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
