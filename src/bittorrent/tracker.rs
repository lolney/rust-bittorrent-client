/* Tracker communication handled here
*/
use bittorrent::{keys. to_keys, parse_error, Bencodable, BencodeT, ParseError};
use bittorrent::Peer::{Peer, PeerInfo};
use bittorrent::bedecoder::parse;
use byteorder::BigEndian;
use futures::{Future, Stream};
use hyper::Client;
use tokio_core::reactor::Core;
use url::form_urlencoded;

#[derive(Debug)]
pub struct Tracker {
    tracker_id: String,
    peers: Vec<Peer>,
    interval: i64,
    min_interval: Option<i64>,
    complete: i64,
    incomplete: i64,
}

#[derive(Debug)]
pub struct TrackerResponse {
    warning_message: Option<String>,
    interval : i64,
    min_interval : Option<i64>,
    tracker_id : String,
    complete : i64,
    incomplete : i64,
    peers : Vec<TrackerPeer>,
}

impl TrackerResponse {
    fn from_BencodeT(bencode_t: &BencodeT) -> Result<String, ParseError> {
        match bencode_t {
            &BencodeT::Dictionary(ref hm) => {
                if let Some(val) = hm.get("failure reason") {
                    return parse_error!(
                        "Tracker returned failure: {:?}", val
                    );
                }
                keys!(TrackerResponse::Dictionary, hm,
                    (warning_message, Option<String>)
                    (interval , i64)
                    (min_interval , Option<i64>)
                    (tracker_id , String)
                    (complete , i64)
                    (incomplete , i64)
                    (peers, Vec<TrackerPeer>)
                )
            },
            _ => Err(parse_error!(
                "Tracker response is not a dictionary: {:?}", bencode_t
            )),
        }
    }
}

pub enum TrackerPeer {
    Dictionary{peer_id: [u8;20], ip : String, port : usize},
    Binary{ip : String, port : usize},
}

impl TrackerPeer {
    pub fn from_binary(bytes: [u8;6]) -> TrackerPeer {
        TrackerPeer::Binary{
            ip: BigEndian::read_u32(bytes[0..4]).to_string(),
            port: BigEndian::read_u16(bytes[4..6] as usize,
        }
    }

    pub fn to_binary(ip : String, port : usize) -> [u8;6] {
        let mut buf : [u8;6] = [0;6];
        BigEndian::write_u32(&buf[0..4], ip.parse::<u32>().unwrap());
        BigEndian::write_u16(&buf[4..6], port as u16);
        return buf;
    }
}

impl Bencodable for TrackerPeer {
    fn to_BencodeT(self) -> BencodeT {
        match self {
            &TrackerPeer::Binary{ref ip, port} => {
                let bytes = TrackerPeer::to_binary(ip, port);
                return unsafe { BencodeT::String(String::from_utf8_unchecked(bytes)) };
            },
            &TrackerPeer::Dictionary{ref peer_id, ip, port} => {
                let hm = to_keys!{ 
                    (peer_id, [u8;20]),
                    (ip, String),
                    (port, usize),
                }; 
                BencodeT::Dictionary(hm);
            }
        }
    }
    fn from_BencodeT(bencode_t: &BencodeT) -> Result<TrackerPeer, ParseError> {
        match bencode_t {
            &BencodeT::String(ref string) => Ok(TrackerPeer::from_binary(string.as_bytes())),
            &BencodeT::Dictionary(ref hm) => {
                Ok(keys!(TrackerPeer::Dictionary, hm,
                    (peer_id, [u8;20]),
                    (ip, String),
                    (port, usize),
                ))
            },
            _ => Err(parse_error!(
                "Attempted to create TrackerPeer with non-string or dict: {:?}", bencode_t
            )),
        }
    }
}

trait Serializable {
    fn serialize(&self) -> String;
}

impl Serializable for usize {
    fn serialize(&self) -> String {
        self.to_string()
    }
}

impl Serializable for [u8] {
    fn serialize(&self) -> String {
        form_urlencoded::byte_serialize(self).collect()
    }
}

impl Serializable for String {
    fn serialize(&self) -> String {
        return self.clone()
    }
}

macro_rules! serialize {
    ((($a : expr, $b : expr),)*) => {
        let mut serializer = form_urlencoded::Serializer::new(String::new()); 
        serializer($(append_pair($a.serialize(), $b.serialize()))*).finish()
    }
}

impl Tracker {
    pub fn new(url: String) {}

    pub fn get_peers(
        info_hash: [u8; 20], 
        peer_id: [u8; 20], urls: Vec<String>,
        callback : PeerInfo -> ()
        ) {
        let trackers = urls.iter().map(|url| Tracker::new(url.clone())).collect();
        
        let query_string = serialize!(
            ("info_hash",info_hash),
            ("peer_id",peer_id),
            ("port", ::PORT_NUM),
            ("uploaded", 0),
            ("downloaded", 0),
            ("left", 0),
            ("compact", 1),
            ("event", "started"),
            ("numwant", ::MAX_PEERS),
        );
        make_request(url.push_str("?").push_str(&query_string), callback);
    }

    pub fn make_request(url : String, callback: PeerInfo -> ()) -> Result<(), Error>{
        let mut core = Core::new()?;
        let client = Client::new(&core.handle());

        let uri = url.parse()?;
        let work = client.get(uri).map(|res| {
            let peerinfos = Tracker::parse_response(res.body);
            // Callback launches a thread. Can checking the response be deferred until there?
            callback(peerinfos);
        });
        core.run(work)?;
    }

    fn parse_response(body : &[u8]) -> Result<Vec<PeerInfo>, ParseError>{
        let bencodet = parse(body)?;
        let response = TrackerResponse::from_BencodeT(bencodet)?;
        return Ok(response.peers);
    }

    pub fn announce() {}

    /// Queries the state of a given torrent
    pub fn scrape() {}
}

#[cfg(test)]
mod tests {

    use bittorrent::tracker::*;

}