use bittorrent::bittorrent::manager::{BidirectionalChannel, ClientMsg, InfoMsg};
use bittorrent::bittorrent::manager::{Info, Manager, Status};
use bittorrent::bittorrent::{Hash, ParseError};
use std::sync::mpsc::SendError;

use std::collections::HashMap;

#[derive(Debug)]
pub enum ClientError {
    NotFound,
    Disconnect,
    ParseError(ParseError),
    SendError(SendError<ClientMsg>),
}

impl From<ParseError> for ClientError {
    fn from(error: ParseError) -> Self {
        ClientError::ParseError(error)
    }
}

impl From<SendError<ClientMsg>> for ClientError {
    fn from(error: SendError<ClientMsg>) -> Self {
        ClientError::SendError(error)
    }
}

pub struct TorrentState {
    manager: Manager,
    channel: BidirectionalChannel<ClientMsg, InfoMsg>,
    infos: HashMap<Hash, Info>,
}

impl TorrentState {
    fn new() -> TorrentState {
        let mut manager = Manager::new();
        let chan = manager.handle();
        TorrentState {
            manager: manager,
            channel: chan,
            infos: HashMap::new(),
        }
    }

    /// Used to mock updates in tests
    fn mock() -> (TorrentState, BidirectionalChannel<InfoMsg, ClientMsg>) {
        let (private, public) = BidirectionalChannel::create();
        let obj = TorrentState {
            manager: Manager::new(),
            channel: private,
            infos: HashMap::new(),
        };
        return (obj, public);
    }

    pub fn torrent(&mut self, info_hash: String) -> Result<&Info, ClientError> {
        self.update_state()?;
        self.infos
            .get(&Hash::from(info_hash))
            .ok_or_else(|| ClientError::NotFound)
    }

    pub fn torrents(&mut self) -> Result<Vec<&Info>, ClientError> {
        self.update_state()?;
        Ok(self.infos.values().collect())
    }

    /// Called before any GET request.
    /// Sets state to the latest InfoMsg broadcast
    pub fn update_state(&mut self) -> Result<(), ClientError> {
        let mut infos_batch: Option<Vec<Info>> = Option::None;
        let mut single_updates = Vec::new();

        for msg in self.channel.try_iter() {
            match msg {
                InfoMsg::All(vec) => {
                    single_updates.clear();
                    infos_batch = Some(vec);
                }
                InfoMsg::One(info) => {
                    single_updates.push(info);
                }
                InfoMsg::Disconnect => return Err(ClientError::Disconnect),
            }
        }

        if infos_batch.is_some() {
            self.infos.clear();
            for info in infos_batch.unwrap() {
                self.infos.insert(info.info_hash, info);
            }
        }

        for info in single_updates {
            self.infos.insert(info.info_hash, info);
        }

        Ok(())
    }

    pub fn add(&mut self, metainfo_path: &str, download_path: &str) -> Result<(), ClientError> {
        self.manager.add_torrent(metainfo_path, download_path)?;
        Ok(())
    }

    pub fn remove(self, info_hash: Hash) -> Result<(), ClientError> {
        self.channel.send(ClientMsg::Remove(info_hash))?;
        Ok(())
    }

    pub fn pause(self, info_hash: Hash) -> Result<(), ClientError> {
        self.channel.send(ClientMsg::Pause(info_hash))?;
        Ok(())
    }

    pub fn resume(self, info_hash: Hash) -> Result<(), ClientError> {
        self.channel.send(ClientMsg::Resume(info_hash))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {

    use torrent_state::*;

    use bittorrent::bittorrent::manager::{BidirectionalChannel, ClientMsg, InfoMsg};
    use bittorrent::bittorrent::manager::{Info, Manager, Status};

    fn gen_info() -> Info {
        Info {
            info_hash: Hash::random(),
            name: "Example".to_string(),
            status: Status::Running,
            progress: 0.1f32,
            up: 100,
            down: 50,
            npeers: 5,
        }
    }

    fn gen10() -> Vec<Info> {
        let mut vec: Vec<Info> = (0..10).map(|_| gen_info()).collect();
        vec.sort();
        vec
    }

    fn unwrap_and_sort(state: &mut TorrentState) -> Vec<&Info> {
        let mut vec = state.torrents().unwrap();
        vec.sort();
        vec
    }

    fn compare_all(sent: &Vec<&Info>, original: &Vec<Info>) {
        for (a, b) in sent.iter().zip(original.iter()) {
            assert_eq!(*a, b);
        }
    }

    #[test]
    fn test_initial_message() {
        let (mut state, chan) = TorrentState::mock();
        // Initial message
        let mut torrents: Vec<Info> = gen10();
        chan.send(InfoMsg::All(torrents.clone()));
        compare_all(&unwrap_and_sort(&mut state), &torrents);

        // One is deleted
        torrents.remove(0);
        chan.send(InfoMsg::All(torrents.clone()));
        let sent_torrents = state.torrents().unwrap();
        assert_eq!(sent_torrents.len(), torrents.len());
    }

    #[test]
    fn test_update_state() {
        let (mut state, chan) = TorrentState::mock();
        let batch1 = gen10();
        let batch2 = gen10();
        let mut batch3 = gen10();

        let single1 = gen_info();
        let single2 = gen_info();
        // Send a number of updates; make only last one is reflected
        // batch, batch
        chan.send(InfoMsg::All(batch1.clone()));
        chan.send(InfoMsg::All(batch2.clone()));
        compare_all(&unwrap_and_sort(&mut state), &batch2);
        // single, batch
        chan.send(InfoMsg::One(single1.clone()));
        chan.send(InfoMsg::All(batch2.clone()));
        compare_all(&unwrap_and_sort(&mut state), &batch2);
        // batch, single, single
        chan.send(InfoMsg::All(batch3.clone()));
        chan.send(InfoMsg::One(single1.clone()));
        chan.send(InfoMsg::One(single2.clone()));
        batch3.push(single1);
        batch3.push(single2);
        batch3.sort();
        compare_all(&unwrap_and_sort(&mut state), &batch3);
    }

}
