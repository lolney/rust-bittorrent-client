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

pub trait TorrentState {
    fn infos<'a>(&'a mut self) -> &'a HashMap<Hash, Info>;

    fn torrent(&mut self, info_hash: String) -> Result<&Info, ClientError> {
        self.update_state()?;
        self.infos()
            .get(&Hash::from_str(info_hash)?)
            .ok_or_else(|| ClientError::NotFound)
    }

    fn torrents(&mut self) -> Result<Vec<Info>, ClientError> {
        self.update_state()?;
        Ok(self.infos().values().map(|info| info.clone()).collect())
    }

    fn update_state(&mut self) -> Result<(), ClientError>;

    fn add(&mut self, metainfo_path: String, download_path: String) -> Result<(), ClientError>;

    fn remove(&self, info_hash: String) -> Result<(), ClientError>;

    fn pause(&self, info_hash: String) -> Result<(), ClientError>;

    fn resume(&self, info_hash: String) -> Result<(), ClientError>;
}

pub struct BackedTorrentState {
    manager: Manager,
    channel: BidirectionalChannel<ClientMsg, InfoMsg>,
    infos: HashMap<Hash, Info>,
}

pub struct StaticTorrentState {
    infos: HashMap<Hash, Info>,
}

impl TorrentState for StaticTorrentState {
    fn infos<'a>(&'a mut self) -> &'a HashMap<Hash, Info> {
        return &self.infos;
    }

    fn update_state(&mut self) -> Result<(), ClientError> {
        Ok(())
    }

    fn add(&mut self, metainfo_path: String, download_path: String) -> Result<(), ClientError> {
        Ok(())
    }

    fn remove(&self, info_hash: String) -> Result<(), ClientError> {
        Ok(())
    }

    fn pause(&self, info_hash: String) -> Result<(), ClientError> {
        Ok(())
    }

    fn resume(&self, info_hash: String) -> Result<(), ClientError> {
        Ok(())
    }
}

impl StaticTorrentState {
    pub fn new() -> StaticTorrentState {
        let example_torrent: Info = Info {
            info_hash: Hash::from_str("XYZ".to_string()).unwrap(),
            name: "Example".to_string(),
            status: Status::Running,
            progress: 0.1f32,
            up: 100,
            down: 50,
            npeers: 5,
        };
        let mut obj = StaticTorrentState {
            infos: HashMap::new(),
        };
        obj.infos.insert(example_torrent.info_hash, example_torrent);
        return obj;
    }
}

impl TorrentState for BackedTorrentState {
    fn infos<'a>(&'a mut self) -> &'a HashMap<Hash, Info> {
        return &self.infos;
    }

    /// Called before any GET request.
    /// Sets state to the latest InfoMsg broadcast
    fn update_state(&mut self) -> Result<(), ClientError> {
        let mut infos_batch: Option<Vec<Info>> = Option::None;
        let mut single_updates = Vec::new();

        for msg in self.channel.try_iter() {
            println!("Msg: {:?}", msg);
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

    fn add(&mut self, metainfo_path: String, download_path: String) -> Result<(), ClientError> {
        self.manager.add_torrent(&metainfo_path, &download_path)?;
        Ok(())
    }

    fn remove(&self, info_hash: String) -> Result<(), ClientError> {
        self.channel
            .send(ClientMsg::Remove(Hash::from_str(info_hash)?))?;
        Ok(())
    }

    fn pause(&self, info_hash: String) -> Result<(), ClientError> {
        self.channel
            .send(ClientMsg::Pause(Hash::from_str(info_hash)?))?;
        Ok(())
    }

    fn resume(&self, info_hash: String) -> Result<(), ClientError> {
        self.channel
            .send(ClientMsg::Resume(Hash::from_str(info_hash)?))?;
        Ok(())
    }
}

impl BackedTorrentState {
    pub fn new() -> BackedTorrentState {
        let mut manager = Manager::new().port(0);
        let chan = manager.handle();
        BackedTorrentState {
            manager: manager,
            channel: chan,
            infos: HashMap::new(),
        }
    }

    /// Used to mock updates in tests
    fn mock() -> (BackedTorrentState, BidirectionalChannel<InfoMsg, ClientMsg>) {
        let (private, public) = BidirectionalChannel::create();
        let obj = BackedTorrentState {
            manager: Manager::new(),
            channel: private,
            infos: HashMap::new(),
        };
        return (obj, public);
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

    fn unwrap_and_sort(state: &mut TorrentState) -> Vec<Info> {
        let mut vec = state.torrents().unwrap();
        vec.sort();
        vec
    }

    fn compare_all(sent: &Vec<Info>, original: &Vec<Info>) {
        for (a, b) in sent.iter().zip(original.iter()) {
            assert_eq!(a, b);
        }
    }

    #[test]
    fn test_initial_message() {
        let (mut state, chan) = BackedTorrentState::mock();
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
        let (mut state, chan) = BackedTorrentState::mock();
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
