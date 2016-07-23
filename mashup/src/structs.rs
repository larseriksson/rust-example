extern crate hyper;
extern crate rustc_serialize;

use std;
use errors::ResourceError;
use errors::ResourceError::*;
use rustc_serialize::{json, Decodable, Decoder};

#[derive(RustcDecodable, Debug)]
pub struct CoverArtResponse {
	pub images : Vec<self::Image>
}

#[derive(RustcDecodable, Debug)]
pub struct Image {
	pub front : bool,
	pub image : String
}

#[derive(Debug, RustcEncodable)]
pub struct ArtistReference {
	pub name: String,
	pub albums : Vec<AlbumReference>
}

#[derive(Debug, Clone, RustcEncodable)]
pub struct AlbumReference {
	pub id: String,
	pub title: String,
	pub image : Option<String>,
	pub error : bool
}

impl AlbumReference {
	pub fn with_image(&mut self, image : String) -> &AlbumReference {
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
