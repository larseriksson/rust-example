extern crate hyper;
extern crate rustc_serialize;
extern crate url;

use std::fmt::{Display, Formatter};
use hyper::client::{Client};
use std::fs::File;
use hyper::header::*;
use std::io::prelude::*;
use std::path::Path;
use std::thread;
use std::sync::Arc;
use rustc_serialize::{json, Decodable, Decoder};
use hyper::mime::{Mime};
use std::sync::mpsc;
use ResourceError::*;
use hyper::status::StatusCode;
use url::Url;
use std::fs;

//const TEST_ID : &'static str = "5b11f4ce-a62d-471e-81fc-a69a8278c7da";

const USER_ARGENT: &'static str = "Mozilla/5.0 (Windows NT 6.1) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/41.0.2228.0 Safari/537.36";

macro_rules! println_stderr(
    ($($arg:tt)*) => { {
        let r = writeln!(&mut ::std::io::stderr(), $($arg)*);
        r.expect("failed printing to stderr");
    } }
);

macro_rules! musicbrainz_url {
	($id : expr) => ( format!("http://musicbrainz.org/ws/2/artist/{}?&fmt=json&inc=url-rels+release-groups", $id))
	//($id : expr) => ( format!("http://localhost:8000/ws/2/artist/{}?&fmt=json&inc=url-rels+release-groups", $id))
}

macro_rules! musicbrainz_file {
	($id : expr) => ( format!("tmp/mb_{}.json", $id))
}

macro_rules! cover_art_url {
	($id : expr) => ( format!("http://coverartarchive.org/release-group/{}", $id) )
}

macro_rules! cover_art_file {
	($id : expr) => ( format!("tmp/ca_{}.json", $id) )
}

fn read_from_file(url : &str) -> Result<String, TypedIOError> {
	let path = Path::new(url);
	let mut content = String::new();

	File::open(&path)
		.and_then(|mut file| file.read_to_string(&mut content))
		.map(|_| {
			//return the content rather than the size
			content
		})
		.map_err(|err| TypedIOError {
			resource : url.to_string(),
			cause : hyper::Error::from(err)
		})
}

#[allow(dead_code)]
fn filter_successful(resource: &str, mut resp : hyper::client::response::Response) -> Result<String, TypedIOError>
{
	match resp.status {
		StatusCode::Ok => {
			let mut s = String::new();
			resp.read_to_string(&mut s);

			Ok(s)
		},
		code  @ _ => Err( TypedIOError {
			resource : resource.to_string(),
			cause: hyper::Error::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, format!("Service responded with statuscode {}", code)))
		})
	}
}

#[allow(dead_code)]
fn save_response_to_file(url : &str, content : &str) {
	let provider = Provider::find_by_url(url).unwrap();
	let id = Provider::extract_id(url);

	fs::create_dir_all("tmp");

	let path = Path::new("tmp").join(provider.format_file_name(&id));

	if !path.exists() {
		let f = File::create(path)
			.and_then(|mut f| f.write_all(content.as_bytes()));

		println_stderr!("saving file {:?}", f);
	}
}


fn read_from_url(url : &str) -> Result<String, TypedIOError> {
	println_stderr!("invoking {}", url);

	let client = Client::new();
	let mime: Mime = "text/json".parse().unwrap();

	let mut response = client.get(url)
		.header(ContentType::json())
		.header(UserAgent(USER_ARGENT.to_owned()))
		.header(Connection::keep_alive())
		.header(Accept(vec![qitem(mime)]))
		.send()
		.map_err(|err| TypedIOError {
			resource : url.to_string(),
			cause : err
		})
		.and_then(|resp| filter_successful(url, resp))
		.map(|resp| {
			if cfg!(feature="meshup_mode_save_web") {
				save_response_to_file(url, &resp);
			}

			resp
		});

	response
}

//TODO: define trait
fn query_mb_from_file(id : &str) -> Result<ArtistReference, ResourceError> {
	let mb_file = musicbrainz_file!(id);
	let mb_response = try!(read_from_file(&mb_file).map_err(|err| {
		ArtistError {
			artist_id : id.to_string(),
			cause: err.into()
		}
	}));

	let artist_ref = process_mb_response(&mb_response);
	let albums = query_cover_art(id.to_string(), artist_ref.albums, |id| {
		let file_name = cover_art_file!(id);

		read_from_file(&file_name)
	});

	Ok(ArtistReference {
		name: artist_ref.name,
		albums: albums
	})
}

fn query_mb_from_web(id: &str) -> Result<ArtistReference, ResourceError> {
	let mb_query_url = musicbrainz_url!(id);
	let mb_response = try!(read_from_url(&mb_query_url).map_err(|err| ArtistError {
		artist_id: id.to_string(),
		cause: err
	}));

	let artist_id = id.to_string();
	let artist_ref = process_mb_response(&mb_response);
	let albums = query_cover_art(artist_ref.name.clone(), artist_ref.albums, |id| {
		let url = cover_art_url!(id);
		read_from_url(&url)
	});

	Ok(ArtistReference {
		name : artist_ref.name.clone(),
		albums : albums
	})
}


fn query_cover_art<F>(artist_id: String, list_of_references: Vec<AlbumReference>, cover_art: F) -> Vec<AlbumReference>
	where F: Send + 'static + Fn(&String)->Result<String, TypedIOError> + Sync {
	let album_references = Arc::new(list_of_references);
	let shareable_cover_art = Arc::new(cover_art);

	let threads : Vec<_> = album_references.clone().iter().map(|album_reference| {
		let mut album = album_reference.clone();
		let (tx, rx): (mpsc::Sender<Result<AlbumReference, ResourceError>>, mpsc::Receiver<Result<AlbumReference, ResourceError>>) = mpsc::channel();
		let child_cover_art = shareable_cover_art.clone();

		let artist_id = artist_id.to_string();
		let album_id = album.id.clone();
		let album_title = album.title.clone();

		thread::spawn(move || {
			let result = child_cover_art(&album_id)
				.map(|resp| {
					album.with_image(image_from_cover_art_response(&resp));

					album
				})
				.map_err(|err| ResourceError::AlbumError {
					artist_id : artist_id,
					album_id: album_id,
					album_title : Some(album_title),
					cause: TypedIOError::from(err)
				});

			tx.send(result)
		});

		rx
	}).collect();


	let updated_album_refs: Vec<AlbumReference> = threads.into_iter().map(|thread| {
		let item = thread.recv().unwrap();

		item.unwrap_or_else(|err| {
			println_stderr!("{}", err);
			AlbumReference::from(err)
		})
	}).collect();

	updated_album_refs
}


fn image_from_cover_art_response(payload : &str) -> String {
	let body : self::CoverArtResponse = json::decode(&payload).unwrap();

	body.images.into_iter().find(|item| item.front).unwrap().image
}

#[test]
fn test_image_from_cover_art_response() {
	let payload = "{\"images\":[{\"front\":true,\"image\":\"http://coverartarchive.org/release/a146429a-cedc-3ab0-9e41-1aaf5f6cdc2d/3012495605.jpg\"}]}";

	let response = image_from_cover_art_response(payload);

	assert_eq!("http://coverartarchive.org/release/a146429a-cedc-3ab0-9e41-1aaf5f6cdc2d/3012495605.jpg", response);
}


fn process_mb_response(payload: &str) -> ArtistReference {
	let a: ArtistReference = json::decode(payload).unwrap();

	a
}


#[allow(dead_code)]
#[derive(Debug)]
enum Provider {
	MUSICBRAINZ, COVER_ART
}

//it must be a better way to express it
impl std::cmp::PartialEq<Provider> for Provider {
	fn eq(&self, other: &Provider) -> bool {
		match *self {
			Provider::COVER_ART => match *other {
				Provider::COVER_ART => true,
				Provider::MUSICBRAINZ => false
			},
			Provider::MUSICBRAINZ => match *other {
				Provider::COVER_ART => false,
				Provider::MUSICBRAINZ => true
			}
		}
	}
}

#[allow(dead_code)]
impl Provider {
	fn extract_id(url: &str) -> String {
		let parsed : Url = Url::parse(url).unwrap();

		parsed.path_segments().unwrap().last().unwrap().to_string()
	}

	fn find_by_url(url: &str) -> Option<Provider> {
		let extract_id = Provider::extract_id(url);

		if Provider::MUSICBRAINZ.format_url(&extract_id) == url {
			return Some(Provider::MUSICBRAINZ)
		}

		if Provider::COVER_ART.format_url(&extract_id) == url {
			return Some(Provider::COVER_ART)
		}

		None
	}

	fn format_url(&self, id: &str) -> String {
		match *self {
			Provider::MUSICBRAINZ => musicbrainz_url!(id),
			Provider::COVER_ART => cover_art_url!(id)
		}
	}

	fn format_file_name(&self, id: &str) -> String {
		match *self {
			Provider::MUSICBRAINZ => format!("mb_{}.json", id),
			Provider::COVER_ART => format!("ca_{}.json", id)
		}
	}
}


#[test]
fn test_extract_id_from_url() {
	let mb : String = musicbrainz_url!("1289836171-250");

	assert_eq!("1289836171-250", Provider::extract_id(&mb));
}

#[test]
fn test_format_url() {
	assert_eq!("http://musicbrainz.org/ws/2/artist/123?&fmt=json&inc=url-rels+release-groups", Provider::MUSICBRAINZ.format_url("123"));
	assert_eq!("http://coverartarchive.org/release-group/123", Provider::COVER_ART.format_url("123"));

}

#[test]
fn test_format_file_name() {
	assert_eq!("ca_123", Provider::COVER_ART.format_file_name("123"));
	assert_eq!("mb_123", Provider::MUSICBRAINZ.format_file_name("123"));
}

#[test]
fn test_find_by_url() {
	let test_url = Provider::MUSICBRAINZ.format_url("123");
	assert_eq!(Some(Provider::MUSICBRAINZ), Provider::find_by_url(&test_url));
	assert_eq!(None, Provider::find_by_url(&"http://google.com".to_string()))
}

#[derive(Debug)]
enum ResourceError {
	AlbumError{artist_id : String, album_id: String, album_title: Option<String>, cause : TypedIOError},
	ArtistError{artist_id : String, cause : TypedIOError}
}

#[derive(Debug)]
struct TypedIOError {
	resource : String,
	cause : hyper::Error
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

#[derive(RustcDecodable, Debug)]
struct CoverArtResponse {
	images : Vec<self::Image>
}

#[derive(RustcDecodable, Debug)]
struct Image {
	front : bool,
	image : String
}

#[derive(Debug, RustcEncodable)]
struct ArtistReference {
	name: String,
	albums : Vec<AlbumReference>
}

#[derive(Debug, Clone, RustcEncodable)]
struct AlbumReference {
	id: String,
	title: String,
	image : Option<String>,
	error : bool
}

impl AlbumReference {
	fn with_image(&mut self, image : String) -> &AlbumReference {
		self.image = Some(image);

		self
	}
}

impl std::convert::From<ResourceError> for AlbumReference {
	fn from(r : ResourceError) -> Self {
		match r {
			AlbumError{album_id, album_title, ..} => AlbumReference {
				id : album_id,
				title: album_title.unwrap_or("".to_string()),
				image : None,
				error : true
			},
			ArtistError { .. } => unreachable!()
		}
	}
}

impl Decodable for ArtistReference {
	fn decode<D : Decoder>(d: &mut D) -> Result<ArtistReference, D::Error> {
		d.read_struct("ArtistReference", 2, |d| {
			let name = try!(d.read_struct_field("name", 0, |d| d.read_str()));
			let albums = try!(d.read_struct_field("release-groups", 0, |d| {
				let buffer = d.read_seq(|d, len| {
					let mut buffer = Vec::new();

					for idx in 0..(len-1) {
						let item: AlbumReference = try!(d.read_seq_elt(idx, Decodable::decode));
						buffer.push(item);
					};

					Ok(buffer)
				});

				buffer
			}));

			Ok(ArtistReference {
				name: name,
				albums: albums
			})
		})
	}
}

impl  Decodable for AlbumReference {
	fn decode<D: Decoder>(d: &mut D) -> Result<AlbumReference, D::Error> {
		d.read_struct("AlbumReference", 3, |d| {
			Ok(AlbumReference{
				id: try!(d.read_struct_field("id", 0, |d| d.read_str())),
				title: try!(d.read_struct_field("title", 0, |d| d.read_str())),
				//primary_type: try!(d.read_struct_field("primary-type", 0, |d| d.read_str())),
				image: None,
				error : false
			})
		})
	}
}

#[cfg(not(feature="meshup_mode_web"))]
fn query(id: &str) -> Result<ArtistReference, ResourceError> {
	query_mb_from_file(id)
}

#[cfg(feature="meshup_mode_web")]
fn query(id: &str) -> Result<ArtistReference, ResourceError> {
	query_mb_from_web(id)
}

fn main() {
	let args : Vec<String> = std::env::args().into_iter().collect();
	let id = &args[1];

//	let response = query_mb_from_file(id);
	let web_response = query(id).unwrap();

//	print!("{:?}", response);
	print!("{}", json::encode(&web_response).unwrap())
}
