extern crate hyper;
extern crate rustc_serialize;
extern crate url;

mod errors;
mod structs;

use errors::*;
use errors::ResourceError::*;
use structs::*;

use hyper::client::Client;
use std::fs::File;
use hyper::header::*;
use std::io::prelude::*;
use std::path::Path;
use std::thread;
use std::sync::Arc;
use rustc_serialize::json;
use hyper::mime::{Mime};
use std::sync::mpsc;
use hyper::status::StatusCode;
use url::Url;
//use std::fs;

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
	($id : expr) => ( format!("mb_{}.json", $id))
}

macro_rules! cover_art_url {
	($id : expr) => ( format!("http://coverartarchive.org/release-group/{}", $id) )
}

macro_rules! cover_art_file {
	($id : expr) => ( format!("ca_{}.json", $id) )
}

#[allow(dead_code)]
#[allow(unused_must_use)]
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
struct SimpleFs {
    directory : String
}

#[allow(dead_code)]
#[allow(unused_must_use)]
impl SimpleFs {
    fn read(&self, id: String) -> Result<String, TypedIOError> {
        std::fs::create_dir_all(Path::new(&self.directory));

        let path = Path::new(&self.directory).join(id);

        read_resource_from_file(path.as_path())
    }

    fn store(&self, id : &str, content: &str) {
        std::fs::create_dir_all(Path::new(&self.directory));

        let path = Path::new(&self.directory).join(id);

        if !path.exists() {
            File::create(path)
    			.and_then(|mut f| f.write_all(content.as_bytes()));
        };
    }
}

#[allow(unused_must_use)]
#[allow(dead_code)]
fn save_response_to_file(url : &str, content : &str, provider : &Provider) {
    let fs = provider.fs();
	let id = provider.extract_id(url);

    fs.store(&provider.format_file_name(&id), content);
}

trait Meshup {
    fn artist_resource_by_id (&self, id : &str) -> String;

    fn album_resource_by_id (&self, id : &str) -> String;

    fn query(&self, id : &str) -> Result<ArtistReference, ResourceError>;

    fn query_cover_art<F>(&self, artist_id: String, list_of_references: Vec<AlbumReference>, cover_art: F) -> Vec<AlbumReference>
    	where F: Send + 'static + Fn(&str)->Result<String, TypedIOError> + Sync {
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
}

struct FileMeshup;
struct WebMeshup;


fn read_resource_from_url(url : &str, provider : &Provider) -> Result<String, TypedIOError> {
    println_stderr!("invoking {}", url);

    let client = Client::new();
    let mime: Mime = "text/json".parse().unwrap();

    let response = client.get(url)
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
                save_response_to_file(url, &resp, provider);
            }

            resp
        });

    response
}

impl Meshup for WebMeshup {
    fn album_resource_by_id (&self, id : &str) -> String {
        cover_art_url!(id)
    }

    fn artist_resource_by_id (&self, id : &str) -> String {
        musicbrainz_url!(id)
    }

    fn query(&self, id : &str) -> Result<ArtistReference, ResourceError> {
        let mb_query_url = self.artist_resource_by_id(id);

        print!("{}", mb_query_url);

    	let mb_response = try!(read_resource_from_url(&mb_query_url, &Provider::Musicbrainz).map_err(|err| ArtistError {
    		artist_id: id.to_string(),
    		cause: err
    	}));

    	let artist_ref = process_mb_response(&mb_response);
    	let albums = self.query_cover_art(artist_ref.name.clone(), artist_ref.albums, |id| {
    		let url = cover_art_url!(id);
    		read_resource_from_url(&url, &Provider::CoverArt)
    	});

    	Ok(ArtistReference {
    		name : artist_ref.name.clone(),
    		albums : albums
    	})
    }
}

fn read_resource_from_file(path : &Path) -> Result<String, TypedIOError> {
    let mut content = String::new();

    File::open(&path)
        .and_then(|mut file| file.read_to_string(&mut content))
        .map(|_| {
            //return the content rather than the size
            content
        })
        .map_err(|err| TypedIOError {
            resource : path.to_str().unwrap_or("").to_string(),
            cause : hyper::Error::from(err)
        })
}

impl Meshup for FileMeshup {
    fn album_resource_by_id (&self, id : &str) -> String {
        musicbrainz_file!(id)
    }

    fn artist_resource_by_id (&self, id : &str) -> String {
        cover_art_file!(id)
    }

    fn query (&self, id : &str) -> Result<ArtistReference, ResourceError> {
        let mb_file = self.album_resource_by_id(id);
        let fs = Provider::Musicbrainz.fs();

    	let mb_response = try!(fs.read(mb_file).map_err(|err| {
    		ArtistError {
    			artist_id : id.to_string(),
    			cause: err.into()
    		}
    	}));

    	let artist_ref = process_mb_response(&mb_response);
    	let albums = self.query_cover_art(id.to_string(), artist_ref.albums, |id| {
    		let file_name = cover_art_file!(id);
            let fs = Provider::CoverArt.fs();

            fs.read(file_name)
    	});

    	Ok(ArtistReference {
    		name: artist_ref.name,
    		albums: albums
    	})
    }
}

#[allow(dead_code)]
fn query_cover_art<F>(artist_id: String, list_of_references: Vec<AlbumReference>, cover_art: F) -> Vec<AlbumReference>
	where F: Send + 'static + Fn(&str)->Result<String, TypedIOError> + Sync {
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
	let body : CoverArtResponse = json::decode(&payload).unwrap();

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

enum Provider {
    Musicbrainz,
    CoverArt
}

impl Provider {
    fn fs(&self) -> SimpleFs {
        match *self {
            Provider::Musicbrainz => SimpleFs { directory : "tmp".to_string() },
            Provider::CoverArt => SimpleFs { directory : "tmp".to_string() }
        }
    }

    fn extract_id(&self, url: &str) -> String {
		let parsed : Url = Url::parse(url).unwrap();

		parsed.path_segments().unwrap().last().unwrap().to_string()
	}

    fn format_file_name (&self, id : &str) -> String {
        match *self {
            Provider::Musicbrainz => musicbrainz_file!(id),
            Provider::CoverArt => cover_art_file!(id)
        }
    }
}


#[test]
fn test_extract_id_from_url() {
	let mb : String = musicbrainz_url!("1289836171-250");

	assert_eq!("1289836171-250", Provider::extract_id(&mb));
}

#[test]
fn test_format_file_name() {
	assert_eq!("ca_123", Provider::COVER_ART.format_file_name("123"));
	assert_eq!("mb_123", Provider::MUSICBRAINZ.format_file_name("123"));
}



#[cfg(not(feature="meshup_mode_web"))]
fn query(id: &str) -> Result<ArtistReference, ResourceError> {
    FileMeshup.query(id)
}

#[cfg(feature="meshup_mode_web")]
fn query(id: &str) -> Result<ArtistReference, ResourceError> {
	WebMeshup.query(id)
}

fn main() {
	let args : Vec<String> = std::env::args().into_iter().collect();
	let id = &args[1];

	let web_response = query(id).unwrap();

	print!("{}", json::encode(&web_response).unwrap())
}
