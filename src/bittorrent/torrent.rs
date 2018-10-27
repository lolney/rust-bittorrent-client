use bit_vec::BitVec;
use bittorrent::{metainfo::BTFile, metainfo::MetaInfo, Hash, ParseError, Piece, PieceData};
use serde::de::{Deserialize, Deserializer};
use serde::ser::Serializer;
use std::cmp;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::fs::{create_dir_all, File, OpenOptions};
use std::io::Error as IOError;
use std::io::ErrorKind;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;

#[derive(PartialEq, Debug, Clone, Serialize, Deserialize)]
/// Represents persistent elements of a torrent and associated filesystem operations
pub struct Torrent {
    metainfo: MetaInfo,
    #[serde(deserialize_with = "deserialize_from_bytes")]
    #[serde(serialize_with = "serialize_to_bytes")]
    bitfield: BitVec, // pieces we've downloaded
    map: HashMap<u32, Vec<DLMarker>>, // Piece indices -> indices indicated downloaded parts
    files: Vec<BTFile>,               // contains path (possibly renamed from metainfo) info
}

fn serialize_to_bytes<S>(bv: &BitVec, s: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    s.serialize_bytes(&bv.to_bytes())
}

fn deserialize_from_bytes<'de, D>(deserializer: D) -> Result<BitVec, D::Error>
where
    D: Deserializer<'de>,
{
    let s: Vec<u8> = Deserialize::deserialize(deserializer)?;
    Ok(BitVec::from_bytes(&s))
}

#[derive(PartialEq, Debug, Clone)]
struct FilePiece {
    path: PathBuf,
    begin: i64, // following convention of i64 for files
    length: i64,
}

#[derive(PartialEq, Debug, Clone, Deserialize, Serialize)]
enum DLMarker {
    Begin(u32),
    End(u32),
}

impl DLMarker {
    fn val(&self) -> u32 {
        match self {
            &DLMarker::Begin(val) => val,
            &DLMarker::End(val) => val,
        }
    }
}

impl Torrent {
    pub fn new(
        infofile: &str,
        download_dir: &str,
        downloaded: bool,
    ) -> Result<Torrent, ParseError> {
        match MetaInfo::read(infofile) {
            Ok(metainfo) => {
                let files = metainfo.info().file_info.as_BTFiles(download_dir);
                let npieces = metainfo.npieces();
                let mut torrent = Torrent {
                    bitfield: BitVec::from_elem(npieces as usize, downloaded),
                    metainfo: metainfo,
                    map: Default::default(),
                    files: files,
                };
                // Verify already downloaded files
                if downloaded {
                    if let Err(e) = torrent.verify_files() {
                        Err(ParseError::from_parse("Failed to create torrent", e))
                    } else {
                        Ok(torrent)
                    }
                } else {
                    Ok(torrent)
                }
            }
            Err(e) => Err(ParseError::from_parse_string(
                String::from("Error adding torrent"),
                e,
            )),
        }
    }

    fn verify_files(&mut self) -> Result<(), ParseError> {
        let n = self.metainfo.npieces();
        for i in 0..n {
            let piece = Piece {
                index: i as u32,
                begin: 0,
                length: if i == n - 1 {
                    self.metainfo.last_piece_length()
                } else {
                    self.piece_length() as u32
                },
            };
            let piece_data = self.read_block(&piece)?;
            if !self.metainfo.valid_piece(&piece_data) {
                return Err(parse_error!("Piece in supplied file is invalid"));
            }
        }
        return Ok(());
    }

    /// Clone the metainfo
    pub fn metainfo(&self) -> MetaInfo {
        self.metainfo.clone()
    }

    pub fn trackers(&self) -> Vec<String> {
        self.metainfo.trackers()
    }

    pub fn name(&self) -> &String {
        self.metainfo.name()
    }

    pub fn piece_length(&self) -> i64 {
        self.metainfo.info().piece_length
    }

    pub fn bitfield(&self) -> &BitVec {
        &self.bitfield
    }

    /// Returns number of bytes downloaded
    pub fn downloaded(&self) -> usize {
        let n = self.bitfield.len();
        let last = if self.bitfield[n - 1] { 1 } else { 0 };
        let count = self.bitfield.iter().take(n - 1).filter(|x| *x).count();
        (last * self.metainfo.last_piece_length() as usize) + (count * self.piece_length() as usize)
    }

    /// Returns number of bytes remaining
    pub fn remaining(&self) -> usize {
        let dl = self.downloaded();
        let size = self.metainfo.total_size();
        size - dl
    }

    /// Returns the total size in bytes of the torrent
    pub fn size(&self) -> usize {
        self.metainfo.total_size()
    }

    pub fn npieces(&self) -> usize {
        self.bitfield.len()
    }

    pub fn default_piece(&self, index: usize) -> Piece {
        if index == self.npieces() - 1 {
            Piece::new(index as u32, 0, self.metainfo.last_piece_length() as u32)
        } else {
            Piece::new(index as u32, 0, self.piece_length() as u32)
        }
    }

    /// For each of the files specified in the torrent file, create it and parent directories
    pub fn create_files(&self) -> Result<(), IOError> {
        for file in self.files.iter() {
            create_dir_all(file.path.parent().unwrap())?;
            let f = File::create(file.path.clone())?;
            f.set_len(file.length as u64)?;
        }

        Ok(())
    }

    /// Write block after verifying that hash is correct
    pub fn write_block(&mut self, piece: &PieceData) -> Result<(), IOError> {
        if self.metainfo.valid_piece(piece) {
            self.write_block_raw(piece)
        } else {
            Err(IOError::new(
                ErrorKind::Other,
                format!("Invalid block: {:?}", piece.piece),
            ))
        }
    }

    fn write_block_raw(&mut self, piece: &PieceData) -> Result<(), IOError> {
        for file in self.map_files(&piece.piece).iter() {
            let mut options = OpenOptions::new();
            options.write(true);

            let mut fp = match options.open(file.path.clone()) {
                Ok(f) => f,
                Err(_) => {
                    info!("File {:?} doesn't exist; creating", file.path);
                    File::create(file.path.clone())?
                }
            };
            fp.seek(SeekFrom::Start(file.begin as u64))?;
            fp.write_all(piece.data.as_slice());
        }

        self.insert_piece(&piece.piece);

        Ok(())
    }

    /// Read block after verifying that we have it
    pub fn read_block(&mut self, piece: &Piece) -> Result<PieceData, IOError> {
        match self.bitfield.get(piece.index as usize) {
            Some(have) => {
                if have || self.have_block(piece) {
                    self.read_block_raw(piece)
                } else {
                    Err(IOError::new(
                        ErrorKind::Other,
                        format!("Don't yet have requested piece: {:?}", piece),
                    ))
                }
            }
            None => Err(IOError::new(
                ErrorKind::Other,
                format!("Peer requested nonexistent piece: {:?}", piece),
            )),
        }
    }

    fn read_block_raw(&mut self, piece: &Piece) -> Result<PieceData, IOError> {
        let mut vec: Vec<u8> = Vec::new();
        for file in self.map_files(piece).iter() {
            let mut buf = vec![0; file.length as usize];
            let mut fp = File::open(file.path.clone())?;

            fp.seek(SeekFrom::Start(file.begin as u64))?;
            fp.read_exact(buf.as_mut_slice());
            vec.extend_from_slice(buf.as_slice());
        }

        Ok(PieceData {
            piece: piece.clone(),
            data: vec,
        })
    }

    pub fn info_hash(&self) -> Hash {
        return self.metainfo.info_hash();
    }

    /// Determine if all of this block has been downloaded
    fn have_block(&self, piece: &Piece) -> bool {
        match self.map.get(&piece.index) {
            Some(vec) => {
                let mut inrange = false;
                let end = piece.begin + piece.length;
                let begin = piece.begin;

                for index in vec {
                    match index {
                        &DLMarker::Begin(val) => {
                            if !inrange && val <= begin {
                                inrange = true;
                            } else if !inrange && val > begin {
                                break;
                            }
                        }
                        &DLMarker::End(val) => {
                            if inrange && val < end {
                                inrange = false;
                                if val >= begin {
                                    break;
                                }
                            } else if inrange && val >= end {
                                break;
                            }
                        }
                    }
                }

                return inrange;
            }
            None => false,
        }
    }

    /// Determine if dl_marker is in a downloaded area
    /// exclusive; inclusive if end of range and begin flag is true or beginning and begin is false)
    /// of the piece given by piece_index
    fn in_shaded(&self, piece_index: &u32, dlmarker: &u32, begin: bool) -> bool {
        let mut shaded = false;
        let vec: &Vec<DLMarker> = self.map.get(piece_index).unwrap();

        for index in vec {
            match index {
                &DLMarker::Begin(ref val) => {
                    if val == dlmarker && begin {
                        break;
                    } else if val == dlmarker && !begin {
                        shaded = true;
                        break;
                    } else if val > dlmarker {
                        break;
                    }
                    shaded = true;
                }
                &DLMarker::End(ref val) => {
                    if val == dlmarker && begin {
                        break;
                    } else if val == dlmarker && !begin {
                        shaded = false;
                        break;
                    } else if val > dlmarker {
                        break;
                    }
                    shaded = false;
                }
            }
        }
        return shaded;
    }

    /// Update data structure to reflect parts of piece
    /// that have been downloaded
    fn insert_piece(&mut self, piece: &Piece) {
        let max_length: u32 = if piece.index == self.npieces() as u32 - 1 {
            self.metainfo.last_piece_length() as u32
        } else {
            self.piece_length() as u32
        };

        if piece.begin == 0 && piece.length == max_length {
            // Downloaded whole piece at once
            self.bitfield.set(piece.index as usize, true);
            self.map.remove(&piece.index);
        } else {
            // Insert these pieces into the dictionary
            let begin = piece.begin;
            let end = piece.begin + piece.length;
            if begin == end {
                return;
            }

            {
                match self.map.entry(piece.index) {
                    Entry::Occupied(o) => o.into_mut(),
                    Entry::Vacant(v) => v.insert(Default::default()),
                };
            }

            let end_shaded = self.in_shaded(&piece.index, &end, false);
            let begin_shaded = self.in_shaded(&piece.index, &begin, true);

            // Have to reborrow as mut here
            let mut vec = self.map.get_mut(&piece.index).unwrap();

            vec.retain(|index| index.val() < begin || index.val() > end);

            // Decide where to insert - before the next biggest index
            let mut insert = vec.len();
            for (i, index) in vec.iter().enumerate() {
                if index.val() > begin {
                    insert = i;
                    break;
                }
            }
            if !end_shaded {
                vec.insert(insert, DLMarker::End(end));
            }
            if !begin_shaded {
                vec.insert(insert, DLMarker::Begin(begin));
            }
        }
    }

    /// Map piece -> files
    fn map_files(&self, piece: &Piece) -> Vec<FilePiece> {
        let mut vec = Vec::new();
        let piece_length = self.piece_length();

        let mut iter = self.files.iter();
        let mut total: i64 = 0;
        let mut index = piece.file_index(piece_length);
        let mut file: &BTFile = iter.next().unwrap();

        // Find the first file
        while (total + file.length) < index {
            total += file.length;
            file = iter.next().unwrap();
        }

        // Apportion the load of the piece
        let mut remaining = piece.length as i64;
        while remaining > 0 {
            let index_in_file = index - total;
            let load = cmp::min(remaining, file.length - index_in_file);
            remaining -= load as i64;
            total += file.length;
            index += load;

            vec.push(FilePiece {
                path: file.path.clone(),
                begin: index_in_file,
                length: load,
            });
            if remaining == 0 {
                break;
            }
            file = iter.next()
                .expect("Piece length requested was longer than expected");
        }

        return vec;
    }
}

macro_rules! piece {
    ($i:expr, $n:expr) => {
        Piece {
            index: 0,
            begin: $i,
            length: $n,
        }
    };
}

macro_rules! fpiece {
    ($b:expr, $n:expr) => {
        FilePiece {
            path: Default::default(),
            begin: $b as i64,
            length: $n as i64,
        }
    };
}

fn test_vec(torrent: &Torrent, vec2: &Vec<DLMarker>) {
    let vec = torrent.map.get(&0).unwrap();
    assert_eq!(vec2, vec);
}

fn make_pieces() -> Vec<Piece> {
    vec![
        Piece::new(0, 1, 8),  // 0#######
        Piece::new(0, 1, 2),  // 0##0..
        Piece::new(0, 0, 2),  // #000..
        Piece::new(0, 3, 4),  // 000####
        Piece::new(0, 0, 7),  // #######
        Piece::new(0, 0, 8),  // ########
        Piece::new(0, 7, 2),  // 000000###
        Piece::new(0, 10, 1), // 0000000000#
        Piece::new(0, 0, 12), // ############
    ]
}

#[cfg(test)]
mod tests {

    use bittorrent::torrent::*;
    use bittorrent::utils::gen_random_bytes;
    use bittorrent::{metainfo::BTFile, Piece};
    use std::fs::remove_dir_all;
    use std::path::PathBuf;

    #[test]
    fn test_from_downloaded() {
        let res = Torrent::new(
            ::TEST_FILE,
            &format!("{}/{}", ::READ_DIR, "valid_torrent"),
            true,
        );
        assert!(res.is_ok());

        let res = Torrent::new(
            ::TEST_FILE,
            &format!("{}/{}", ::READ_DIR, "invalid_torrent"),
            true,
        );
        assert!(res.is_err());
    }

    #[test]
    fn test_in_range() {
        let p = make_pieces();

        let mut torrent = Torrent::new(::TEST_FILE, "", false).unwrap();

        for b in p.iter() {
            assert!(!torrent.have_block(&b));
        }

        torrent.insert_piece(&p[1]);

        for b in p.iter() {
            if *b != p[1] {
                assert!(!torrent.have_block(&b));
            } else {
                assert!(torrent.have_block(&b));
            }
        }

        torrent.insert_piece(&p[6]);

        for b in p.iter() {
            if *b != p[1] && *b != p[6] {
                assert!(!torrent.have_block(&b));
            } else {
                assert!(torrent.have_block(&b));
            }
        }

        torrent.insert_piece(&p[8]);

        for b in p.iter() {
            assert!(torrent.have_block(&b));
        }
    }

    #[test]
    fn test_insert_piece() {
        use self::DLMarker::Begin as B;
        use self::DLMarker::End as E;

        let p = make_pieces();

        let mut torrent = Torrent::new(::TEST_FILE, "", false).unwrap();

        torrent.insert_piece(&p[1]);
        torrent.insert_piece(&p[7]);

        test_vec(&torrent, &vec![B(1), E(3), B(10), E(11)]);

        // In shaded boundary conditions
        assert!(!torrent.in_shaded(&0, &0, true));
        assert!(!torrent.in_shaded(&0, &1, true));
        assert!(torrent.in_shaded(&0, &2, true));
        assert!(torrent.in_shaded(&0, &3, true));

        assert!(torrent.in_shaded(&0, &1, false));
        assert!(torrent.in_shaded(&0, &2, false));
        assert!(!torrent.in_shaded(&0, &3, false));

        torrent.insert_piece(&p[2]); // ###0
        assert!(torrent.in_shaded(&0, &0, false));
        test_vec(&torrent, &vec![B(0), E(3), B(10), E(11)]);

        torrent.insert_piece(&p[3]); // ######
        test_vec(&torrent, &vec![B(0), E(7), B(10), E(11)]);

        torrent.insert_piece(&p[4]);
        test_vec(&torrent, &vec![B(0), E(7), B(10), E(11)]);

        torrent.insert_piece(&p[5]);
        test_vec(&torrent, &vec![B(0), E(8), B(10), E(11)]);

        torrent.insert_piece(&p[6]);
        test_vec(&torrent, &vec![B(0), E(9), B(10), E(11)]);

        torrent.insert_piece(&p[8]);
        test_vec(&torrent, &vec![B(0), E(12)]);
    }

    #[test]
    fn test_map_files() {
        let mut torrent = Torrent::new(::TEST_FILE, "", false).unwrap();
        let piece_length: u32 = torrent.metainfo.info().piece_length as u32;
        let piece0 = Piece {
            index: 0,
            begin: 0,
            length: piece_length,
        };
        let piece1 = Piece {
            index: 1,
            begin: 500,
            length: piece_length,
        };
        let piece2 = Piece {
            index: 3,
            begin: 500,
            length: 2 * (piece_length / 4) + 1,
        };
        let files = vec![
            piece_length,
            piece_length,
            piece_length,
            piece_length / 4,
            0,
            1,
            piece_length / 4,
            500,
        ];
        torrent.files = files
            .iter()
            .map(|l| BTFile {
                length: *l as i64,
                md5sum: None,
                path: PathBuf::new(),
            })
            .collect();

        assert_eq!(torrent.map_files(&piece0), vec![fpiece!(0, piece_length)]);
        assert_eq!(
            torrent.map_files(&piece1),
            vec![fpiece!(500, piece_length - 500), fpiece!(0, 500)]
        );
        assert_eq!(
            torrent.map_files(&piece2),
            vec![
                fpiece!(500, (piece_length / 4) - 500),
                fpiece!(0, 0),
                fpiece!(0, 1),
                fpiece!(0, piece_length / 4),
                fpiece!(0, 500),
            ]
        );
    }

    #[test]
    fn test_create_files() {
        let download_dir = format!("{}/{}", ::DL_DIR, "test_create_files");
        remove_dir_all(PathBuf::from(download_dir.clone()));
        let mut fnames = vec!["test/torrent.torrent", ::TEST_FILE];
        let test_files = fnames
            .iter_mut()
            .map(|x| Torrent::new(x, &download_dir, false).unwrap());
        for mut torrent in test_files {
            torrent.create_files().unwrap();
            test_read_write_torrent(&mut torrent);
        }
    }

    // This needs to be run after create_files
    fn test_read_write_torrent(torrent: &mut Torrent) {
        let bytes = gen_random_bytes(40);
        let p1 = PieceData {
            piece: piece!(0, 20),
            data: bytes[0..20].to_vec(),
        };
        let p2 = PieceData {
            piece: piece!(20, 20),
            data: bytes[20..40].to_vec(),
        };

        torrent.write_block_raw(&p2);
        torrent.write_block_raw(&p1);
        let p1_2: PieceData = torrent.read_block_raw(&p1.piece).unwrap();
        let p2_2: PieceData = torrent.read_block_raw(&p2.piece).unwrap();

        assert_eq!(p1, p1_2);
        assert_eq!(p2, p2_2);
    }
}
