use byteorder::BigEndian;
use byteorder::ByteOrder;
use bit_vec::BitVec;
use bittorrent::{Piece, PieceData, ParseError};
use std::mem::transmute;
use rand::{Rng};
use rand;
use std::str::from_utf8;

#[derive(Debug)]
pub struct Peer {
    pub am_choking: bool, // if true, will not answer requests
    pub am_interested: bool, // interested in something the client has to offer
    pub peer_choking: bool,
    pub peer_interested: bool,
    pub peer_info : PeerInfo,
    pub time: u32, // time in ms since last communication
    pub bitfield: BitVec,
    pub request_queue: Vec<Piece>,
}

#[derive(Debug, PartialEq)]
pub enum Action {
    Request(Vec<Piece>),
    Write(PieceData),
    InterestedChange,
    None,
}

const QUEUE_LENGTH : usize = 5;

// Macro for creating peer messages with 32-byte arguments
macro_rules! message {
        ($len:expr,$id:expr$(, $other:expr)*) => {
            {
                let mut vec = Vec::<u8>::new();
                let bytes: [u8; 4] = unsafe { transmute($len.to_be()) };
                vec.extend_from_slice(&bytes);
                vec.push($id);

                $(  
                    let bytes: [u8; 4] = unsafe { transmute($other.to_be()) };
                    vec.extend_from_slice(&bytes);
                )*

                vec
            }
        };
    }

macro_rules! message_from_bytes {
        ($len:expr,$id:expr,$bytes:expr$(, $other:expr)*) => {
            {
                let mut vec = message!($len, $id$(, $other)*);
                vec.extend_from_slice(&$bytes);

                vec
            }
        };
    }

macro_rules! byte_slice_from_u32s {
    ($($int:expr),*) => {
        {
            let mut vec = Vec::<u8>::new();
            $(  
                let bytes: [u8; 4] = unsafe { transmute($int.to_be()) };
                vec.extend_from_slice(&bytes);
            )*
            vec
        }
    };
}

macro_rules! message_calc_length {
    ($id:expr$(, $other:expr)*) => {
        {
            let mut len : u32 = 1;

            $(  
                let l = $other;
                len = len + 4;
            )*

            message!(len, $id $(, $other )*)
        }
    };
}

/// Maintains its own request queue
/// Outsources actual networking
/// Contains methods for parsing and sending relevant P2P messages
impl Peer {

    // TODO: errors for lengths larger than the allowed size
    pub fn new(info : PeerInfo) -> Peer {
        Peer{
            am_choking: true, // we won't send (we control)
            am_interested: false, // requesting peer to send (we control)
            peer_choking: true, // peer won't send (they control)
            peer_interested: false, // peer wants us to send (they control)
            peer_info: info,
            time: 0,
            bitfield: BitVec::new(), // their bitfield
            request_queue: Vec::new(),
        }
    }

    pub fn parse_message(&mut self, msg : &[u8]) -> Result<Action,&'static str> {
        self.time = 0;
        let len : u32 = BigEndian::read_u32(&msg[0 .. 4]); // parse u32
        if len == 0 {Ok(Action::None)} // keep alive
        else {
            let id = msg[4];

            match id {
                0 => self.parse_choke(true),
                1 => self.parse_choke(false),
                2 => self.parse_interested(true),
                3 => self.parse_interested(false),
                4 => self.parse_have(BigEndian::read_u32(&msg[5 .. 9])),
                5 => self.parse_bitfield(&msg[5 .. 5 + len as usize - 1]),
                6 => self.parse_request(msg),
                7 => self.parse_piece(msg, len + 4),
                8 => self.parse_cancel(msg),
                9 => self.parse_port(),
                _ => Err("Invalid message id")
            }
        }
        
    }

    pub fn info_hash(&self) -> &[u8; 20] {
        &self.peer_info.info_hash
    }

    pub fn peer_id(&self) -> &[u8; 20] {
        &self.peer_info.peer_id
    }

    pub fn peer_id_string(&self) -> String {
        String::from(from_utf8(&self.peer_info.peer_id).unwrap())
    }

    // TODO: these errors should probably be PareErrors
    pub fn parse_choke(&mut self, choke : bool) -> Result<Action,&'static str> {
        self.peer_choking = choke;
        Ok(Action::None)
    }

    pub fn parse_interested(&mut self, interested : bool) -> Result<Action,&'static str> {
        self.peer_interested = interested;
        Ok(Action::InterestedChange)
    }

    pub fn parse_have(&mut self, piece_index : u32) -> Result<Action,&'static str> {
        if self.bitfield.is_empty() {
            return Err("Have not received bitfield from Peer");
        }
        if piece_index as usize >= self.bitfield.len() {
            return Err("Received have message with piece index that exceeds number of pieces");
        }
        self.bitfield.set(piece_index as usize, true);
        Ok(Action::None)
    }

    pub fn  parse_bitfield(&mut self, bitfield : &[u8]) -> Result<Action,&'static str>{
        self.bitfield = BitVec::from_bytes(bitfield);
        Ok(Action::None)
    }

    pub fn parse_piece_generic(&mut self, msg : &[u8]) -> Piece {
        let index = BigEndian::read_u32(&msg[5 .. 9]);
        let begin = BigEndian::read_u32(&msg[9 .. 13]);
        let length = BigEndian::read_u32(&msg[13 .. 17]);

        Piece{index: index, begin: begin, length: length}
    }

    // TODO: error when exceeding 2^14?
    pub fn parse_request(&mut self, msg : &[u8]) -> Result<Action,&'static str> {
        let piece = self.parse_piece_generic(msg);
        self.request_queue.push(piece);

        if self.request_queue.len() >= QUEUE_LENGTH {
            let action = Action::Request(self.request_queue.clone());
            self.request_queue.clear();
            Ok(action)
        }
        else {Ok(Action::None)}
    }
    
    // TODO: error when exceeding 2^14?
    pub fn parse_piece(&mut self, msg : &[u8], len : u32) -> Result<Action,&'static str> {
        let index = BigEndian::read_u32(&msg[5 .. 9]);
        let begin = BigEndian::read_u32(&msg[9 .. 13]);
        let block = &msg[13 .. len as usize];

        let piece = Piece{index: index, begin: begin, length: len - 13};
        let mut vec = Vec::new();
        vec.extend_from_slice(block);
        let piece_data = PieceData{piece: piece, data: vec};
        Ok(Action::Write(piece_data))
    }

    pub fn parse_cancel(&mut self, msg : &[u8]) -> Result<Action,&'static str> {
        let piece = self.parse_piece_generic(msg);
        match self.request_queue.remove_item(&piece) {
            Some(obj) => Ok(Action::None),
            None => Err("Received cancel for piece not in queue")
        }
    }

    pub fn parse_port(&mut self) -> Result<Action,&'static str> {
        unimplemented!();
        Ok(Action::None)
    }

    pub fn choke(&mut self, choke : bool) -> Vec<u8> {
        self.am_choking = choke;
        if(choke) {message!(1u32, 0u8)}
        else {message!(1u32, 1u8)}
    }

    pub fn interested(&mut self, interested : bool) -> Vec<u8> {
        self.am_interested = interested;
        if(interested) {message!(1u32, 2u8)}
        else {message!(1u32, 3u8)}
    }
    
    pub fn have(piece_index : u32) -> Vec<u8> {
        message!(5u32, 4u8, piece_index)
    }

    // Accomodates bitvecs of max length (MAX_U32 - 1)
    pub fn bitfield(bitvec : BitVec) -> Vec<u8> {
        let bytes = bitvec.to_bytes();
        let length = 1 + bytes.len() as u32;
        message_from_bytes!(length, 5u8, bytes)
    }   

    pub fn piece(pd : &PieceData) -> Vec<u8> {
        let length = 9 + pd.data.len() as u32;
        message_from_bytes!(length, 7u8, &pd.data.as_slice(), pd.piece.index, pd.piece.begin)
    }

    // 2^14 is generally the max length;
    // probably will enforce this when making requests
    pub fn request(piece : &Piece) -> Vec<u8> {
        message!(13u32, 6u8, piece.index, piece.begin, piece.length)
    }

    pub fn cancel(piece : &Piece) -> Vec<u8> {
        message!(13u32, 8u8, piece.index, piece.begin, piece.length)
    }

    pub fn gen_peer_id() -> [u8; 20] {
        let mut id : [u8;20] = [0;20];
        let mut rng = rand::thread_rng();
        for x in id.iter_mut() {
            *x = rng.gen();
        }
        return id;
    }

    /// Static method: received before the peer is created
    /// Extracts the torrent and Peer ID from the handshake
    pub fn parse_handshake(msg : &[u8]) -> Result<([u8 ; 20], [u8 ; 20]), ParseError>{
        let mut i = 0;

        // check message size
        if(msg.len() < ::HSLEN){
            return Err(ParseError::new(format!("Unexpected handshake length: {}", msg.len())));
        }

        let pstrlen = msg[i];
        if pstrlen != ::PSTRLEN {
            return Err(ParseError::new(format!("Unexpected pstrlen: {}", pstrlen)));
        }

        i = i + 1;
        let pstr = &msg[i .. i + pstrlen as usize];
        if from_utf8(pstr).unwrap() != ::PSTR {
            return Err(ParseError::new(format!("Unexpected protocol string: {:?}", pstr)));
        }

        i = i + 8 + pstrlen as usize;
        let mut info_hash: [u8; 20] = Default::default();
        info_hash.copy_from_slice(&msg[i .. i + 20]);

        i = i + 20;
        let mut peer_id: [u8; 20] = Default::default();
        peer_id.copy_from_slice(&msg[i .. i + 20]);

        return Ok((peer_id, info_hash))
    }

    /// Generate the handshake message;
    pub fn handshake(peer_id : &[u8;20], info_hash : &[u8;20]) -> [u8; ::HSLEN] {
        let mut i = 0;
        let mut msg : [u8; ::HSLEN] = [0 ;  ::HSLEN];

        msg[i] = ::PSTRLEN;
        i = i+1;
        msg[i .. i+::PSTRLEN as usize].copy_from_slice(::PSTR.as_bytes());
        i += 8 + ::PSTRLEN as usize;
        msg[i .. i+20].copy_from_slice(info_hash);
        i += 20;
        msg[i .. i+20].copy_from_slice(peer_id);

        return msg;
    }

}

#[test]
fn test_simple_messages() {
    let mut peer = Peer::new(PeerInfo::new());

    let keep_alive = [0u32];
    let choke = message!(1u32, 0u8);
    let unchoke = message!(1u32, 1u8);
    let interested = message!(1u32, 2u8);
    let not_interested = message!(1u32, 3u8);

    peer.parse_message(&choke);
    assert_eq!(peer.peer_choking, true);
    assert_eq!(peer.choke(true), choke);
    assert_eq!(peer.choke(true), &[0b00000000, 0b00000000, 0b00000000, 0b00000001, 0b000000000]);

    peer.parse_message(&unchoke);
    assert_eq!(peer.peer_choking, false);
    assert_eq!(peer.choke(false), unchoke);

    peer.parse_message(&interested);
    assert_eq!(peer.peer_interested, true);
    assert_eq!(peer.interested(true), interested);

    peer.parse_message(&not_interested);
    assert_eq!(peer.peer_interested, false);
    assert_eq!(peer.interested(false), not_interested);
}

#[test]
fn test_parse_have() {
    let mut peer = Peer::new(PeerInfo::new());

    let have = message!(5u32, 4u8, 0u32);
    assert_eq!(Peer::have(0), have);
    assert_eq!(peer.parse_message(&have), Err("Have not received bitfield from Peer"));
    assert_eq!(peer.bitfield.get(0), None);

    let bitfield = message!(5u32, 5u8, 0b01000000000000000000000000000000 as u32);
    assert_eq!(Peer::bitfield(BitVec::from_bytes(&[0b01000000, 0b00000000, 0b00000000, 0b00000000])), bitfield);
    peer.parse_message(&bitfield);
    assert_eq!(peer.bitfield.get(1).unwrap(), true);

    let have = message!(5u32, 4u8, 32u32);
    assert_eq!(Peer::have(32), have);
    assert_eq!(peer.parse_message(&have), Err("Received have message with piece index that exceeds number of pieces"));

    let bitfield = message!(9u32, 5u8, 0b10000000000000000000000000000000 as u32, 0b10000000000000000000000000000000 as u32);
    peer.parse_message(&bitfield);
    assert_eq!(peer.bitfield.get(32).unwrap(), true);

    assert_eq!(peer.bitfield.get(7).unwrap(), false);
    let have = message!(5u32, 4u8, 7u32);
    peer.parse_message(&have);
    assert_eq!(peer.bitfield.get(7).unwrap(), true);
}

#[test]
fn test_parse_request() {
    let mut peer = Peer::new(PeerInfo::new());

    let request = message!(13u32, 6u8, 0u32, 0u32, 16384u32);

    for i in  0 .. QUEUE_LENGTH - 1 {
        assert_eq!(peer.parse_request(&request), Ok(Action::None));
    }

    peer.parse_request(&request);
    assert_eq!(peer.request_queue.len(), 0);
}

#[test]
fn test_parse_piece() {
    let mut peer = Peer::new(PeerInfo::new());

    let message = message_calc_length!(7u8, 0u32, 0u32, 1u32, 2u32, 3u32);
    let bytes = byte_slice_from_u32s!(1u32, 2u32, 3u32);
    let piece = Piece{index: 0u32, begin: 0u32, length: 12u32};
    let piece_data = PieceData{piece: piece, data: bytes};

    assert_eq!(Peer::piece(&piece_data), message);

    let result = peer.parse_message(&message);
    assert_eq!(result, Ok(Action::Write(piece_data)));
}

#[test]
fn test_parse_handshake() {
    let mut buf : [u8 ; 68] = [0;68];
    let peer_id = Peer::gen_peer_id();
    let info_hash = Peer::gen_peer_id();
    let hsmsg = Peer::handshake(&peer_id, &info_hash);
    let (pi, inf) = Peer::parse_handshake(&hsmsg).unwrap();

    assert_eq!(pi, peer_id);
    assert_eq!(inf,  info_hash);
}

#[derive(Debug)]
pub struct PeerInfo{
    pub peer_id: [u8 ; 20],
    pub info_hash: [u8 ; 20],
    pub ip: String,
    pub port: i64,
}

impl PeerInfo {
    fn new() -> PeerInfo {
        PeerInfo{
            peer_id: Default::default(),
            info_hash: Default::default(),
            ip: String::from("127.0.0.1"),
            port: 8000,
        }
    }
}

/* Found these data structures unecessary
trait Message{
    fn send(peer : Peer);
    fn handle();
    fn concat() -> Vec<u8>; // possible to generically add all struct members together?
}

#[derive(Debug)]
struct StdMessage{
    length_prefix: i32,
    message_id: String
}
*/