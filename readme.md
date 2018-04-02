# Rust-Bittorrent-Client

An implementation of the Bittorrent spec in Rust, along with a terminal-based client (uses the ncurses toolkit). Note: pre-release, still undergoing rapid changes.

## Running the client

```
git clone https://github.com/lolney/rust-bittorrent-client.git
cd rust-bittorrent-client
cargo run
```

## Using the library

```
extern crate bittorrent;

use bittorrent::manager::Manager;

fn main() {
    let mut manager = Manager::new();
    let rx = manager.handle("3000");

    // The manager publishes updates on this channel
    thread::spawn(move || {
        match rx.recv() {
            Ok(v) => println!({":?"}, v);
            Err(e) => println!("Error receiving update: {:?}", e); 
        }
    })

    // Adding a torrent
    manager.add_torrent(
        "/path/to/torrent_file.torrent", 
        "/path/to/download/directory",
    );
}
```
Manager updates are of type `bittorrent::manager::InfoMsg`:
```
enum InfoMsg {
    All(Vec<Info>),
    One(Info),
}

struct Info {
    info_hash: hash,
    name: String,
    status: Status,
    progress: f32,
    up: usize,
    down: usize,
    npeers: usize,
}

enum Status {
    Paused,
    Running,
    Complete,
}
```

## Running the tests

`cargo test`
