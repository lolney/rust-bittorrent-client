use std::collections::HashMap; 

mod bencoder;
mod bedecoder;
mod utils;
mod Peer;
mod Trackers;
mod Metainfo;

#[derive(PartialEq, Debug, Clone)]
pub enum BencodeT {
    String(String),
    Integer(i64),
    Dictionary(HashMap<String, BencodeT>),
    List(Vec<BencodeT>),
}

pub trait Bencodable {
    fn to_BencodeT(self) -> BencodeT;
}

impl Bencodable for String {
    fn to_BencodeT(self) -> BencodeT {BencodeT::String(self)}
}

impl Bencodable for HashMap<String, BencodeT> {
    fn to_BencodeT(self) -> BencodeT {BencodeT::Dictionary(self)}
}

impl Bencodable for Vec<BencodeT> {
    fn to_BencodeT(self) -> BencodeT {BencodeT::List(self)}
}

impl Bencodable for i64 {
    fn to_BencodeT(self) -> BencodeT {BencodeT::Integer(self)}
}

impl Bencodable for i32 {
    fn to_BencodeT(self) -> BencodeT {BencodeT::Integer(self as i64)}
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
    data : & 'a[u8],
}