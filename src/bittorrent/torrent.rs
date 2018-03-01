use bittorrent::{Piece, PieceData, metainfo::MetaInfo, ParseError};
use std::collections::HashMap;
use bit_vec::BitVec;

/* Need to:
- Maintain file access to downloading/uploading data; 
  should probably cache in memory
- Keep track of partial download of pieces
- Maintain our bitfield

Pieces are treated as part of a single file,
so also need to abstract away file boundaries
*/

#[derive(PartialEq, Debug, Clone)]
/// Represents a downloading torrent and assoicated file operations
pub struct Torrent {
    metainfo: MetaInfo,
    bitfield: BitVec,
    map : HashMap<u32, Vec<u32>>, // Piece indices -> indices within piece
}

impl Torrent {

    pub fn new(path : String) -> Result<Torrent, ParseError> {
        match MetaInfo::read(path) {
            Ok(metainfo) => {
                Ok(Torrent {
                    metainfo : metainfo,
                    bitfield : BitVec::new(),
                    map : HashMap::new(),
                })
            },
            Err(e) => Err(ParseError::from_parse_string(String::from("Error adding torrent"), e))
        }
    }

    pub fn write_block(&mut self, piece : PieceData){
        unimplemented!();
    }

    pub fn read_block<'a>(&mut self, piece : &'a Piece) -> PieceData {
        unimplemented!();
    }

    pub fn info_hash(&self) -> [u8; 20] {
        return self.metainfo.info_hash();
    }

    /// Update data structure to reflect parts of piece
    /// that have been downloaded
    fn insertPiece(){
        unimplemented!();
    }
}