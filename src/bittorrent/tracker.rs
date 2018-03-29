/* Tracker communication handled here
*/
use bittorrent::{hash, Bencodable, BencodeT, Keys, ParseError};
use std::net::SocketAddr;
use bittorrent::Peer::{Peer, PeerInfo};
use bittorrent::bedecoder::parse;
use byteorder::{BigEndian, ByteOrder};
use futures::{Future, Stream};
use hyper::{Chunk, Client, Response, Uri};
use hyper::client::{FutureResponse, HttpConnector};
use tokio_core::reactor::Core;
use url::form_urlencoded;
use std::collections::HashMap;
use hyper::error::Error as HyperError;
use std::ops::Try;
use futures::prelude::*;

#[derive(Debug, Clone)]
pub struct Tracker {
    tracker_id: String,
    peers: Vec<Peer>,
    interval: i64,
    min_interval: Option<i64>,
    complete: i64,
    incomplete: i64,
}

#[derive(Debug, Clone)]
pub struct TrackerResponse {
    warning_message: Option<String>,
    interval: i64,
    min_interval: Option<i64>,
    tracker_id: String,
    complete: i64,
    incomplete: i64,
    peers: Vec<TrackerPeer>,
}

impl TrackerResponse {
    fn from_BencodeT(bencode_t: &BencodeT) -> Result<TrackerResponse, ParseError> {
        match bencode_t {
            &BencodeT::Dictionary(ref hm) => {
                if let Some(val) = hm.get("failure reason") {
                    return Err(parse_error!("Tracker returned failure: {:?}", val));
                }
                Ok(get_keys!(
                    TrackerResponse,
                    hm,
                    (warning_message, OptionString),
                    (interval, i64),
                    (min_interval, Optioni64),
                    (tracker_id, String),
                    (complete, i64),
                    (incomplete, i64),
                    (peers, VecTrackerPeer)
                ))
            }
            _ => Err(parse_error!(
                "Tracker response is not a dictionary: {:?}",
                bencode_t
            )),
        }
    }
}

#[derive(Debug, Clone)]
pub enum TrackerPeer {
    Dictionary {
        peer_id: hash,
        ip: String,
        port: usize,
    },
    Binary {
        ip: String,
        port: usize,
    },
}

impl TrackerPeer {
    pub fn from_binary(bytes: &[u8]) -> Result<TrackerPeer, ParseError> {
        if bytes.len() < 6 {}

        Ok(TrackerPeer::Binary {
            ip: BigEndian::read_u32(&bytes[0..4]).to_string(),
            port: BigEndian::read_u16(&bytes[4..6]) as usize,
        })
    }

    pub fn to_binary(ip: &str, port: usize) -> [u8; 6] {
        let mut buf: [u8; 6] = [0; 6];
        BigEndian::write_u32(&mut buf[0..4], ip.parse::<u32>().unwrap());
        BigEndian::write_u16(&mut buf[4..6], port as u16);
        return buf;
    }

    pub fn ip(&self) -> SocketAddr {
        return match self {
            &TrackerPeer::Binary { ref ip, port } => {
                SocketAddr::new(ip.parse().unwrap(), port as u16)
            }
            &TrackerPeer::Dictionary {
                ref peer_id,
                ref ip,
                port,
            } => SocketAddr::new(ip.parse().unwrap(), port as u16),
        };
    }
}

type OptionString = Option<String>;
type Optioni64 = Option<i64>;
type VecTrackerPeer = Vec<TrackerPeer>;

impl Bencodable for TrackerPeer {
    fn to_BencodeT(self) -> BencodeT {
        match self {
            TrackerPeer::Binary { ref ip, port } => {
                let bytes = TrackerPeer::to_binary(ip, port);
                return unsafe { BencodeT::String(String::from_utf8_unchecked(bytes.to_vec())) };
            }
            TrackerPeer::Dictionary { peer_id, ip, port } => {
                let hm = to_keys!{
                    (peer_id, hash),
                    (ip, String),
                    (port, usize)
                };
                BencodeT::Dictionary(hm)
            }
        }
    }
    fn from_BencodeT(bencode_t: &BencodeT) -> Result<TrackerPeer, ParseError> {
        match bencode_t {
            &BencodeT::String(ref string) => Ok(TrackerPeer::from_binary(string.as_bytes())?),
            &BencodeT::Dictionary(ref hm) => Ok(get_keys_enum!(
                TrackerPeer,
                Dictionary,
                hm,
                (peer_id, hash),
                (ip, String),
                (port, usize)
            )),
            _ => Err(parse_error!(
                "Attempted to create TrackerPeer with non-string or dict: {:?}",
                bencode_t
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
        return self.clone();
    }
}

impl Serializable for str {
    fn serialize(&self) -> String {
        return self.to_string();
    }
}

pub struct RequestStream {
    reqs: Vec<FutureResponse>,
}

impl RequestStream {
    fn new(urls: Vec<Uri>, client: Client<HttpConnector>) -> RequestStream {
        let reqs = urls.iter().map(|url| client.get(url.clone())).collect();
        return RequestStream { reqs: reqs };
    }
}

// Note: this changes a lot in futures 0.2
// https://docs.rs/futures/*/futures/stream/trait.Stream.html
impl Stream for RequestStream {
    type Item = Vec<SocketAddr>;
    type Error = ParseError;
    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        // Check for completion
        // Make requests
        if self.reqs.len() == 0 {
            return Ok(Async::Ready(None));
        }
        let mut res = None;
        let mut index = 0;
        for (i, req) in self.reqs.iter_mut().enumerate() {
            match req.poll() {
                Ok(Async::Ready(v)) => match v.body().concat2().poll() {
                    Ok(Async::Ready(v)) => {
                        let ips = Tracker::get_ips(v)?;
                        res = Some(Async::Ready(Some(ips)));
                        index = i;
                        break;
                    }
                    Ok(Async::NotReady) => {}
                    Err(e) => {
                        return Err(ParseError::new_str("canceled"));
                    }
                },
                Ok(Async::NotReady) => {}
                Err(e) => {
                    return Err(ParseError::new_str("canceled"));
                }
            }
        }
        if res.is_some() {
            self.reqs.remove(index);
            return Ok(res.unwrap());
        }
        return Ok(Async::Ready(None));
    }
}

macro_rules! serialize {
    ($(($a : expr, $b : expr),)*) => {
        {
            let mut serializer = form_urlencoded::Serializer::new(String::new());
            serializer$(.append_pair(&$a.serialize(), &$b.serialize()))*.finish()
        }
    }
}

impl Tracker {
    pub fn new(url: String) {}

    pub fn get_peers(
        info_hash: hash,
        peer_id: hash,
        urls: Vec<String>,
    ) -> Result<RequestStream, ParseError> {
        let query_string = serialize!(
            ("info_hash", info_hash),
            ("peer_id", peer_id),
            ("port", ::PORT_NUM),
            ("uploaded", 0),
            ("downloaded", 0),
            ("left", 0),
            ("compact", 1),
            ("event", "started"),
            ("numwant", ::MAX_PEERS),
        );
        let mut uris = Vec::new();
        for url in urls {
            let uri: Uri = format!("{}?{}", url, query_string).parse()?;
            uris.push(uri);
        }
        let core = Core::new()?;
        let client = Client::new(&core.handle());
        let stream = RequestStream::new(uris, client);
        return Ok(stream);
    }

    fn parse_response(body: &[u8]) -> Result<Vec<TrackerPeer>, ParseError> {
        let bencodet = parse(body)?;
        let response = TrackerResponse::from_BencodeT(&bencodet)?;
        return Ok(response.peers);
    }

    fn get_ips(chunk: Chunk) -> Result<Vec<SocketAddr>, ParseError> {
        let peers = Tracker::parse_response(&chunk)?;
        return Ok(peers.iter().map(|info| info.ip()).collect());
    }

    pub fn announce() {}

    /// Queries the state of a given torrent
    pub fn scrape() {}
}

#[cfg(test)]
mod tests {

    use bittorrent::tracker::*;

}
