use bit_vec::BitVec;
use bittorrent::{manager::Status, metainfo::MetaInfo, torrent::Torrent, Hash, ParseError, Piece,
                 PieceData};
use priority_queue::PriorityQueue;
use std::collections::HashSet;
use std::collections::VecDeque;
use std::io::Error as IOError;
use std::iter::FromIterator;
use std::time::{Duration, Instant};

#[derive(PartialEq, Debug, Clone)]
/// Represents torrent state that is not persisted
/// Exposes API for reading/writing blocks
pub struct TorrentRuntime {
    torrent: Torrent,
    piece_queue: PieceQueue,                 // index -> rarity
    outstanding_requests: VecDeque<Request>, // Requests that have been made, but not fullfilled
    dl_rate: Rate,
    ul_rate: Rate,
    status: Status,
}

#[derive(PartialEq, Debug, Clone)]
struct Request {
    piece: Piece,
    time: Instant,
}

/// Two-tiered priority queue
/// 0-priority pieces are kept in `zeros`; they have effectively -inf priority,
/// since there are no peers that have them
/// Pieces move between zeros and queue as necessary
#[derive(PartialEq, Debug, Clone)]
struct PieceQueue {
    zeros: HashSet<usize>,
    queue: PriorityQueue<usize, isize>,
}

const MIN_PRIORITY: isize = -(::MAX_PEERS as isize) - 1; // never a valid value
impl PieceQueue {
    pub fn new(npieces: usize) -> PieceQueue {
        PieceQueue {
            zeros: HashSet::from_iter(0..npieces),
            queue: PriorityQueue::from_iter((0..npieces).map(|i| (i, MIN_PRIORITY))),
        }
    }

    pub fn peek(&self) -> Option<(usize, isize)> {
        if let Some((i, v)) = self.queue.peek() {
            Some((*i, *v))
        } else {
            None
        }
    }

    pub fn pop(&mut self) -> Option<(usize, isize)> {
        if let Some((i, priority)) = self.peek() {
            if priority != MIN_PRIORITY {
                self.queue.pop()
            } else {
                info!("Element in queue, but min priority");
                None
            }
        } else {
            info!("No elements in queue");
            None
        }
    }

    fn get_priority(&self, index: &usize) -> Option<isize> {
        if let Some(v) = self.queue.get_priority(index) {
            Some(*v)
        } else {
            None
        }
    }

    pub fn change_priority_by<F>(&mut self, index: &usize, f: F) -> Option<isize>
    where
        F: Fn(isize) -> isize,
    {
        if let Some(_) = self.zeros.get(index) {
            let new_priority = f(0);
            if new_priority > 0 {
                panic!("Priority can't be above 0");
            } else if new_priority < 0 {
                info!("Setting priority from zero: {}", new_priority);
                self.zeros.remove(index);
                self.queue.change_priority(index, new_priority);
            }
            Some(new_priority)
        } else if let Some(priority) = self.get_priority(index) {
            let new_priority = f(priority);
            if new_priority == 0 {
                info!("Setting priority to zero");
                self.zeros.insert(*index);
                self.queue.change_priority(index, MIN_PRIORITY);
            } else {
                info!("Setting priority to {}", new_priority);
                self.queue.change_priority(index, new_priority);
            }
            Some(new_priority)
        } else {
            info!("Not in either queue or zeros: {}", index);
            None
        }
    }
}

impl Default for PieceQueue {
    fn default() -> Self {
        PieceQueue {
            zeros: Default::default(),
            queue: Default::default(),
        }
    }
}

#[derive(PartialEq, Debug, Clone)]
struct Rate {
    total: usize,
    buffer: VecDeque<(Instant, usize)>,
}

impl Rate {
    pub fn new() -> Rate {
        Rate::default()
    }

    fn prune(&mut self) {
        while !self.buffer.is_empty() && self.buffer[0].0.elapsed() > Duration::from_secs(5) {
            let (_, old) = self.buffer.pop_front().unwrap();
            self.total -= old;
        }
    }

    pub fn rate(&mut self) -> usize {
        self.prune();
        self.total
    }

    pub fn update(&mut self, n: usize) {
        self.prune();
        self.buffer.push_back((Instant::now(), n));
        self.total += n;
    }
}

impl Default for Rate {
    fn default() -> Self {
        Rate {
            total: 0,
            buffer: VecDeque::new(),
        }
    }
}

impl TorrentRuntime {
    pub fn new(
        infofile: &str,
        download_dir: &str,
        downloaded: bool,
    ) -> Result<TorrentRuntime, ParseError> {
        let torrent = Torrent::new(infofile, download_dir, downloaded)?;
        Ok(TorrentRuntime {
            piece_queue: PieceQueue::new(torrent.npieces()),
            torrent: torrent,
            outstanding_requests: Default::default(),
            dl_rate: Rate::new(),
            ul_rate: Rate::new(),
            status: if downloaded {
                Status::Complete
            } else {
                Status::Running
            },
        })
    }

    #[inline]
    pub fn metainfo(&self) -> MetaInfo {
        self.torrent.metainfo()
    }

    #[inline]
    pub fn download_rate(&mut self) -> usize {
        self.dl_rate.rate()
    }

    #[inline]
    pub fn upload_rate(&mut self) -> usize {
        self.ul_rate.rate()
    }

    #[inline]
    pub fn nrequests(&self) -> usize {
        self.outstanding_requests.len()
    }

    #[inline]
    pub fn status(&self) -> Status {
        self.status
    }

    #[inline]
    pub fn downloaded(&self) -> usize {
        self.torrent.downloaded()
    }

    #[inline]
    pub fn remaining(&self) -> usize {
        self.torrent.remaining()
    }

    #[inline]
    pub fn piece_length(&self) -> i64 {
        self.torrent.piece_length()
    }

    #[inline]
    pub fn bitfield(&self) -> &BitVec {
        self.torrent.bitfield()
    }

    #[inline]
    pub fn size(&self) -> usize {
        self.torrent.size()
    }

    #[inline]
    pub fn trackers(&self) -> Vec<String> {
        self.torrent.trackers()
    }

    #[inline]
    pub fn name(&self) -> &String {
        self.torrent.name()
    }

    #[inline]
    pub fn npieces(&self) -> usize {
        self.torrent.npieces()
    }

    #[inline]
    pub fn info_hash(&self) -> Hash {
        self.torrent.info_hash()
    }

    #[inline]
    pub fn create_files(&self) -> Result<(), IOError> {
        self.torrent.create_files()
    }

    fn remove_request(&mut self, piece_data: &PieceData) {
        self.outstanding_requests
            .retain(|req| req.piece != piece_data.piece);
    }

    pub fn write_block(&mut self, piece: &PieceData) -> Result<(), IOError> {
        self.remove_request(piece);
        self.torrent.write_block(piece)?;
        self.dl_rate.update(piece.piece.length as usize);
        Ok(())
    }

    pub fn read_block(&mut self, piece: &Piece) -> Result<PieceData, IOError> {
        let result = self.torrent.read_block(piece)?;
        self.ul_rate.update(result.piece.length as usize);
        Ok(result)
    }

    /// select the next piece to be requested
    pub fn select_piece(&mut self) -> Option<Piece> {
        let timeout = Duration::from_secs(::READ_TIMEOUT);
        let front = self.outstanding_requests.pop_front();
        // Try to take an expired outstanding request first
        if front.is_some() {
            let front = front.unwrap();
            if front.time.elapsed() >= timeout {
                info!("taking an expired request");
                return Some(front.piece);
            } else {
                info!("request hasn't expired yet");
                self.outstanding_requests.push_front(front)
            }
        }
        // Take from the piece queue otherwise
        match self.piece_queue.pop() {
            Some((index, _)) => {
                let piece = self.torrent.default_piece(index);
                self.outstanding_requests.push_back(Request {
                    piece: piece.clone(),
                    time: Instant::now(),
                });
                Some(piece)
            }
            None => {
                info!("no requests in queue");
                None
            }
        }
    }

    /// `d_npeers`: change in number of peers holding that piece
    /// An increase in peers decreases the priority
    pub fn update_priority(&mut self, piece_index: usize, d_npeers: isize) {
        self.piece_queue
            .change_priority_by(&piece_index, |n| n - d_npeers);
    }

    /// `pause`: true to pause, or false to unpause
    /// Returns true if status is changed; else false
    pub fn pause(&mut self, pause: bool) -> bool {
        if pause {
            if self.status != Status::Paused {
                self.status = Status::Paused;
                true
            } else {
                false
            }
        } else {
            if self.status == Status::Paused {
                if self.torrent.remaining() == 0 {
                    self.status = Status::Complete;
                    true
                } else {
                    self.status = Status::Running;
                    true
                }
            } else {
                false
            }
        }
    }

    pub fn set_complete(&mut self) -> Result<(), String> {
        if self.torrent.remaining() == 0 {
            if self.status == Status::Running {
                self.status = Status::Complete;
                Ok(())
            } else {
                Err(format!(
                    "Tried to set torrent as complete, but its status is {:?}",
                    self.status
                ))
            }
        } else {
            Err("Tried to set torrent as complete, but it's not complete".to_string())
        }
    }
}

#[cfg(test)]
mod tests {

    use bittorrent::torrent_runtime::*;
    use bittorrent::utils::gen_random_bytes;
    use bittorrent::{metainfo::BTFile, Piece};
    use std::path::PathBuf;

    fn _test_pause(torrent: &mut TorrentRuntime, begin: Status) {
        assert_eq!(torrent.status, begin);
        assert_eq!(torrent.pause(true), true);
        assert_eq!(torrent.status, Status::Paused);
        assert_eq!(torrent.pause(true), false);
        assert_eq!(torrent.status, Status::Paused);
        assert_eq!(torrent.pause(false), true);
        assert_eq!(torrent.status, begin);
        assert_eq!(torrent.pause(false), false);
        assert_eq!(torrent.status, begin);
    }

    #[test]
    fn test_pause() {
        // Initialize, pause, then unpause
        _test_pause(
            &mut TorrentRuntime::new(::TEST_FILE, "", false).unwrap(),
            Status::Running,
        );
        _test_pause(
            &mut TorrentRuntime::new(
                ::TEST_FILE,
                &format!("{}/{}", ::READ_DIR, "valid_torrent"),
                true,
            ).unwrap(),
            Status::Complete,
        );
    }

    #[test]
    fn test_piece_queue() {
        let mut piece_queue = PieceQueue::new(3);
        // Initially nothing in queue
        assert_eq!(piece_queue.pop(), None);
        // Change priority
        piece_queue.change_priority_by(&0, |p| p - 1).unwrap();
        assert_eq!(piece_queue.queue.len(), 3);
        assert_eq!(piece_queue.zeros.len(), 2);
        assert_eq!(piece_queue.peek().unwrap(), (0, -1));
        // Change it back
        piece_queue.change_priority_by(&0, |p| p + 1).unwrap();
        assert_eq!(piece_queue.pop(), None);
        // Change again
        piece_queue.change_priority_by(&0, |p| p - ::MAX_PEERS as isize);
        assert_eq!(piece_queue.pop().unwrap(), (0, -(::MAX_PEERS as isize)));
        assert_eq!(piece_queue.change_priority_by(&0, |p| p + 1), None);
        // Change the other two
        piece_queue.change_priority_by(&1, |p| p - 1).unwrap();
        piece_queue.change_priority_by(&2, |p| p - 2).unwrap();
        assert_eq!(piece_queue.peek().unwrap(), (1, -1));
    }

    #[test]
    fn test_priority() {
        let mut torrent = TorrentRuntime::new(::TEST_FILE, "", false).unwrap();

        // update priority for piece
        for i in 0..torrent.npieces() {
            torrent.update_priority(i, 1);
            assert_eq!(torrent.select_piece().unwrap().index as usize, i);
        }
        assert_eq!(torrent.select_piece(), None);
        assert_eq!(torrent.outstanding_requests.len(), torrent.npieces());
    }

}
