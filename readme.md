# Rust-Bittorrent-Client

An implementation of the Bittorrent spec in Rust, along with a terminal-based client (uses the ncurses toolkit). Note: pre-release, still undergoing changes.

## Running the client

```
git clone https://github.com/lolney/rust-bittorrent-client.git
cd rust-bittorrent-client
cargo run
```

## Using the library

```
extern crate bittorrent;

use bittorrent::bittorrent::Manager;

let manager = Manager::new()
let recv = manager.handle();

// The manager publishes updates on this channel
thread::spawn(move || {
    match recv.recv() {
        Ok(v) => println!({":?"}, v);
        Err(e) => println!("Error receiving update: {:?}", e); 
    }
})

// Adding a torrent
manager.add_torrent(
    "/path/to/torrent_file.torrent", 
    "/path/to/download/directory",
);
```

## Running the tests

`cargo test`
