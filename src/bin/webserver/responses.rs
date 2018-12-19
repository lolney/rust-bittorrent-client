use rocket::http::Status;
use rocket::request::Request;
use rocket::response::{Responder, Response, Result as RocketResult};
use rocket_contrib::json::JsonValue;
use std::convert::From;
use std::io::Cursor;
use std::result::Result as StdResult;

use torrent_state::ClientError;

pub enum Result {
    JSON(JsonValue),
    Empty,
    Err(ClientError),
}

impl From<StdResult<(), ClientError>> for Result {
    fn from(result: StdResult<(), ClientError>) -> Self {
        match result {
            Ok(()) => Result::Empty,
            Err(err) => Result::Err(err),
        }
    }
}

impl From<StdResult<JsonValue, ClientError>> for Result {
    fn from(result: StdResult<JsonValue, ClientError>) -> Self {
        match result {
            Ok(json) => Result::JSON(json),
            Err(err) => Result::Err(err),
        }
    }
}

impl<'r> Responder<'r> for ClientError {
    fn respond_to(self, _req: &Request) -> RocketResult<'r> {
        match self {
            ClientError::NotFound => Response::build()
                .status(Status::NotFound)
                .sized_body(Cursor::new("Torrent does not exist"))
                .ok(),
            ClientError::Disconnect => Response::build()
                .status(Status::InternalServerError)
                .sized_body(Cursor::new("Disconnected from manager"))
                .ok(),
            ClientError::ParseError(parse_error) => Response::build()
                .status(Status::InternalServerError)
                .sized_body(Cursor::new(format!("BitTorrent Error: {:?}", parse_error)))
                .ok(),
            ClientError::SendError(send_error) => Response::build()
                .status(Status::InternalServerError)
                .sized_body(Cursor::new(format!("Channel Error: {:?}", send_error)))
                .ok(),
        }
    }
}

impl<'r> Responder<'r> for Result {
    fn respond_to(self, req: &Request) -> RocketResult<'r> {
        match self {
            Result::Empty => ().respond_to(req),
            Result::JSON(json) => json.respond_to(req),
            Result::Err(err) => err.respond_to(req),
        }
    }
}
