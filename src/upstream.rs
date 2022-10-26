use std::collections::HashMap;

use camino::Utf8PathBuf;
use clap::Parser;
use dkregistry::errors::Error;
use dkregistry::v2::Client as InnerClient;
use humantime::Duration;
use serde::Deserialize;
use serde_with::serde_as;
use serde_with::DisplayFromStr;
use tokio::fs::read_to_string;
use tracing::info;
use tracing::warn;

#[derive(Clone, Debug)]
pub struct Client {
	pub client: InnerClient,
	pub manifest_invalidation_time: core::time::Duration,
	pub blob_invalidation_time: core::time::Duration,
}

pub struct Clients(HashMap<String, Client>);
impl Clients {
	pub fn get(&mut self, key: Option<&str>) -> Result<Client, Error> {
		match key {
			Some(key) => match self.0.get(key) {
				Some(v) => Ok(v.clone()),
				None => {
					warn!("Unknown namespace '{}' passed; configuring with default settings", key);
					self.insert(key.to_string(), SingleUpstreamConfig::new(key))?;
					Ok(self.0.get(key).unwrap().clone())
				}
			},
			None => self.get(Some(""))
		}
	}

	fn insert(&mut self, key: String, config: SingleUpstreamConfig) -> Result<(), Error> {
		self.0.insert(key, config.try_into()?);
		Ok(())
	}
}

impl FromIterator<(String, Client)> for Clients {
	fn from_iter<T: IntoIterator<Item = (String, Client)>>(iter: T) -> Self {
		Self(iter.into_iter().collect())
	}
}

const fn truth() -> bool {
	true
}

fn default_manifest_invalidation_time() -> Duration {
	core::time::Duration::from_secs(14 * 86400).into()
}

fn default_blob_invalidation_time() -> Duration {
	core::time::Duration::from_secs(14 * 86400).into()
}

#[serde_as]
#[derive(Clone, Debug, Deserialize)]
pub struct SingleUpstreamConfig {
	namespace: String,
	host: String,
	#[serde(default = "truth")]
	tls: bool,
	#[serde(default)]
	accept_invalid_certs: bool,
	#[serde(default)]
	user_agent: Option<String>,
	#[serde(default)]
	username: Option<String>,
	#[serde(default)]
	password: Option<String>,
	#[serde(default = "default_manifest_invalidation_time")]
	#[serde_as(as = "DisplayFromStr")]
	manifest_invalidation_time: Duration,
	#[serde(default = "default_blob_invalidation_time")]
	#[serde_as(as = "DisplayFromStr")]
	blob_invalidation_time: Duration,
}

impl SingleUpstreamConfig {
	fn new(namespace: &str) -> Self {
		Self::with_host(namespace, namespace)
	}

	fn with_host(namespace: &str, host: &str) -> Self {
		Self{
			namespace: namespace.to_owned(),
			host: host.to_owned(),
			tls: true,
			accept_invalid_certs: false,
			user_agent: None,
			username: None,
			password: None,
			manifest_invalidation_time: default_manifest_invalidation_time(),
			blob_invalidation_time: default_blob_invalidation_time()
		}
	}
}

impl TryFrom<SingleUpstreamConfig> for Client {
	type Error = Error;
	fn try_from(config: SingleUpstreamConfig) -> Result<Self, Self::Error> {
		let client = InnerClient::configure()
			.registry(&config.host)
			.insecure_registry(!config.tls)
			.accept_invalid_certs(config.accept_invalid_certs)
			.user_agent(config.user_agent)
			.username(config.username)
			.password(config.password)
			.build()?;
		Ok(Self{
			client,
			manifest_invalidation_time: config.manifest_invalidation_time.into(),
			blob_invalidation_time: config.blob_invalidation_time.into()
		})
	}
}

#[derive(Debug, Parser)]
pub struct UpstreamConfig {
	#[clap(env, long, default_value = "docker.io")]
	/// For requests made without a namespace (I'm looking at you, Docker), this namespace will be
	/// used.
	default_upstream_namespace: String,
	#[clap(env, long)]
	/// If not passed, will default to a configuration that works for Docker Hub.  If a client
	/// passes in a `ns` parameter that isn't found in the configuration, the namespace will be
	/// treated as an upstream hostname, and we will try to connect with TLS enabled, requiring
	/// valid certs, and with no user-agent/credentials.
	upstream_config_file: Option<Utf8PathBuf>
}

impl UpstreamConfig {
	pub async fn clients(&self) -> Result<Clients, Error> {
		let mut clients = match self.upstream_config_file.as_ref() {
			Some(file) => {
				let upstream_config = read_to_string(file).await.unwrap();
				let upstream_config: Vec<SingleUpstreamConfig> = serde_yaml::from_str(&upstream_config).unwrap();
				let upstream_config = upstream_config
					.into_iter()
					.map(|conf| {
						info!("Parsed upstream config: {:?}", conf);
						conf
					})
					.map(|conf| Ok::<_, Error>((conf.namespace.clone(), conf.try_into()?)))
					.collect::<Result<Vec<_>, _>>()?;
				upstream_config.into_iter().collect()
			},
			None => {
				let client = SingleUpstreamConfig{
					namespace: "docker.io".into(),
					host: "registry-1.docker.io".into(),
					tls: true,
					accept_invalid_certs: false,
					user_agent: None,
					username: None,
					password: None,
					manifest_invalidation_time: default_manifest_invalidation_time(),
					blob_invalidation_time: default_blob_invalidation_time()
				}.try_into()?;
				let mut map = HashMap::with_capacity(1);
				map.insert("docker.io".into(), client);
				Clients(map)
			}
		};
		let default_client = clients.get(Some(&self.default_upstream_namespace))?;
		clients.0.insert("".into(), default_client);
		Ok(clients)
	}
}

