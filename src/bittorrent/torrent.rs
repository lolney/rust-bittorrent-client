use bittorrent::{ParseError, Piece, PieceData, metainfo::BTFile, metainfo::MetaInfo};
use std::collections::HashMap;
use bit_vec::BitVec;
use std::io::Error as IOError;
use std::fs::{create_dir_all, remove_dir_all, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use rand::random;
use std::collections::hash_map::Entry;
use std::cmp;
use std::path::PathBuf;

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
    map: HashMap<u32, Vec<DLMarker>>, // Piece indices -> indices indicated downloaded parts
    files: Vec<BTFile>,
}

struct FilePiece {
    path: PathBuf,
    begin: i64, // following convention of i64 for files
    length: i64,
}

#[derive(PartialEq, Debug, Clone)]
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
    pub fn new(infofile: String, download_dir: String) -> Result<Torrent, ParseError> {
        match MetaInfo::read(infofile) {
            Ok(metainfo) => {
                let files = metainfo.info().file_info.as_BTFiles(download_dir);
                Ok(Torrent {
                    metainfo: metainfo,
                    bitfield: BitVec::new(),
                    map: HashMap::new(),
                    files: files,
                })
            }
            Err(e) => Err(ParseError::from_parse_string(
                String::from("Error adding torrent"),
                e,
            )),
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

    pub fn write_block(&mut self, piece: &PieceData) -> Result<(), IOError> {
        for file in self.map_files(&piece.piece).iter() {
            let mut options = OpenOptions::new();
            options.write(true);

            let mut fp = match options.open(file.path.clone()) {
                Ok(f) => f,
                Err(_) => File::create(file.path.clone())?, // LOG: had to create file
            };
            fp.seek(SeekFrom::Start(file.begin as u64))?;
            fp.write_all(piece.data.as_slice());
        }

        self.insert_piece(&piece.piece);

        Ok(())
    }

    pub fn read_block(&mut self, piece: &Piece) -> Result<PieceData, IOError> {
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

    pub fn info_hash(&self) -> [u8; 20] {
        return self.metainfo.info_hash();
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
        let max_length: u32 = self.metainfo.info().piece_length as u32;

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
        let piece_length = self.metainfo.info().piece_length;

        let mut iter = self.files.iter();
        let mut total: i64 = 0;
        let index = piece.file_index(piece_length);
        let mut file: &BTFile = iter.next().unwrap();

        // Find the first file
        while (total + (*file).length) < index {
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
            file = iter.next().unwrap();

            vec.push(FilePiece {
                path: file.path.clone(),
                begin: index_in_file,
                length: load,
            })
        }

        return vec;
    }
}

fn gen_random_bytes(n: usize) -> Vec<u8> {
    (0..n).map(|_| random::<u8>()).collect()
}

macro_rules! piece {
    ($n : expr) => {
        Piece {
            index : 0,
            begin : 0,
            length : $n,
        }
    };
}

fn test_vec(torrent: &Torrent, vec2: &Vec<DLMarker>) {
    let vec = torrent.map.get(&0).unwrap();
    assert_eq!(vec2, vec);
}

#[test]
fn test_insert_piece() {
    use self::DLMarker::Begin as B;
    use self::DLMarker::End as E;

    let p1 = Piece::new(0, 1, 2); // 0##0..
    let p2 = Piece::new(0, 0, 2); // #000..
    let p3 = Piece::new(0, 3, 4); // 000####
    let p4 = Piece::new(0, 0, 7); // #######
    let p5 = Piece::new(0, 0, 8); // ########
    let p6 = Piece::new(0, 7, 2); // 000000###
    let p7 = Piece::new(0, 10, 1); // 0000000000#
    let p8 = Piece::new(0, 0, 12); // ############

    let mut torrent = Torrent::new("test/bible.torrent".to_string(), String::new()).unwrap();

    torrent.insert_piece(&p1);
    torrent.insert_piece(&p7);

    test_vec(&torrent, &vec![B(1), E(3), B(10), E(11)]);

    // In shaded boundary conditions
    assert!(!torrent.in_shaded(&0, &0, true));
    assert!(!torrent.in_shaded(&0, &1, true));
    assert!(torrent.in_shaded(&0, &2, true));
    assert!(torrent.in_shaded(&0, &3, true));

    assert!(torrent.in_shaded(&0, &1, false));
    assert!(torrent.in_shaded(&0, &2, false));
    assert!(!torrent.in_shaded(&0, &3, false));

    torrent.insert_piece(&p2); // ###0
    assert!(torrent.in_shaded(&0, &0, false));
    test_vec(&torrent, &vec![B(0), E(3), B(10), E(11)]);

    torrent.insert_piece(&p3); // ######
    test_vec(&torrent, &vec![B(0), E(7), B(10), E(11)]);

    torrent.insert_piece(&p4);
    test_vec(&torrent, &vec![B(0), E(7), B(10), E(11)]);

    torrent.insert_piece(&p5);
    test_vec(&torrent, &vec![B(0), E(8), B(10), E(11)]);

    torrent.insert_piece(&p6);
    test_vec(&torrent, &vec![B(0), E(9), B(10), E(11)]);

    torrent.insert_piece(&p8);
    test_vec(&torrent, &vec![B(0), E(12)]);
}

#[test]
fn test_map_files() {}

#[test]
fn test_create_files() {
    remove_dir_all(PathBuf::from("test/test"));
    let download_dir = String::from("test/test");
    let fnames = vec!["test/torrent.torrent", "test/bible.torrent"];
    let test_files = fnames
        .iter()
        .map(|x| Torrent::new(x.to_string(), download_dir.clone()).unwrap());
    for torrent in test_files {
        torrent.create_files().unwrap();
    }
}

/*
#[test] 
fn test_single_file() {
    let bytes = gen_random_bytes(40);
    let p1 = PieceData{
        piece : piece!(20),
        data : bytes[0..20].to_vec()
    };
    let p2 = PieceData {
        piece : piece!(20),
        data : bytes[20..40].to_vec()
    };

    let mut torrent = Torrent::new("bible.torrent".to_string()).unwrap();
    torrent.write_block(&p2);
    torrent.write_block(&p1);
    let p1_2 : PieceData = torrent.read_block(&p1.piece).unwrap();
    let p2_2 : PieceData = torrent.read_block(&p2.piece).unwrap();

    assert_eq!(p1, p1_2);
    assert_eq!(p2, p2_2);
}*/
