#![feature(plugin, custom_derive)]
#![plugin(rocket_codegen)]

use bittorrent::bittorrent::manager::{Info, Status};
use bittorrent::bittorrent::Hash;

use rocket::http::Method;
use rocket::response::content::Json as JsonValue;
use rocket::State;
use rocket_contrib::Json;
use rocket_cors::{AllowedHeaders, AllowedOrigins};
use torrent_state::{StaticTorrentState, TorrentState};

use std::sync::{Arc, Mutex};

extern crate bittorrent;
extern crate rocket;
#[macro_use]
extern crate rocket_contrib;
extern crate rocket_cors;
extern crate serde;

mod responses;
mod torrent_state;

use responses::Result as CustomResult;
use torrent_state::ClientError;
/*
Rocket does not like using this alias
type WrappedState<'a> = State<'a, Arc<Mutex<TorrentState + Send>>>;
*/
type WrappedState<'a> = State<'a, Arc<Mutex<StaticTorrentState>>>;
type EmptyResult = Result<(), ClientError>;

macro_rules! get_mutex {
    ($mutex:ident) => {
        ($mutex.lock().expect("State mutex poisoned"))
    };
}

#[get("/torrents")]
fn torrents(mutex: WrappedState) -> CustomResult<Vec<Info>> {
    let mut state = get_mutex!(mutex);
    CustomResult::from(state.torrents().map(|data| Json(data)))
}

#[get("/torrents/<info_hash>")]
fn torrent(info_hash: String, mutex: WrappedState) -> CustomResult<Info> {
    let mut state = get_mutex!(mutex);
    CustomResult::from(state.torrent(info_hash).map(|data| Json(data.clone())))
}

#[post("/torrents/<info_hash>/pause")]
fn pause(info_hash: String, mutex: WrappedState) -> CustomResult<()> {
    let mut state = get_mutex!(mutex);
    CustomResult::from(state.pause(info_hash))
}
/*
#[post("/torrents/<info_hash>/start")]
fn start(info_hash: String, mutex: WrappedState) -> CustomResult<()> {
    let mut state = get_mutex!(mutex);
    CustomResult::from(state.start(info_hash))
}*/

#[delete("/torrents/<info_hash>/remove")]
fn remove(info_hash: String, mutex: WrappedState) -> CustomResult<()> {
    let mut state = get_mutex!(mutex);
    CustomResult::from(state.remove(info_hash))
}

// TODO: error handling
#[put("/torrents?<params>")]
fn add(params: AddParams, mutex: WrappedState) -> CustomResult<()> {
    let mut state = get_mutex!(mutex);
    CustomResult::from(state.add(params.metainfo_path, params.download_path))
}

#[derive(FromForm)]
struct AddParams {
    metainfo_path: String,
    download_path: String,
}

fn main() {
    let (allowed_origins, failed_origins) =
        AllowedOrigins::some(&["null", "http://localhost:8080"]);
    let options = rocket_cors::Cors {
        allowed_origins: allowed_origins,
        allowed_methods: vec![Method::Get].into_iter().map(From::from).collect(),
        allowed_headers: AllowedHeaders::some(&["Authorization", "Accept"]),
        allow_credentials: true,
        ..Default::default()
    };

    let state = Arc::new(Mutex::new(StaticTorrentState::new()));

    rocket::ignite()
        .mount("/", routes![torrents, torrent])
        .manage(state)
        .attach(options)
        .launch();
}
