use std::collections::HashMap;
use bittorrent::{BencodeT, Bencodable, Peer}; 

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
    legnth: i64,
    md5sum: Option<String>
}

struct MultiFileInfo {
    name: String,
    files: Vec<File>,
}

// Component of a multi-file info
struct File {
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

impl MetaInfo {
    pub fn decode(&self, infile : String) {

    }

    pub fn encode(&self, outfile : String) {
        
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
}

impl Info {
    pub fn to_BencodeT(&self) -> BencodeT {
        let hm = HashMap::new();
        BencodeT::Dictionary(hm)
    }
}