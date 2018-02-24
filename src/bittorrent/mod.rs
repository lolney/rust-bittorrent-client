use std::collections::HashMap; 
use std::error::Error;
use std::path::Path;
use std::fmt::Display;
use std::fmt;
use std::io::Error as IOError;

mod bencoder;
mod bedecoder;
mod utils;
mod Peer;
mod Trackers;
mod Metainfo;

/* Describes a decoded benencodable object */
#[derive(PartialEq, Debug, Clone)]
pub enum BencodeT {
    String(String),
    Integer(i64),
    Dictionary(HashMap<String, BencodeT>),
    List(Vec<BencodeT>),
}

#[derive(Debug)]
pub struct ParseError{
    msg: String,
}

impl ParseError {
    pub fn new(string : String) -> ParseError {
        ParseError {
            msg: string,
        }
    }

    // No polymorphic constructors
    pub fn new_str(string : &str) -> ParseError {
        ParseError {
            msg: String::from(string),
        }
    }
}

impl From<IOError> for ParseError {
    fn from(error: IOError) -> Self {
        ParseError::new_str(error.description())
    }
}

impl Error for ParseError {
    fn description(&self) -> &str {self.msg.as_str()}
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.msg)
    }
}

pub trait Bencodable {
    fn to_BencodeT(self) -> BencodeT;
    fn from_BencodeT(bencode_t : &BencodeT) -> Result<Self, ParseError> where Self: Sized;
}

impl Bencodable for String {
    fn to_BencodeT(self) -> BencodeT {BencodeT::String(self)}
    fn from_BencodeT(bencode_t : &BencodeT) -> Result<String, ParseError> {
        match bencode_t {
            &BencodeT::String(ref string) => Ok(string.clone()),
            _ => Err(ParseError::new_str("Attempted to convert non-string BencodeT to string"))
        }
    }
}

impl Bencodable for HashMap<String, BencodeT> {
    fn to_BencodeT(self) -> BencodeT {BencodeT::Dictionary(self)}
    fn from_BencodeT(bencode_t : &BencodeT) -> Result<HashMap<String, BencodeT>, ParseError> {
        match bencode_t {
            &BencodeT::Dictionary(ref hm) => Ok(hm.clone()),
            _ => Err(ParseError::new_str("Attempted to convert non-dictionary BencodeT to hashmap"))
        }
    }
}

impl Bencodable for Vec<BencodeT> {
    fn to_BencodeT(self) -> BencodeT {BencodeT::List(self)}
    fn from_BencodeT(bencode_t : &BencodeT) -> Result<Vec<BencodeT>, ParseError> {
        match bencode_t {
            &BencodeT::List(ref list) => Ok(list.clone()),
            _ => Err(ParseError::new_str("Attempted to convert non-list BencodeT to vector"))
        }
    }
}

impl Bencodable for i64 {
    fn to_BencodeT(self) -> BencodeT {BencodeT::Integer(self)}
    fn from_BencodeT(bencode_t : &BencodeT) -> Result<i64, ParseError> {
        match bencode_t {
            &BencodeT::Integer(ref i) => Ok(i.clone()),
            _ => Err(ParseError::new_str("Attempted to convert non-integer BencodeT to integer"))
        }
    }
}

impl Bencodable for u32 {
    fn to_BencodeT(self) -> BencodeT {BencodeT::Integer(self as i64)}
    fn from_BencodeT(bencode_t : &BencodeT) -> Result<u32, ParseError> {
        match bencode_t {
            &BencodeT::Integer(ref i) => {
                if *i < 0 {Err(ParseError::new_str("Attempted to convert negative BencodeT::Integer to u32"))}
                else if *i > 2^32 - 1 {Err(ParseError::new_str("BencodeT::Integer is too large to convert to u32"))}
                else {Ok(i.clone() as u32)}
            },
            _ => Err(ParseError::new_str("Attempted to convert non-integer BencodeT to integer"))
        }
    }
}

impl Bencodable for u8 {
    fn to_BencodeT(self) -> BencodeT {BencodeT::Integer(self as i64)}
    fn from_BencodeT(bencode_t : &BencodeT) -> Result<u8, ParseError> {
        match bencode_t {
            &BencodeT::Integer(ref i) => {
                if *i < 0 {Err(ParseError::new_str("Attempted to convert negative BencodeT::Integer to u8"))}
                else if *i > 2^8 - 1 {Err(ParseError::new_str("BencodeT::Integer is too large to convert to u8"))}
                else {Ok(i.clone() as u8)}
            },
            _ => Err(ParseError::new_str("Attempted to convert non-integer BencodeT to integer"))
        }
    }
}

impl Bencodable for bool {
    fn to_BencodeT(self) -> BencodeT {BencodeT::Integer(self as i64)}
    fn from_BencodeT(bencode_t : &BencodeT) -> Result<bool, ParseError> {
        match bencode_t {
            &BencodeT::Integer(ref i) => {
                if *i == 0 {Ok(false)}
                else if *i == 1 {Ok(true)}
                else {Err(ParseError::new_str("Can't interpret BencodeT::Integer as bool"))}
            },
            _ => Err(ParseError::new_str("Attempted to convert non-integer BencodeT to bool"))
        }
    }
}

#[derive(PartialEq, Debug, Clone)]
pub struct Piece {
    index : u32,
    begin : u32,
    length : u32,
}

#[derive(PartialEq, Debug, Clone)]
pub struct PieceData<'a>{
    piece : Piece,
    //info_hash : [u8 ; 20], // note: not sure why this was here, but leaving in case
    data : & 'a[u8],
}