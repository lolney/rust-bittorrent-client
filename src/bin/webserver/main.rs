#![feature(plugin, custom_derive)]
#![feature(proc_macro_hygiene, decl_macro)]

use rocket::http::Method;
use rocket::{Rocket, State};
use rocket_cors::{AllowedHeaders, AllowedOrigins};
use torrent_state::{BackedTorrentState, TorrentState};

use std::sync::{Arc, Mutex};

extern crate bittorrent;
#[macro_use]
extern crate rocket;
#[macro_use]
extern crate rocket_contrib;
extern crate rocket_codegen;
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
fn torrents(mutex: WrappedState) -> CustomResult {
    let mut state = get_mutex!(mutex);
    CustomResult::from(state.torrents().map(|data| json!(data)))
}

#[get("/torrents/<info_hash>")]
fn torrent(info_hash: String, mutex: WrappedState) -> CustomResult {
    let mut state = get_mutex!(mutex);
    CustomResult::from(state.torrent(info_hash).map(|data| json!(data.clone())))
}

#[post("/torrents/<info_hash>/pause")]
fn pause(info_hash: String, mutex: WrappedState) -> CustomResult {
    let mut state = get_mutex!(mutex);
    CustomResult::from(state.pause(info_hash))
}

#[post("/torrents/<info_hash>/resume")]
fn resume(info_hash: String, mutex: WrappedState) -> CustomResult {
    let mut state = get_mutex!(mutex);
    CustomResult::from(state.resume(info_hash))
}

#[delete("/torrents/<info_hash>")]
fn remove(info_hash: String, mutex: WrappedState) -> CustomResult {
    let mut state = get_mutex!(mutex);
    CustomResult::from(state.remove(info_hash))
}

#[put("/torrents?<metainfo_path>&<download_path>")]
fn add(metainfo_path: String, download_path: String, mutex: WrappedState) -> CustomResult {
    let mut state = get_mutex!(mutex);
    CustomResult::from(state.add(metainfo_path, download_path))
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
    use bittorrent::{DL_DIR, TEST_FILE};
    use rocket::http::Status;
    use rocket::local::{Client, LocalResponse as Response};

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
            format!(
                "/torrents?download_path={}&metainfo_path={}",
                DL_DIR.replace("/", "%2F"),
                TEST_FILE.replace("/", "%2F")
            ),
            put,
            |response: &mut Response| {
                assert_eq!(response.status(), Status::InternalServerError);
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
