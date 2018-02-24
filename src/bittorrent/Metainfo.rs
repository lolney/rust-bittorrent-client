use std::collections::HashMap;
use bittorrent::{BencodeT, Bencodable, Peer, ParseError, PieceData};
use bittorrent::bencoder::{encode};
use bittorrent::bedecoder::{parse};
use std::fs::File;
use std::path::Path;
use std::io::BufReader;
use std::io::prelude::*;
use sha1::Sha1;
use std::io::{Error};

/*
MetaInfo: The meat of a torrent file, containing hashes of all pieces and other metadata
*/
pub struct MetaInfo {
    info : Info,
    announce : String,
    announce_list : Option<Vec<String>>,
    creation_date : Option<String>,
    comment : Option<String>,
    created_by : Option<String>,
    encoding : Option<String>,
}

// Common traits to single- and multi- fileinfo
struct Info{
    piece_length : i64,
    pieces: Vec<u8>, // concatination of all 20-bit SHA-1 hash values
    private: bool,
}

#[derive(Debug)]
struct SingleFileInfo {
    name: String,
    length: i64,
    md5sum: Option<String>
}

struct MultiFileInfo {
    name: String,
    files: Vec<MIFile>,
}

// Component of a multi-file info
struct MIFile {
    length: i64,
    md5sum: Option<String>,
    path: Vec<String>, 
}

macro_rules! string_field {
    ($t:expr,$hm:expr,$($x:ident),*) => {
        $($hm.insert(String::from(stringify!($x)), BencodeT::String($t.$x)))*;
    }
}

macro_rules! string_optional {
    ($t:expr,$hm:expr,$($x:ident),*) => {
        $(match $t.$x {
            Some(ref string) => {$hm.insert(String::from(stringify!($x)), BencodeT::String(string.clone()));},
            None => {},
        })*
    }
}
/*
macro_rules! get_keys {
    ($($key:ident),*) => {
        $(
            let info = match hm.get("info") {
                    Some(info) => from_BencodeT(info),
                    None => Err(ParseError{msg: format!("Bencoded object does not contain key: {}", "info")})
                }
        )
    }
}*/
fn string_from_entry(hm : &HashMap<String, BencodeT>, string : &str) -> Option<String> {
    match hm.get(string){
        Some(string) => Some(String::from_BencodeT(string).unwrap()),
        None => None,
    }
}

// Need to check that the vector is of the required type
fn checkVec<T : Bencodable>(vec : Vec<BencodeT>) -> Result<Vec<T>, ParseError> {
    let mut out : Vec<T> = Vec::new();
    for elem in vec {
        match T::from_BencodeT(&elem) {
            Ok(val) => { out.push(val) }
            Err(error) => return Err(ParseError::new_str("Expected vector to contain u8s"))
        }
    }
    Ok(out)
}

fn checkHM<'a>(hm : &'a HashMap<String, BencodeT>, string : &str) -> Result<&'a BencodeT, ParseError>{
    match hm.get(string){
        Some(ref ben) => Ok(ben),
        None => return Err(ParseError::new(format!("Did not find {} in MetaInfo HashMap", string)))
    }
}

impl MetaInfo {

    pub fn info_hash(&self) -> [u8 ; 20] {
        let bstring = encode(&self.info.to_BencodeT());
        let mut sha = Sha1::new();
        sha.update(bstring.as_bytes());
        sha.digest().bytes()
    }

    pub fn read(infile : String) -> Result<MetaInfo, ParseError> {
        let file = File::open(infile)?;
        let mut buf_reader = BufReader::new(file);
        let mut contents = Vec::new();
        buf_reader.read_to_end(&mut contents)?;

        match parse(&contents){
            Ok(bencodet) => MetaInfo::from_BencodeT(&bencodet),
            Err(err) => Err(ParseError{msg: err}),
        }
    }

    pub fn write(&self, outfile : String) -> Result<(), Error>{
        let bencodet = self.to_BencodeT();
        let bencoded = encode(&bencodet);
        let mut file = File::create(&outfile)?;
        file.write_all(bencoded.as_bytes())
    }

    pub fn from_BencodeT(bencodet : &BencodeT) -> Result<MetaInfo, ParseError> {
        match bencodet {
            &BencodeT::Dictionary(ref hm) => Ok(
                MetaInfo{
                    info : Info::from_BencodeT(checkHM(hm, "info")?)?,
                    announce : String::from_BencodeT(checkHM(hm, "announce")?)?,
                    announce_list : match hm.get("announce_list") {
                        Some(vec) => Some(checkVec(Vec::from_BencodeT(vec)?)?),
                        None => None
                    },
                    creation_date : string_from_entry(hm, "creation_date"),
                    comment : string_from_entry(hm, "comment"),
                    created_by : string_from_entry(hm, "created_by"),
                    encoding : string_from_entry(hm, "encoding"),
                }
            ),
            _ => Err(ParseError::new_str("MetaInfo file not formatted as dictionary"))
        }
    }

    pub fn to_BencodeT(&self) -> BencodeT {
        let mut hm = HashMap::new();

        let info = self.info.to_BencodeT();
        hm.insert(String::from("info"), info);

        let announce = BencodeT::String(self.announce.clone());
        hm.insert(String::from("announce"), announce);

        match &self.announce_list {
            &Some(ref vec) => {
                let bvec = vec.iter().map(|x| BencodeT::String(x.clone())).collect();
                hm.insert(String::from("announce_list"), BencodeT::List(bvec));
            }
            &None => {} 
        }

        string_optional!(self, hm, creation_date, comment, created_by, encoding);

        BencodeT::Dictionary(hm)
    }

    pub fn write_piece(&self, path : &Path, piece : PieceData) {
        unimplemented!();
    }
}

impl Info {

    /* For decoding info objects that are bencoded */
    pub fn from_BencodeT(bencode_t : &BencodeT) -> Result<Info, ParseError> {
        match bencode_t {
            &BencodeT::Dictionary(ref hm) => {
                Ok(
                    Info {
                        piece_length : i64::from_BencodeT(hm.get("piece_length").unwrap())?,
                        pieces : checkVec(Vec::from_BencodeT(hm.get("pieces").unwrap())?)?,
                        private : bool::from_BencodeT(hm.get("private").unwrap())?,
                    }
                )
            },
            _ => Err(ParseError::new_str("Attempted to convert non-dictionary BencodeT to Info"))
        }
    }

    pub fn to_BencodeT(&self) -> BencodeT {
        let hm = HashMap::new();
        BencodeT::Dictionary(hm);
        unimplemented!();
    }
}


// TODO: Tests