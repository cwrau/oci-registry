use core::time::Duration;
use std::time::SystemTime;

use actix_web::web::Bytes;
use async_stream::try_stream;
use camino::Utf8Component;
use camino::Utf8Path;
use camino::Utf8PathBuf;
use clap::Parser;
use futures::stream::BoxStream;
use futures::stream::TryStream;
use futures::stream::TryStreamExt;
use tokio::fs::create_dir_all;
use tokio::fs::symlink_metadata;
use tokio::fs::File;
use tokio::fs::OpenOptions;
use tokio::io::AsyncBufReadExt;
use tokio::io::AsyncWriteExt;
use tokio::io::BufReader;
use tokio::io::BufWriter;

#[derive(Clone, Debug, Parser)]
pub struct Config {
	#[clap(env = "FILESYSTEM_ROOT", long)]
	root: Utf8PathBuf
}

impl Config {
	pub fn repository(&self) -> Repository {
		Repository{
			root: self.root.clone()
		}
	}
}

#[derive(Debug, Clone)]
pub struct Repository {
	root: Utf8PathBuf
}

impl Repository {
	fn full_path(&self, path: &Utf8Path) -> Utf8PathBuf {
		self.root.join(path
			.components()
			.filter(|c| matches!(c, Utf8Component::ParentDir | Utf8Component::Normal(_)))
			.collect::<Utf8PathBuf>()
		)
	}

	pub async fn read(self, object: &Utf8Path, invalidation: Duration) -> Result<BoxStream<'static, Result<Bytes, std::io::Error>>, super::Error> {
		let path = self.full_path(&object);
		let age = {
			let metadata = symlink_metadata(&path).await?;
			SystemTime::now().duration_since(metadata.modified()?).unwrap_or_default()
		};
		if(age > invalidation) {
			return Err(super::Error::ObjectTooOld(age.into()));
		}
		let mut file = BufReader::with_capacity(16384, File::open(path).await?);
		Ok(Box::pin(try_stream! {
			loop {
				let buf = file.fill_buf().await?;
				if(buf.len() == 0) {
					break;
				}
				let len = buf.len();
				yield Bytes::copy_from_slice(buf);
				file.consume(len);
			}
		}))
	}

	pub async fn write<S, E>(&self, object: &Utf8Path, mut reader: S) -> Result<(), super::Error>
	where
		S: TryStream<Ok = Bytes, Error = E> + Unpin,
		super::Error: From<E>
	{
		let path = self.full_path(object);
		if let Some(parent) = path.parent() {
			create_dir_all(parent).await?;
		}
		let file = OpenOptions::default().create(true).read(false).write(true).truncate(true).open(&path).await?;
		let mut file = BufWriter::with_capacity(16384, file);
		while let Some(buf) = reader.try_next().await? {
			if(buf.len() == 0) {
				break;
			}
			file.write_all(buf.as_ref()).await?;
		}
		file.flush().await?;
		Ok(())
	}
}

