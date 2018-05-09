#![feature(plugin)]
#![plugin(rocket_codegen)]

use bittorrent::bittorrent::manager::{Info, Status};
use bittorrent::bittorrent::Hash;
use rocket::http::Method;
use rocket_contrib::json::Value;
use rocket_contrib::Json;
use rocket_cors::{AllowedHeaders, AllowedOrigins};

extern crate bittorrent;
extern crate rocket;
#[macro_use]
extern crate rocket_contrib;
extern crate rocket_cors;

mod torrent_state;

fn example_torrent() -> Info {
    Info {
        info_hash: Hash::from("XYZ".to_string()),
        name: "Example".to_string(),
        status: Status::Running,
        progress: 0.1f32,
        up: 100,
        down: 50,
        npeers: 5,
    }
}

#[get("/torrents")]
fn torrents() -> Json<Value> {
    Json(json!({ "torrents": [example_torrent()] }))
}

#[get("/torrents/<info_hash>")]
fn torrent(info_hash: String) -> Json<Info> {
    Json(example_torrent())
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

    rocket::ignite()
        .mount("/", routes![torrents, torrent])
        .attach(options)
        .launch();
}
