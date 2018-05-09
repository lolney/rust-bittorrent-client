use bittorrent::bittorrent::manager::{BidirectionalChannel, ClientMsg, InfoMsg};
use bittorrent::bittorrent::manager::{Info, Manager, Status};
use bittorrent::bittorrent::Hash;

use std::collections::HashMap;

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

    /*
    fn mock() -> TorrentState, BidirectionalChannel<InfoMsg, ClientMsg> {

    }*/

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

    use webserver::torrent_state::*;

    #[test]
    fn test_torrent_state() {
        // Initial message
        // One is deleted

    }

}
