use byteorder::BigEndian;
use byteorder::ByteOrder;
use bit_vec::BitVec;
use bittorrent::{Piece, PieceData};
use std::mem::transmute;

#[derive(Debug)]
pub struct Peer {
    am_choking: bool, // if true, will not answer requests
    am_interested: bool, // interested in something the client has to offer
    peer_choking: bool,
    peer_interested: bool,
    peer_info : PeerInfo,
    time: u32, // time in ms since last communication
    bitfield: BitVec,
    request_queue: Vec<Piece>,
}

#[derive(Debug, PartialEq)]
pub enum Action<'a> {
    Request(Vec<Piece>),
    Write(PieceData<'a>),
    None,
}

const QUEUE_LENGTH : usize = 5;

// Macro for creating peer messages
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
impl Peer {

    // TODO: errors for lengths larger than the allowed size
    pub fn new(info : PeerInfo) -> Peer {
        Peer{
            am_choking: true,
            am_interested: false,
            peer_choking: true,
            peer_interested: false,
            peer_info: info,
            time: 0,
            bitfield: BitVec::new(),
            request_queue: Vec::new(),
        }
    }

    fn parse_message<'a>(&mut self, msg : &'a [u8]) -> Result<Action<'a>,&'static str> {
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

    fn parse_choke<'a>(&mut self, choke : bool) -> Result<Action<'a>,&'static str> {
        self.peer_choking = choke;
        Ok(Action::None)
    }

    fn parse_interested<'a>(&mut self, interested : bool) -> Result<Action<'a>,&'static str> {
        self.peer_interested = interested;
        Ok(Action::None)
    }

    fn parse_have<'a>(&mut self, piece_index : u32) -> Result<Action<'a>,&'static str> {
        if self.bitfield.is_empty() {
            return Err("Have not received bitfield from Peer");
        }
        if piece_index as usize >= self.bitfield.len() {
            return Err("Received have message with piece index that exceeds number of pieces");
        }
        self.bitfield.set(piece_index as usize, true);
        Ok(Action::None)
    }

    fn  parse_bitfield<'a>(&mut self, bitfield : &[u8]) -> Result<Action<'a>,&'static str>{
        self.bitfield = BitVec::from_bytes(bitfield);
        Ok(Action::None)
    }

    fn parse_piece_generic(&mut self, msg : &[u8]) -> Piece {
        let index = BigEndian::read_u32(&msg[5 .. 9]);
        let begin = BigEndian::read_u32(&msg[9 .. 13]);
        let length = BigEndian::read_u32(&msg[13 .. 17]);

        Piece{index: index, begin: begin, length: length}
    }

    fn parse_request<'a>(&mut self, msg : &[u8]) -> Result<Action<'a>,&'static str> {
        let piece = self.parse_piece_generic(msg);
        self.request_queue.push(piece);

        if self.request_queue.len() >= QUEUE_LENGTH {
            let action = Action::Request(self.request_queue.clone());
            self.request_queue.clear();
            Ok(action)
        }
        else {Ok(Action::None)}
    }
    
    fn parse_piece<'a>(&mut self, msg : &'a [u8], len : u32) -> Result<Action<'a>,&'static str> {
        let index = BigEndian::read_u32(&msg[5 .. 9]);
        let begin = BigEndian::read_u32(&msg[9 .. 13]);
        let block = &msg[13 .. len as usize];

        let piece = Piece{index: index, begin: begin, length: len - 13};
        let piece_data = PieceData{piece: piece, data: block};
        Ok(Action::Write(piece_data))
    }

    fn parse_cancel<'a>(&mut self, msg : &[u8]) -> Result<Action<'a>,&'static str> {
        let piece = self.parse_piece_generic(msg);
        match self.request_queue.remove_item(&piece) {
            Some(obj) => Ok(Action::None),
            None => Err("Received cancel for piece not in queue")
        }
    }

    fn parse_port<'a>(&mut self) -> Result<Action<'a>,&'static str> {
        unimplemented!();
        Ok(Action::None)
    }

    fn choke(&mut self, choke : bool) -> Vec<u8> {
        self.am_choking = choke;
        if(choke) {message!(1u32, 0u8)}
        else {message!(1u32, 1u8)}
    }

    fn interested(&self, interested : bool) -> Vec<u8> {
        unimplemented!();
    }
    
    fn have(&self, piece_index : u32) -> Vec<u8> {
        unimplemented!();
    }

    fn bitfield(&self) -> Vec<u8> {
        unimplemented!();
    }   

    fn piece(&self, msg : &[u8]) -> Vec<u8> {
        unimplemented!();
    }

    fn cancel(&self, msg : &[u8]) -> Vec<u8> {
        unimplemented!();
    }

    fn request(&self, pieces : Vec<Piece>) {
        unimplemented!();
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
    peer.parse_message(&unchoke);
    assert_eq!(peer.peer_choking, false);
    peer.parse_message(&interested);
    assert_eq!(peer.peer_interested, true);
    peer.parse_message(&not_interested);
    assert_eq!(peer.peer_interested, false);
}

#[test]
fn test_parse_have() {
    let mut peer = Peer::new(PeerInfo::new());

    let have = message!(5u32, 4u8, 0u32);
    assert_eq!(peer.parse_message(&have), Err("Have not received bitfield from Peer"));
    assert_eq!(peer.bitfield.get(0), None);

    let bitfield = message!(5u32, 5u8, 0b01000000000000000000000000000000 as u32);
    peer.parse_message(&bitfield);
    assert_eq!(peer.bitfield.get(1).unwrap(), true);

    let have = message!(5u32, 4u8, 32u32);
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
    let piece_data = PieceData{piece: piece, data: &bytes};

    let result = peer.parse_message(&message);
    assert_eq!(result, Ok(Action::Write(piece_data)));
}

#[derive(Debug)]
pub struct PeerInfo{
    peer_id: String,
    ip: String,
    port: i64,
}

impl PeerInfo {
    fn new() -> PeerInfo {
        PeerInfo{
            peer_id: String::from(""),
            ip: String::from("127.0.0.1"),
            port: 8000,
        }
    }
}

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

/*impl Message for HandshakeMessage {
    fn send(peer: Peer) {

    }

    fn concat() -> Vec<u8> {

    }
}*/