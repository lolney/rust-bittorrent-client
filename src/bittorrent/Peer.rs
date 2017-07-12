use byteorder::BigEndian;
use byteorder::ByteOrder;


#[derive(Debug)]
pub struct Peer {
    am_choking: bool, // if true, will not answer requests
    am_interested: bool, // interested in something the client has to offer
    peer_choking: bool,
    peer_interested: bool,
    peer_info : PeerInfo,
    time: i32 // time in ms since last communication
}

impl Peer {

    pub fn new(info : PeerInfo) -> Peer {
        Peer{
            am_choking: true,
            am_interested: false,
            peer_choking: true,
            peer_interested: false,
            peer_info: info,
            time: 0
        }
    }

    fn parse_message(&mut self, msg : &[u8]) -> Result<(),&'static str> {
        self.time = 0;
        let len : u32 = BigEndian::read_u32(&msg[0 .. 4]); // parse u32
        if len == 0 {return Ok(());} // keep alive
        else {
            let id = msg[4];

            match id {
                0 => Ok(self.parse_choke(true)),
                1 => Ok(self.parse_choke(false)),
                2 => Ok(self.parse_interested(true)),
                3 => Ok(self.parse_interested(false)),
                4 => Ok(self.parse_have(BigEndian::read_u32(&msg[6 .. 9]))),
                _ => Err("Invalid message format")
            }
        }
        
    }

    /*fn handshake(&self){
        let msg = HandshakeMessage::new();
        msg.send();
    }*/

    // TODO: timer that close the connection when expired
    // May have to have a manager to do this
    // Is there an efficient way to set timers? (eg, via sys call)
    // Otherwise, can implement as a manager with a priority queue
    // Sleep until longest-lived peer expires, then delete

    fn parse_choke(&mut self, choke : bool){
        self.peer_choking = choke;
    }

    fn parse_interested(&mut self, interested : bool){
        self.peer_choking = interested;
    }

    fn parse_have(&self, piece_index : u32){

    }
    

}

#[derive(Debug)]
pub struct PeerInfo{
    peer_id: String,
    ip: String,
    port: i64,
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

#[derive(Debug)]
struct HandshakeMessage{
    pstrlen: u8,
    pstr: String, //protocol string identifier
    reserved: &'static [u8 ; 8], // 8 bytes
    info_hash: [u8 ; 20], // 20 bytes
    peer_id: [u8 ; 20] // 20 bytes
}

/*impl Message for HandshakeMessage {
    fn send(peer: Peer) {

    }

    fn concat() -> Vec<u8> {

    }
}*/