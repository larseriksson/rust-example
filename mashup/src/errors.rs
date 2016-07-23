extern crate hyper;
extern crate rustc_serialize;

use std;
use std::fmt::{Display, Formatter};
use errors::ResourceError::*;

#[derive(Debug)]
pub enum ResourceError {
	AlbumError{artist_id : String, album_id: String, album_title: Option<String>, cause : TypedIOError},
	ArtistError{artist_id : String, cause : TypedIOError}
}

#[derive(Debug)]
pub struct TypedIOError {
	pub resource : String,
	pub cause : hyper::Error
}

impl std::convert::From<ResourceError> for TypedIOError {
	fn from(err: ResourceError) -> Self {
		match err {
			ArtistError {cause, ..} => cause,
			AlbumError {cause, ..} => cause
		}
	}
}

impl Display for TypedIOError {
	fn fmt(&self, f: &mut Formatter) -> Result<(), std::fmt::Error> {
		write!(f, "Underlying IO Error when accessing {}: {}", self.resource, self.cause)
	}
}

impl Display for ResourceError {
	fn fmt(&self, f: &mut Formatter) -> Result<(),std::fmt::Error> {
		match *self {
			AlbumError {ref artist_id, ref album_id, ref cause, ..} => write!(f, "AlbumError for id={}-{}: {}", artist_id, album_id, cause),
			ArtistError {ref artist_id, ref cause} => write!(f, "ArtistError for id={}: {}", artist_id, cause)
		}
	}
}

impl std::error::Error for TypedIOError {
	fn description(&self) -> &str {
		"Underyling IO Error"
	}

	fn cause(&self) -> Option<&std::error::Error> {
		Some(&self.cause)
	}
}

impl std::error::Error for ResourceError {
	fn description(&self) -> &str {
		match *self {
			AlbumError {..} => "Error while parsing Album",
			ArtistError {..} => "Error while parsing Artist"
		}
	}

    fn cause(&self) -> Option<&std::error::Error> {
		match *self {
			AlbumError{ ref cause, ..} => Some(cause),
			ArtistError{ ref cause, ..} => Some(cause)
		}
	}
}
