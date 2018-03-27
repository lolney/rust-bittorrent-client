/* Tracker communication handled here
*/
use bittorrent::{hash, Bencodable, BencodeT, Keys, ParseError};
use std::net::SocketAddr;
use bittorrent::Peer::{Peer, PeerInfo};
use bittorrent::bedecoder::parse;
use byteorder::{BigEndian, ByteOrder};
use futures::{Future, Stream};
use hyper::{Chunk, Client, Response};
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
                ip,
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
            TrackerPeer::Dictionary {
                ref peer_id,
                ip,
                port,
            } => {
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
                TrackerPeer::Dictionary,
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

struct RequestStream {
    urls : Vec<String>,
    status : Vec<RequestStatus>,
}

enum RequestStatus {
    None,
    Downloading,
    Complete,
}

impl RequestStream {
    fn new(urls : Vec<String>) -> RequestStream {
        let status = Vec::new();
        urls.iter().for_each(|url| status.push(RequestStatus::None));
        return RequestStream{
            urls : urls,
            status : status,
        }
    }
}

// Note: this changes a lot in futures 0.2
// https://docs.rs/futures/*/futures/stream/trait.Stream.html
impl Stream for RequestStream {
    type Item = Vec<SocketAddr>;
    type Error = ParseError;
    fn poll(
        &mut self
    ) -> Poll<Option<Self::Item> {
        // Check for completion
        // Make requests
        for (i, url) in self.urls.iter().enumerate() {
            match self.status[i] {
                RequestStatus::None => {
                    // Execute the request
                    let client = Client::new(self.core_handle);
                    let uri = url.parse()?;
                    let work = client.get(uri);
                    self.core_handle.execute(work);
                    self.status[i] = RequestStatus::Downloading;
                }
                RequestStatus::Downloading => {
                    // Poll download. Change status if done, then yield
                }
                RequestStatus::Complete => {}
            }
        }
        Ok(Async::Ready(None))
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
    ) -> Result<Vec<SocketAddr>, ParseError> {
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
        let mut vec = Vec::new();
        let stream = RequestStream::new(
            urls.iter()
                .map(|url| format!("{}?{}", url, query_string))
                .collect()
        );
        /*
        urls.iter()
            .map(|url| Tracker::make_request(format!("{}?{}", url, query_string)))
            .map(|res| res.and_then(|val| vec.append(&mut val)));
        return Ok(vec); */
    }

    #[async]
    pub fn make_request<F>(url: String) -> Result<Vec<SocketAddr>, ParseError>
    where
        F: Fn(SocketAddr),
    {
        let mut core = Core::new()?;
        let client = Client::new(&core.handle());

        let uri = url.parse()?;
        let work = client.get(uri).map(|res| {
            if !res.status().is_success() {
                return Err(parse_error!(
                    "Failed to get response from tracker: {}",
                    res.status()
                ));
            }
            // Error prop: https://github.com/rust-lang-nursery/futures-rs/issues/402
            Ok(res.body().concat2())
        });
        let body: Chunk = await!(await!(work)??)?;
        let peers = Tracker::parse_response(&body).unwrap();
        return Ok(peers.iter().map(|info| info.ip()).collect());
    }

    fn parse_response(body: &[u8]) -> Result<Vec<TrackerPeer>, ParseError> {
        let bencodet = parse(body)?;
        let response = TrackerResponse::from_BencodeT(&bencodet)?;
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
