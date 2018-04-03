/* Tracker communication handled here
*/
use bittorrent::{Bencodable, BencodeT, Hash, Keys, ParseError};
use std::net::SocketAddr;
use bittorrent::peer::{Peer, PeerInfo};
use bittorrent::bedecoder::parse;
use byteorder::{BigEndian, ByteOrder};
use futures::{Async, Future, Stream};
use futures::stream::Concat2;
use hyper::{Body, Chunk, Client, Response, Uri};
use hyper::client::{FutureResponse, HttpConnector};
use tokio_core::reactor::{Core, Handle};
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

#[derive(Debug, Clone, PartialEq)]
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

#[derive(Debug, Clone, PartialEq)]
pub enum TrackerPeer {
    Dictionary {
        peer_id: Hash,
        ip: String,
        port: usize,
    },
    Binary {
        ip: String,
        port: usize,
    },
}

impl TrackerPeer {
    fn ip_from_u32(ip: u32) -> String {
        let mut out = String::with_capacity(15);
        let mut bytes: [u8; 4] = [0; 4];
        BigEndian::write_u32(&mut bytes, ip);

        for i in 0..4 {
            out.push_str(&bytes[i].to_string());
            if i != 3 {
                out.push('.')
            }
        }
        out
    }

    fn u32_from_ip(ip: &str) -> u32 {
        let mut total: u32 = 0;
        for (i, octet) in ip.split('.').enumerate() {
            total += octet
                .parse::<u32>()
                .expect(&format!("IP string incorrectly formated: {}", ip))
                << ((3 - i) * 8)
        }
        total
    }

    pub fn from_binary(bytes: &[u8]) -> Result<TrackerPeer, ParseError> {
        if bytes.len() < 6 {
            return Err(parse_error!(
                "Expected binary-encoded TrackerPeer, but data field not long enough: {}",
                bytes.len()
            ));
        }

        Ok(TrackerPeer::Binary {
            ip: TrackerPeer::ip_from_u32(BigEndian::read_u32(&bytes[0..4])),
            port: BigEndian::read_u16(&bytes[4..6]) as usize,
        })
    }

    pub fn to_binary(ip: &str, port: usize) -> [u8; 6] {
        let mut buf: [u8; 6] = [0; 6];
        BigEndian::write_u32(&mut buf[0..4], TrackerPeer::u32_from_ip(ip));
        BigEndian::write_u16(&mut buf[4..6], port as u16);
        return buf;
    }

    fn create_ip(ip: &str, port: usize) -> Result<SocketAddr, ParseError> {
        match ip.parse() {
            Ok(v) => Ok(SocketAddr::new(v, port as u16)),
            Err(e) => Err(parse_error!("IP string incorrectly formatted: {}", ip)),
        }
    }

    pub fn ip(&self) -> Result<SocketAddr, ParseError> {
        return match self {
            &TrackerPeer::Binary { ref ip, port } => TrackerPeer::create_ip(ip, port),
            &TrackerPeer::Dictionary {
                ref peer_id,
                ref ip,
                port,
            } => TrackerPeer::create_ip(ip, port),
        };
    }
}

type OptionString = Option<String>;
type Optioni64 = Option<i64>;
type VecTrackerPeer = Vec<TrackerPeer>;

impl Bencodable for TrackerPeer {
    /// Note: expects input to be properly formatted, since only used to serialize
    /// already sanitized or internally created data
    fn to_BencodeT(self) -> BencodeT {
        match self {
            TrackerPeer::Binary { ref ip, port } => {
                let bytes = TrackerPeer::to_binary(ip, port);
                return unsafe { BencodeT::String(String::from_utf8_unchecked(bytes.to_vec())) };
            }
            TrackerPeer::Dictionary { peer_id, ip, port } => {
                let hm = to_keys!{
                    (peer_id, Hash),
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
                (peer_id, Hash),
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

impl Serializable for Hash {
    fn serialize(&self) -> String {
        form_urlencoded::byte_serialize(&self.0).collect()
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
        'outer: for (i, req) in self.reqs.iter_mut().enumerate() {
            match req.poll() {
                Ok(Async::Ready(v)) => {
                    let mut chunk = v.body();
                    // Is there a case where we have to wait here?
                    loop {
                        match chunk.poll() {
                            Ok(Async::Ready(v)) => {
                                let ips = Tracker::get_ips(v.unwrap())?;
                                res = Some(Async::Ready(Some(ips)));
                                index = i;
                                break 'outer;
                            }
                            Ok(Async::NotReady) => {}
                            Err(e) => {
                                return Err(ParseError::new_hyper(
                                    "Error while polling tracker response",
                                    e,
                                ));
                            }
                        }
                    }
                }
                Ok(Async::NotReady) => {}
                Err(e) => {
                    return Err(ParseError::new_hyper(
                        "Error while waiting for tracker response",
                        e,
                    ));
                }
            }
        }
        if let Some(v) = res {
            self.reqs.remove(index);
            return Ok(v);
        }
        return Ok(Async::NotReady);
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
        handle: Handle,
        info_hash: Hash,
        peer_id: Hash,
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
        let client = Client::new(&handle);
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
        let mut vec = Vec::new();
        for info in peers {
            vec.push(info.ip()?);
        }
        Ok(vec)
    }

    pub fn announce() {}

    /// Queries the state of a given torrent
    pub fn scrape() {}
}

#[cfg(test)]
pub mod tests {

    use bittorrent::tracker::*;
    use bittorrent::bencoder::encode;
    use bittorrent::peer::Peer;

    use futures::future::{ok, Future};
    use tokio_core::reactor::{Core, Handle};
    use std::time::Duration;
    use std::thread;

    use hyper::Error;
    use hyper::header::ContentLength;
    use hyper::server::{Http, Request, Response, Service};

    #[inline]
    fn place(i: usize, place: usize) -> bool {
        i & (1 << place) != 0
    }

    fn make_resps() -> Vec<TrackerResponse> {
        (0..8)
            .map(|i| TrackerResponse {
                warning_message: if place(i, 0) {
                    None
                } else {
                    Some("msg".to_string())
                },
                interval: 1,
                min_interval: if place(i, 1) { None } else { Some(1) },
                tracker_id: String::from("xyz"),
                complete: 10,
                incomplete: 10,
                peers: make_trackerpeers(20, place(i, 2)),
            })
            .collect()
    }

    pub fn default_tracker(port: &'static usize) -> Vec<TrackerResponse> {
        vec![
            TrackerResponse {
                warning_message: None,
                interval: 1,
                min_interval: Some(1),
                tracker_id: String::from("xyz"),
                complete: 10,
                incomplete: 10,
                peers: vec![
                    TrackerPeer::Binary {
                        ip: "127.0.0.1".to_string(),
                        port: *port,
                    },
                ],
            },
        ]
    }

    fn serialize_resp(resp: &TrackerResponse) -> BencodeT {
        let hm = to_keys_serialize!{
            resp,
            (warning_message, OptionString),
            (interval, i64),
            (min_interval, Optioni64),
            (tracker_id, String),
            (complete, i64),
            (incomplete, i64),
            (peers, VecTrackerPeer)
        };
        BencodeT::Dictionary(hm)
    }

    fn make_trackerpeers(n: usize, binary: bool) -> VecTrackerPeer {
        let make: Box<Fn(usize) -> TrackerPeer> = if binary {
            Box::new(|i| TrackerPeer::Binary {
                ip: format!("127.0.0.{}", i),
                port: i,
            })
        } else {
            Box::new(|i| TrackerPeer::Dictionary {
                ip: format!("127.0.0.{}", i),
                port: i,
                peer_id: Peer::gen_peer_id(),
            })
        };
        (0..n).map(|i| make(i)).collect()
    }

    #[test]
    fn test_tracker_resp() {
        for resp in make_resps() {
            assert_eq!(
                resp,
                TrackerResponse::from_BencodeT(&serialize_resp(&resp)).unwrap()
            );
        }
    }

    #[test]
    fn test_u32_from_ip() {
        type TP = TrackerPeer;
        assert_eq!(
            TP::u32_from_ip("255.255.255.255"),
            ((1_u64 << 32) - 1) as u32
        );
        assert_eq!(TP::u32_from_ip("0.0.0.0"), 0);
        assert_eq!(TP::ip_from_u32(256), "0.0.1.0");

        for i in 0..(1 << 4) {
            let i = i << 28;
            assert_eq!(TP::u32_from_ip(&TP::ip_from_u32(i as u32)), i as u32);
        }
    }

    #[derive(Debug, Clone)]
    struct SimpleServer {
        resp: String,
    }

    impl SimpleServer {
        fn new<F>(make_resps: F) -> SimpleServer
        where
            F: Fn() -> Vec<TrackerResponse>,
        {
            let responses = make_resps();
            SimpleServer {
                resp: encode(&serialize_resp(&responses[0])),
            }
        }
    }

    impl Service for SimpleServer {
        type Request = Request;
        type Response = Response;
        type Error = Error;
        type Future = Box<Future<Item = Self::Response, Error = Self::Error>>;

        fn call(&self, _req: Request) -> Self::Future {
            Box::new(ok(Response::new()
                .with_header(ContentLength(self.resp.len() as u64))
                .with_body(self.resp.clone())))
        }
    }

    pub fn run_server<F>(addr: &str, make_resps: &'static F)
    where
        F: Fn() -> Vec<TrackerResponse>,
    {
        let socket = addr.parse().unwrap();
        let server = Http::new()
            .bind(&socket, move || Ok(SimpleServer::new(make_resps)))
            .unwrap();
        server.run();
    }

    #[test]
    fn test_tracker() {
        // Make request to tracker
        let peer_id = Peer::gen_peer_id();
        let info_hash = Peer::gen_peer_id();
        let urls: Vec<String> = vec!["127.0.0.1:4000"]
            .into_iter()
            .map(|s| s.to_string())
            .collect();
        let responses = make_resps();

        let resp = &responses[0];

        let mut core = Core::new().unwrap();

        // Setup server
        let url = urls[0].clone();
        thread::spawn(move || {
            run_server(&url, &make_resps);
        });

        let stream = Tracker::get_peers(
            core.handle(),
            info_hash,
            peer_id,
            urls.into_iter()
                .map(|url| format!("http://{}", url))
                .collect(),
        ).unwrap();

        let future = stream.for_each(|vec| {
            let ips: Vec<SocketAddr> = resp.peers.iter().map(|peer| peer.ip().unwrap()).collect();
            assert_eq!(vec, ips);
            Ok(())
        });

        core.run(future);
    }
    /*
    TODO
    fn test_tracker_err() {
        // Tracker not available
        // Tracker returns bad response (not bencoded)
        // Can't convert to TrackerResponse
        stream.for_each(|vec| {});
    }*/
}
