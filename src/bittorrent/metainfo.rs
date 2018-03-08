use std::collections::HashMap;
use bittorrent::{BencodeT, Bencodable, Peer, ParseError, PieceData, Piece};
use bittorrent::bencoder::{encode};
use bittorrent::bedecoder::{parse};
use std::fs::File;
use std::path::Path;
use std::io::BufReader;
use std::io::prelude::*;
use sha1::Sha1;
use std::str;
use std::error::Error;
use std::io::Error as IOError;

/*
Ideally the methods in this file would use a macro that enumerated the fields
of each struct, then used the Bencodable trait to automatically convert to the right type.
A few this make this difficult - e.g. that BencodeT is implemented as a struct,
so you can't implement from_BencodeT for each subtype
*/

/// MetaInfo: Representation of a .torrent file, 
/// containing hashes of all pieces and other metadata
#[derive(PartialEq, Debug, Clone)]
pub struct MetaInfo {
    info : Info,
    announce : Option<String>,
    announce_list : Option<Vec<String>>,
    creation_date : Option<String>,
    comment : Option<String>,
    created_by : Option<String>,
    encoding : Option<String>,
}

/// Common traits to single- and multi- fileinfo
#[derive(PartialEq, Debug, Clone)]
pub struct Info{
    pub piece_length : i64,
    pieces: Vec<[u8 ; 20]>, // concatination of all 20-byte SHA-1 hash values
    private: Option<bool>,
    file_info: FileInfo,
}

#[derive(PartialEq, Debug, Clone)]
enum FileInfo {
    SingleFileInfo {
        name: String,
        length: i64,
        md5sum: Option<String>
    },
    MultiFileInfo {
        name: String,
        files: Vec<MIFile>,
    }
}

/// Component of a multi-file info
#[derive(PartialEq, Debug, Clone)]
struct MIFile {
    length: i64,
    md5sum: Option<String>,
    path: Vec<String>, 
}

/// Converts non-optional string fields to BencodeT
macro_rules! string_field {
    ($t:expr,$hm:expr,$($x:ident),*) => {
        $($hm.insert(String::from(stringify!($x)), $t.$x.to_BencodeT());)*
    }
}

macro_rules! insert_elems {
    ($hm:expr,$($x:ident),*) => {
        $($hm.insert(String::from(stringify!($x)), $x.clone().to_BencodeT());)*
    }
}

/// Used in to_BencodeT to convert optional string fields to BencodeT
macro_rules! string_optional {
    ($t:expr,$hm:expr,$($x:ident),*) => {
        $(match $t.$x {
            Some(ref string) => {$hm.insert(String::from(stringify!($x)), string.clone().to_BencodeT());},
            None => {},
        })*
    }
}

macro_rules! insert_elems_optional {
    ($hm:expr,$($x:ident),*) => {
        $(match $x {
            &Some(ref string) => {$hm.insert(String::from(stringify!($x)), string.clone().to_BencodeT());},
            &None => {},
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

/// In to_BencodeT: inserts a vector into the hm, converting elems to BencodeT
fn insert_vector<T : Bencodable>(hm : &mut HashMap<String, BencodeT>, string : String, vec : &Vec<T>) {
    let bvec = vec.iter().map(|x| x.clone().to_BencodeT()).collect();
    hm.insert(String::from(string), BencodeT::List(bvec));
}

/// In from_BencodeT: inserts a vector into the hm, converting elems to BencodeT
fn elem_from_entry<T : Bencodable>(hm : &HashMap<String, BencodeT>, string : &str) -> Option<T> {
    match hm.get(string){
        Some(string) => Some(T::from_BencodeT(string).unwrap()),
        None => None,
    }
}

/// Unwraps double-wrapped Bencoded lists
fn convert(b : BencodeT) -> Result<String, ParseError> {
    let vec : Vec<String> = _checkVec(Vec::from_BencodeT(&b)?)?;
    Ok(vec[0].clone())
}

// Need to check that the vector is of the required type
// Try to convert if necessary
fn checkVec<T: Bencodable>(vec : Vec<BencodeT>, convert : fn(BencodeT) ->
        Result<T, ParseError>) -> Result<Vec<T>, ParseError> {

    let mut out : Vec<T> = Vec::new();
    for elem in vec {
        match T::from_BencodeT(&elem) {
            Ok(val) => { out.push(val) }
            Err(error) => {
                match convert(elem) {
                    Ok(val2) => { out.push(val2) }
                    Err(error2) => {
                        let string = format!("Vector is of incorrect type : {}", error2.description());
                        return Err(ParseError::from_parse_string(string, error));
                    }
                }
            }
        }
    }
    Ok(out)
}

fn _checkVec<T: Bencodable>(vec : Vec<BencodeT>) -> Result<Vec<T>, ParseError> {

    let mut out : Vec<T> = Vec::new();
    for elem in vec {
        match T::from_BencodeT(&elem) {
            Ok(val) => { out.push(val) }
            Err(error) => {
                let string = format!("Vector is of incorrect type : {}", error.description());
                return Err(ParseError::from_parse_string(string, error));
            }
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

impl FileInfo {
    pub fn from_BencodeT(hm : &HashMap<String, BencodeT>) -> Result<FileInfo, ParseError> {
        let name = String::from_BencodeT(hm.get("name").unwrap())?;
        match hm.get("length") {
            Some(i) => { // singlefile 
                Ok(
                    FileInfo::SingleFileInfo{
                        name: name,
                        length: i64::from_BencodeT(i)?,
                        md5sum: elem_from_entry(hm, "md5sum"),
                    }
                )
            },
            None => { // multifile
                let files = hm.get("files").unwrap();
                Ok(
                    FileInfo::MultiFileInfo{
                        name: name,
                        files: {
                            let vec = hm.get("files").unwrap();
                            checkVec(Vec::from_BencodeT(vec)?, |x| MIFile::from_BencodeT(&x))?
                        }
                    }
                )
            }
        }
    }

    /// Unlike other to_BencodeT methods, this just fills the hm,
    /// since FileInfo is just a component of FileInfo
    pub fn to_BencodeT(&self, hm : &mut HashMap<String, BencodeT>) {

        match self {
            &FileInfo::SingleFileInfo{ref name, length, ref md5sum} => {
                insert_elems!(hm, name, length);
                insert_elems_optional!(hm, md5sum);
            },
            &FileInfo::MultiFileInfo{ref name, ref files} => {
                insert_elems!(hm, name);
                insert_vector(hm, "files".to_string(), &files);
            },
        }
    }
}

impl Bencodable for MIFile {

    fn to_BencodeT(self) -> BencodeT {
        let mut hm = HashMap::new();

        string_field!(self, hm, length);
        string_optional!(self, hm, md5sum);
        insert_vector(&mut hm, "path".to_string(), &self.path);

        BencodeT::Dictionary(hm)
    }

    fn from_BencodeT(bencode_t : &BencodeT) -> Result<MIFile, ParseError> {
        match bencode_t {
            &BencodeT::Dictionary(ref hm) => Ok(
                MIFile{
                    length : i64::from_BencodeT(hm.get("length").unwrap())?,
                    md5sum : elem_from_entry(hm, "md5sum"),
                    path : {
                        let vec = hm.get("path").unwrap();
                        checkVec(Vec::from_BencodeT(vec)?, |x| String::from_BencodeT(&x))?
                    }
                }
            ),
            _ => Err(ParseError::new_str("Multifile info not formatted as dictionary"))
        }
    }
}

// TODO: error handling when a field isn't present
impl MetaInfo {

    pub fn info_hash(&self) -> [u8 ; 20] {
        let bstring = encode(&self.info.to_BencodeT());
        let mut sha = Sha1::new();
        sha.update(bstring.as_bytes());
        sha.digest().bytes()
    }

    pub fn info(&self) -> &Info {
        &self.info
    }

    pub fn read(infile : String) -> Result<MetaInfo, ParseError> {
        let file = File::open(infile)?;
        let mut buf_reader = BufReader::new(file);
        let mut contents = Vec::new();
        buf_reader.read_to_end(&mut contents)?;

        match parse(&contents){
            Ok(bencodet) => MetaInfo::from_BencodeT(&bencodet),
            Err(err) => Err(ParseError::new(err)),
        }
    }

    pub fn write(&self, outfile : String) -> Result<(), IOError>{
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
                    announce : elem_from_entry(hm, "announce"),
                    announce_list : match hm.get("announce-list") {
                        Some(vec) => Some(checkVec(Vec::from_BencodeT(vec)?, convert)?),
                        None => None
                    },
                    creation_date : elem_from_entry(hm, "creation date"),
                    comment : elem_from_entry(hm, "comment"),
                    created_by : elem_from_entry(hm, "created by"),
                    encoding : elem_from_entry(hm, "encoding"),
                }
            ),
            _ => Err(ParseError::new_str("MetaInfo file not formatted as dictionary"))
        }
    }

    pub fn to_BencodeT(&self) -> BencodeT {
        let mut hm = HashMap::new();

        let info = self.info.to_BencodeT();
        hm.insert(String::from("info"), info);

        match &self.announce_list {
            &Some(ref vec) => {
                insert_vector(&mut hm, "announce-list".to_string(), vec);
            }
            &None => {} 
        }

        string_optional!(self, hm, announce, creation_date, comment, created_by, encoding);

        BencodeT::Dictionary(hm)
    }
}

impl Info {

    /* For decoding info objects that are bencoded */
    pub fn from_BencodeT(bencode_t : &BencodeT) -> Result<Info, ParseError> {
        match bencode_t {
            &BencodeT::Dictionary(ref hm) => {
                Ok(
                    Info {
                        piece_length : i64::from_BencodeT(hm.get("piece length").unwrap())?,
                        pieces : {
                            let string = String::from_BencodeT(hm.get("pieces").unwrap())?;
                            Info::split_hashes(string)?
                        },
                        private : elem_from_entry(hm, "private"),
                        file_info : FileInfo::from_BencodeT(hm)?,
                    }
                )
            },
            _ => Err(ParseError::new_str("Attempted to convert non-dictionary BencodeT to Info"))
        }
    }

    fn split_hashes(string : String) -> Result<Vec<[u8; 20]>, ParseError>{
        
        if string.len() % 20 != 0 {
            return Err(ParseError::new_str("pieces string must be multiple of 20"));
        }

        let mut vec = Vec::<[u8; 20]>::new();

        for i in 0 .. string.len()/20 {
            let s = i*20;
            let e = (i+1)*20;
            let mut a : [u8;20] = Default::default();
            a.copy_from_slice(&string.as_bytes()[s..e]);
            vec.push(a);
        }

        Ok(vec)
    }

    fn hashes_to_string(hashes : &Vec<[u8; 20]>) -> BencodeT {
        let mut vec = Vec::<u8>::new();
        for slice in hashes {
            vec.extend_from_slice(slice);
        }
        return unsafe {
            BencodeT::String(String::from_utf8_unchecked(vec))
        };
    }

    pub fn to_BencodeT(&self) -> BencodeT {
        let mut hm = HashMap::new();

        hm.insert(String::from("piece length"), self.piece_length.to_BencodeT());
        hm.insert(String::from("pieces"), Info::hashes_to_string(&self.pieces));
        string_optional!(self, hm, private);

        self.file_info.to_BencodeT(&mut hm);

        BencodeT::Dictionary(hm)
    }
}


#[test]
fn test_read_write() {
    let metainfo = MetaInfo::read("bible.torrent".to_string()).unwrap();
    metainfo.write("bible-out.torrent".to_string());
    let metainfo2 = MetaInfo::read("bible-out.torrent".to_string()).unwrap();
    assert_eq!(metainfo, metainfo2);
}

#[test]
fn test_pieces() {
    let bytes : [u8; 40] = [54, 30, 209, 250, 31, 227, 163, 34, 205, 182, 4, 37, 119, 22, 3, 185, 16, 53, 29, 166,
                            204, 177, 1, 160, 101, 203, 150, 69, 169, 79, 86, 153, 37, 219, 218, 106, 227, 35, 24, 1];
    let string = unsafe {
        str::from_utf8_unchecked(&bytes).to_string()
    };
    let vec = Info::split_hashes(string.clone()).unwrap();
    let bstring = Info::hashes_to_string(&vec);

    match &bstring {
        &BencodeT::String(ref string2) => {
            assert_eq!(string.as_bytes(), string2.as_bytes());
        }
        _ => {}
    }
    assert_eq!(BencodeT::String(string), bstring);
}