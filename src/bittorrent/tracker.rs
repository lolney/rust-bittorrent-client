/* Tracker communication handled here
*/
use bittorrent::Peer::{Peer, PeerInfo};
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
            callback(peerinfos);
        });
        core.run(work)?;
    }

    fn parse_response(body : &[u8]) -> Result<Vec<PeerInfo>, ParseError>{
        let bencodet = parse(body);
        match bencodet {
            &BencodeT::Dictionary(ref hm) => Ok(MetaInfo {
                failure_reason: elem_from_entry(hm, "failure reason"),
                announce_list: match hm.get("announce-list") {
                    Some(vec) => Some(checkVec(Vec::from_BencodeT(vec)?, convert)?),
                    None => None,
                },
                warning_message: elem_from_entry(hm, "warning message"),
                interval,
                min_interval,
                tracker_id,
                
            }),
            _ => Err(ParseError::new_str(
                "MetaInfo file not formatted as dictionary",
            )),
        }
    }

    pub fn announce() {}

    /// Queries the state of a given torrent
    pub fn scrape() {}
}
