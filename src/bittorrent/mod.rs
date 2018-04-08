use std::collections::HashMap;
use std::error::Error;
use std::path::Path;
use std::fmt::Display;
use std::fmt;
use std::io::Error as IOError;
use hyper::error::Error as HyperError;
use hyper::error::UriError;
use std::default::Default;
use serde::{Deserialize, Serialize};

/// Describes a decoded benencodable object
#[derive(PartialEq, Debug, Clone)]
pub enum BencodeT {
    String(String),
    Integer(i64),
    Dictionary(HashMap<String, BencodeT>),
    List(Vec<BencodeT>),
}

#[derive(Debug)]
pub enum ParseError {
    Parse(String),
    IO(String, IOError),
    Hyper(String, HyperError),
    Uri(String, UriError),
}

#[derive(Hash, Eq, PartialEq, Clone, Debug, Default, Copy, Serialize, Deserialize)]
pub struct Hash([u8; 20]);

impl From<[u8; 20]> for Hash {
    fn from(bytes: [u8; 20]) -> Self {
        Hash(bytes)
    }
}

impl Into<[u8; 20]> for Hash {
    fn into(self) -> [u8; 20] {
        self.0
    }
}

fn byte_to_nibbles(byte: u8) -> (u8, u8) {
    let a = byte & 0b00001111;
    let b = byte >> 4;
    (b, a)
}

impl Display for Hash {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let string: String = self.0
            .iter()
            .map(|x| {
                let (b, a) = byte_to_nibbles(*x);
                format!("{:x}{:x}", b, a)
            })
            .collect();
        write!(f, "{}", string)
    }
}

// TODO: this needs an overhaul (boxing errors? using traits?)
impl ParseError {
    pub fn new(string: String) -> ParseError {
        ParseError::Parse(string)
    }

    pub fn new_str(string: &str) -> ParseError {
        ParseError::Parse(String::from(string))
    }

    pub fn new_cause(string: &str, cause: IOError) -> ParseError {
        ParseError::IO(String::from(string), cause)
    }

    pub fn new_hyper(string: &str, cause: HyperError) -> ParseError {
        ParseError::Hyper(String::from(string), cause)
    }

    pub fn new_uri(string: &str, cause: UriError) -> ParseError {
        ParseError::Uri(String::from(string), cause)
    }

    pub fn from_parse(string: &str, cause: ParseError) -> ParseError {
        ParseError::Parse(format!("{} : {}", string, cause.description()))
    }

    pub fn from_parse_string(string: String, cause: ParseError) -> ParseError {
        ParseError::Parse(format!("{} : {}", string, cause.description()))
    }
}

impl From<IOError> for ParseError {
    fn from(error: IOError) -> Self {
        ParseError::new_cause("", error)
    }
}

impl From<HyperError> for ParseError {
    fn from(error: HyperError) -> Self {
        ParseError::new_hyper("", error)
    }
}

impl From<String> for ParseError {
    fn from(error: String) -> Self {
        ParseError::new(error)
    }
}

impl From<UriError> for ParseError {
    fn from(error: UriError) -> Self {
        ParseError::new_uri("", error)
    }
}

impl Error for ParseError {
    fn description(&self) -> &str {
        match self {
            &ParseError::Parse(ref string) => string.as_str(),
            &ParseError::IO(ref string, ref ioerror) => string.as_str(),
            &ParseError::Hyper(ref string, ref error) => string.as_str(),
            &ParseError::Uri(ref string, ref error) => string.as_str(),
        }
    }

    fn cause(&self) -> Option<&Error> {
        match self {
            &ParseError::Parse(ref string) => None,
            &ParseError::IO(ref string, ref ioerror) => Some(ioerror),
            &ParseError::Hyper(ref string, ref error) => Some(error),
            &ParseError::Uri(ref string, ref error) => Some(error),
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            &ParseError::Parse(ref string) => write!(f, "{}", string),
            &ParseError::IO(ref string, ref ioerror) => {
                write!(f, "{}; Cause: {:?}", string, ioerror)
            }
            &ParseError::Hyper(ref string, ref error) => {
                write!(f, "{}; Cause: {:?}", string, error)
            }
            &ParseError::Uri(ref string, ref error) => write!(f, "{}; Cause: {:?}", string, error),
        }
    }
}

/// Keys has two implementations: for Option<T : Bencodable> or  T: Bencodable
/// Meant to be able to handle those types generically when converting from Bencoded dict
pub trait Keys<T: Bencodable> {
    fn keys<'a>(opt: Option<&'a BencodeT>) -> Result<Self, ParseError>
    where
        Self: Sized;
    fn to_value(self) -> Option<T>
    where
        T: Sized;
}

impl<T: Bencodable> Keys<T> for Option<T> {
    fn keys<'a>(opt: Option<&'a BencodeT>) -> Result<Option<T>, ParseError> {
        if opt.is_some() {
            Ok(Some(T::from_BencodeT(opt.unwrap())?))
        } else {
            Ok(None)
        }
    }
    fn to_value(self) -> Option<T> {
        self
    }
}

impl<T: Bencodable> Keys<T> for T {
    fn keys<'a>(opt: Option<&'a BencodeT>) -> Result<Self, ParseError> {
        T::from_BencodeT(opt.unwrap())
    }
    fn to_value(self) -> Option<T> {
        Some(self)
    }
}

// convert _ to space
macro_rules! mod_stringify {
    ($key:ident) => {
        &stringify!($key).replace('_'," ")
    }
}

/// Used to construct a struct from entries in a BencodeT Hashmap
macro_rules! get_keys {
    ($Struct:ident, $hm:expr, $(($key:ident, $T:ident)),*) => {
        $Struct{
        $(
            $key: $T::keys($hm.get(mod_stringify!($key)))?,
        )*
        }
    }
}

macro_rules! get_keys_enum {
    ($Enu:ident, $variant:ident, $hm:expr, $(($key:ident, $T:ident)),*) => {
        $Enu::$variant{
        $(
            $key: $T::keys($hm.get(mod_stringify!($key)))?,
        )*
        }
    }
}

/// Used to turn into bencoded objects
macro_rules! to_keys {
    ($(($key:ident, $T:ident)),*) => {
        {
            let mut hm = HashMap::new();
            $(
                if let Some(v) = $key.to_value() {
                    hm.insert(mod_stringify!($key).to_string(), v.to_BencodeT());
                }

            )*
            hm
        }
    }
}

/// Like to_keys, but takes the elements of $struct
macro_rules! to_keys_serialize {
    ($struct:ident, $(($key:ident, $T:ident)),*) => {
        {
            let mut hm = HashMap::new();
            $(
                if let Some(v) = $struct.$key.clone().to_value() {
                    hm.insert(mod_stringify!($key).to_string(), v.to_BencodeT());
                }

            )*
            hm
        }
    }
}

/// Represents a type that can be serialized by bencoding
pub trait Bencodable: Clone {
    /// Converts self to a BencodeT object
    fn to_BencodeT(self) -> BencodeT;
    /// Converts a BencodeT object to Self
    fn from_BencodeT(bencode_t: &BencodeT) -> Result<Self, ParseError>
    where
        Self: Sized;
}

macro_rules! parse_error {
    ($ ( $ arg : expr ), *) => {
        ParseError::new(
            format!($( $arg),*)
        )
    }
}

impl Bencodable for String {
    fn to_BencodeT(self) -> BencodeT {
        BencodeT::String(self)
    }
    fn from_BencodeT(bencode_t: &BencodeT) -> Result<String, ParseError> {
        match bencode_t {
            &BencodeT::String(ref string) => Ok(string.clone()),
            &BencodeT::Integer(int) => Err(parse_error!(
                "Attempted to convert int BencodeT to string: {:?}",
                int
            )),
            &BencodeT::Dictionary(ref hm) => Err(parse_error!(
                "Attempted to convert dict BencodeT to string: {:?}",
                hm
            )),
            &BencodeT::List(ref vec) => Err(parse_error!(
                "Attempted to convert list BencodeT to string: {:?}",
                vec
            )),
            _ => Err(ParseError::new_str(
                "Attempted to convert non-string BencodeT to string",
            )),
        }
    }
}

impl Bencodable for Hash {
    fn to_BencodeT(self) -> BencodeT {
        return unsafe { BencodeT::String(String::from_utf8_unchecked(self.0.to_vec())) };
    }
    fn from_BencodeT(bencode_t: &BencodeT) -> Result<Hash, ParseError> {
        match bencode_t {
            &BencodeT::String(ref string) => {
                let mut a: Hash = Default::default();
                a.0.copy_from_slice(string.as_bytes());
                Ok(a)
            }
            _ => Err(parse_error!(
                "Attempted to convert non-string BencodeT to Hash: {:?}",
                bencode_t
            )),
        }
    }
}

impl Bencodable for HashMap<String, BencodeT> {
    fn to_BencodeT(self) -> BencodeT {
        BencodeT::Dictionary(self)
    }
    fn from_BencodeT(bencode_t: &BencodeT) -> Result<HashMap<String, BencodeT>, ParseError> {
        match bencode_t {
            &BencodeT::Dictionary(ref hm) => Ok(hm.clone()),
            _ => Err(ParseError::new_str(
                "Attempted to convert non-dictionary BencodeT to hashmap",
            )),
        }
    }
}

impl Bencodable for Vec<BencodeT> {
    fn to_BencodeT(self) -> BencodeT {
        BencodeT::List(self)
    }
    fn from_BencodeT(bencode_t: &BencodeT) -> Result<Vec<BencodeT>, ParseError> {
        match bencode_t {
            &BencodeT::List(ref list) => Ok(list.clone()),
            _ => Err(ParseError::new_str(
                "Attempted to convert non-list BencodeT to vector",
            )),
        }
    }
}

impl<T: Bencodable> Bencodable for Vec<T> {
    fn to_BencodeT(self) -> BencodeT {
        BencodeT::List(self.into_iter().map(|x| x.to_BencodeT()).collect())
    }
    fn from_BencodeT(bencode_t: &BencodeT) -> Result<Vec<T>, ParseError> {
        match bencode_t {
            &BencodeT::List(ref list) => {
                let mut vec = Vec::new();
                for elem in list {
                    vec.push(T::from_BencodeT(elem)?);
                }
                Ok(vec)
            }
            _ => Err(ParseError::new_str(
                "Attempted to convert non-list BencodeT to vector",
            )),
        }
    }
}

impl Bencodable for i64 {
    fn to_BencodeT(self) -> BencodeT {
        BencodeT::Integer(self)
    }
    fn from_BencodeT(bencode_t: &BencodeT) -> Result<i64, ParseError> {
        match bencode_t {
            &BencodeT::Integer(ref i) => Ok(i.clone()),
            _ => Err(ParseError::new_str(
                "Attempted to convert non-integer BencodeT to integer",
            )),
        }
    }
}

impl Bencodable for usize {
    fn to_BencodeT(self) -> BencodeT {
        BencodeT::Integer(self as i64)
    }
    fn from_BencodeT(bencode_t: &BencodeT) -> Result<usize, ParseError> {
        match bencode_t {
            &BencodeT::Integer(ref i) => {
                if *i < 0 {
                    Err(parse_error!(
                        "Attempted to convert negative BencodeT::Integer to usize"
                    ))
                } else {
                    Ok(i.clone() as usize)
                }
            }
            _ => Err(ParseError::new_str(
                "Attempted to convert non-integer BencodeT to integer",
            )),
        }
    }
}

impl Bencodable for u32 {
    fn to_BencodeT(self) -> BencodeT {
        BencodeT::Integer(self as i64)
    }
    fn from_BencodeT(bencode_t: &BencodeT) -> Result<u32, ParseError> {
        match bencode_t {
            &BencodeT::Integer(ref i) => {
                if *i < 0 {
                    Err(ParseError::new_str(
                        "Attempted to convert negative BencodeT::Integer to u32",
                    ))
                } else if *i > 2 ^ 32 - 1 {
                    Err(ParseError::new_str(
                        "BencodeT::Integer is too large to convert to u32",
                    ))
                } else {
                    Ok(i.clone() as u32)
                }
            }
            _ => Err(ParseError::new_str(
                "Attempted to convert non-integer BencodeT to integer",
            )),
        }
    }
}

impl Bencodable for u8 {
    fn to_BencodeT(self) -> BencodeT {
        BencodeT::Integer(self as i64)
    }
    fn from_BencodeT(bencode_t: &BencodeT) -> Result<u8, ParseError> {
        match bencode_t {
            &BencodeT::Integer(ref i) => {
                if *i < 0 {
                    Err(ParseError::new_str(
                        "Attempted to convert negative BencodeT::Integer to u8",
                    ))
                } else if *i > 2 ^ 8 - 1 {
                    Err(ParseError::new_str(
                        "BencodeT::Integer is too large to convert to u8",
                    ))
                } else {
                    Ok(i.clone() as u8)
                }
            }
            _ => Err(ParseError::new_str(
                "Attempted to convert non-integer BencodeT to integer",
            )),
        }
    }
}

impl Bencodable for bool {
    fn to_BencodeT(self) -> BencodeT {
        BencodeT::Integer(self as i64)
    }
    fn from_BencodeT(bencode_t: &BencodeT) -> Result<bool, ParseError> {
        match bencode_t {
            &BencodeT::Integer(ref i) => {
                if *i == 0 {
                    Ok(false)
                } else if *i == 1 {
                    Ok(true)
                } else {
                    Err(ParseError::new_str(
                        "Can't interpret BencodeT::Integer as bool",
                    ))
                }
            }
            _ => Err(ParseError::new_str(
                "Attempted to convert non-integer BencodeT to bool",
            )),
        }
    }
}

#[derive(PartialEq, Debug, Clone, Hash, Eq)]
pub struct Piece {
    index: u32,
    begin: u32,
    length: u32,
}

impl Piece {
    pub fn new(index: u32, begin: u32, length: u32) -> Piece {
        Piece {
            index: index,
            begin: begin,
            length: length,
        }
    }

    pub fn file_index(&self, piece_length: i64) -> i64 {
        self.begin as i64 + (self.index as i64 * piece_length)
    }
}

#[derive(PartialEq, Debug, Clone)]
pub struct PieceData {
    piece: Piece,
    data: Vec<u8>,
}

mod bencoder;
mod bedecoder;
mod utils;
mod torrent;
mod peer;
mod tracker;
mod metainfo;
pub mod manager;
