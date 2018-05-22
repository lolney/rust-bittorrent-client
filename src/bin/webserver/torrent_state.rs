use bittorrent::bittorrent::manager::{BidirectionalChannel, ClientMsg, InfoMsg};
use bittorrent::bittorrent::manager::{Info, Manager, Status};
use bittorrent::bittorrent::Hash;

use std::collections::HashMap;

#[derive(PartialEq, Debug, Clone)]
pub enum ClientError {
    NotFound,
    Disconnect,
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
    /// TODO: skip to last update?
    pub fn update_state(&mut self) -> Result<(), ClientError> {
        for msg in self.channel.try_iter() {
            match msg {
                InfoMsg::All(vec) => {
                    self.infos.clear();
                    for info in vec {
                        self.infos.insert(info.info_hash, info);
                    }
                }
                InfoMsg::One(info) => {
                    self.infos.insert(info.info_hash, info);
                }
                InfoMsg::Disconnect => return Err(ClientError::Disconnect),
            }
        }
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

    #[test]
    fn test_torrent_state() {
        let (mut state, chan) = TorrentState::mock();
        // Initial message
        let torrents: Vec<Info> = (0..10).map(|_| gen_info()).collect();
        chan.send(InfoMsg::All(torrents.clone()));
        for i in (0..10) {
            assert_eq!(*state.torrents().unwrap()[i], torrents[i]);
        }
        // One is deleted
    }

}
