#![feature(plugin, custom_derive)]
#![plugin(rocket_codegen)]

use bittorrent::bittorrent::manager::{Info, Status};
use bittorrent::bittorrent::Hash;

use rocket::http::Method;
use rocket::response::content::Json as JsonValue;
use rocket::{Rocket, State};
use rocket_contrib::Json;
use rocket_cors::{AllowedHeaders, AllowedOrigins};
use torrent_state::{BackedTorrentState, TorrentState};

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
type WrappedState<'a> = State<'a, Arc<Mutex<BackedTorrentState>>>;
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

#[post("/torrents/<info_hash>/resume")]
fn resume(info_hash: String, mutex: WrappedState) -> CustomResult<()> {
    let mut state = get_mutex!(mutex);
    CustomResult::from(state.resume(info_hash))
}

#[delete("/torrents/<info_hash>")]
fn remove(info_hash: String, mutex: WrappedState) -> CustomResult<()> {
    let mut state = get_mutex!(mutex);
    CustomResult::from(state.remove(info_hash))
}

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

fn rocket() -> Rocket {
    let (allowed_origins, failed_origins) =
        AllowedOrigins::some(&["null", "http://localhost:8080"]);

    let options = rocket_cors::Cors {
        allowed_origins: allowed_origins,
        allowed_methods: vec![Method::Get, Method::Put, Method::Post, Method::Delete]
            .into_iter()
            .map(From::from)
            .collect(),
        allowed_headers: AllowedHeaders::some(&["Authorization", "Accept"]),
        allow_credentials: true,
        ..Default::default()
    };

    let state = Arc::new(Mutex::new(BackedTorrentState::new()));

    rocket::ignite()
        .mount("/", routes![torrents, torrent, pause, resume, remove, add])
        .manage(state)
        .attach(options)
}

fn main() {
    rocket().launch();
}

#[cfg(test)]
mod test {
    use super::rocket;
    use rocket::http::Status;
    use rocket::local::{Client, LocalResponse as Response};

    use rocket::http::ContentType;

    macro_rules! run_test {
        ($route:expr, $method:ident, $test_fn:expr) => {
            let client = Client::new(rocket()).expect("valid rocket instance");
            let mut response = client.$method($route).dispatch();
            $test_fn(&mut response);
        };
    }

    #[test]
    fn server_get_torrents() {
        run_test!("/torrents", get, |response: &mut Response| {
            assert_eq!(response.status(), Status::Ok);
            assert!(response.body_string().is_some());
            println!("Body string: {:?}", response.body_string());
        });
    }

    #[test]
    fn server_add_valid_path() {
        run_test!(
            "/torrents?download_path=test&metainfo_path=test",
            put,
            |response: &mut Response| {
                assert_eq!(response.status(), Status::Ok);
            }
        );
    }

    #[test]
    fn server_add_invalid_params() {
        let invalid_params = [
            "metainfo_path=test",
            "metainfo_path=test&download=test",
            "download_path=test",
        ];

        for params in invalid_params.iter() {
            run_test!(
                format!("/torrents/add?{}", params),
                put,
                |response: &mut Response| {
                    assert_eq!(response.status(), Status::NotFound);
                    assert!(response.body_string().is_some());
                }
            );
        }
    }
}
