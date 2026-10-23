use std::{cmp::Reverse, collections::BTreeMap, fs, path::{Path, PathBuf}, time::Duration};

use anyhow::{Context, Result, bail, ensure};
use reqwest::{Url, blocking::Client};
use serde::Deserialize;
use serde_json::Value;

#[derive(Clone)]
pub struct Store {
    client: Client,
    base: Url,
}

#[derive(Clone, Debug, Deserialize)]
pub struct StoreSdk {
    pub name: String,
    #[serde(default)]
    pub metadata: Metadata,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Metadata {
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Clone)]
pub struct TrySdk {
    pub name: String,
    pub directory: PathBuf,
}

impl TrySdk {
    pub fn discover(root: &Path) -> Result<Vec<Self>> {
        let entries = match fs::read_dir(root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(error) => return Err(error).context("Cannot read local SDK try area"),
        };
        let mut sdks = Vec::new();
        for entry in entries {
            let entry = entry?;
            let directory = entry.path();
            if directory.is_dir() && let Some(name) = entry.file_name().to_str() {
                sdks.push(Self { name: format!("try-{name}"), directory });
            }
        }
        sdks.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(sdks)
    }

    pub fn metadata(&self, base: &str, arch: &str) -> Result<Metadata> {
        let name = self.name.strip_prefix("try-").context("Invalid local SDK name")?;
        for filename in [
            format!("{name}_{arch}_{base}.sdk"),
            format!("{name}_{arch}.sdk"),
            format!("{name}_all_{base}.sdk"),
            format!("{name}_all.sdk"),
        ] {
            let path = self.directory.join(&filename);
            if fs::File::open(&path).is_err() {
                continue;
            }
            fs::read_to_string(self.directory.join(format!("{filename}.sha3-384")))
                .context("Cannot read local SDK checksum")?;
            let yaml = fs::read_to_string(self.directory.join(format!("{filename}.yaml")))
                .context("Cannot read local SDK metadata")?;
            return serde_yaml::from_str(&yaml).context("Invalid local SDK metadata");
        }
        bail!("No compatible local artifact for {base} / {arch}")
    }
}

pub fn try_sdk_root() -> Option<PathBuf> {
    let data = std::env::var_os("XDG_DATA_HOME").filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("SNAP_REAL_HOME").or_else(|| std::env::var_os("HOME"))
            .map(|home| PathBuf::from(home).join(".local/share")))?;
    Some(data.join("workshop/try"))
}

#[derive(Clone, Debug, Deserialize)]
pub struct StoreInfo {
    pub name: String,
    #[serde(default)]
    pub metadata: Metadata,
    #[serde(default, rename = "channel-map")]
    pub channels: Vec<Release>,
    #[serde(default, rename = "default-track")]
    pub default_track: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Release {
    pub channel: Channel,
    #[serde(default)]
    pub revision: Revision,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Revision {
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub revision: u64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Channel {
    pub name: String,
    pub platform: Platform,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Platform {
    pub name: String,
    pub channel: String,
    pub architecture: String,
}

impl StoreInfo {
    pub fn compatible_releases(&self, base: &str, arch: &str) -> Vec<Release> {
        let mut channels: BTreeMap<&str, &Release> = BTreeMap::new();
        let (os, base_release) = base.split_once('@').unwrap_or((base, ""));
        for release in &self.channels {
            let p = &release.channel.platform;
            if (p.name == os || p.name == "all")
                && (p.channel == base_release || p.channel == "all")
                && (p.architecture == arch || p.architecture == "all")
            {
                let entry = channels.entry(&release.channel.name).or_insert(release);
                if release.revision.revision > entry.revision.revision {
                    *entry = release;
                }
            }
        }
        let preferred = format!(
            "{}/stable",
            self.default_track.as_deref().unwrap_or("latest")
        );
        let mut releases: Vec<_> = channels.into_values().cloned().collect();
        releases.sort_by_key(|r| {
            (
                r.channel.name != "latest/stable",
                r.channel.name != preferred,
                Reverse(r.revision.revision),
                r.channel.name.clone(),
            )
        });
        releases
    }
}

fn client() -> reqwest::blocking::ClientBuilder {
    Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent(concat!("workshop-tui/", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::none())
}

fn base_url(base: &str) -> Result<Url> {
    let mut url = Url::parse(base).context("Invalid API URL")?;
    ensure!(
        ["http", "https"].contains(&url.scheme()) && url.host_str().is_some(),
        "API URL must use HTTP or HTTPS and include a host"
    );
    ensure!(
        url.query().is_none() && url.fragment().is_none(),
        "API base URL cannot contain a query or fragment"
    );
    if !url.path().ends_with('/') {
        url.set_path(&format!("{}/", url.path()));
    }
    Ok(url)
}

fn endpoint(base: &Url, segments: &[&str]) -> Result<Url> {
    let mut url = base.clone();
    url.path_segments_mut()
        .map_err(|_| anyhow::anyhow!("Invalid API base URL"))?
        .pop_if_empty()
        .extend(segments.iter().copied());
    Ok(url)
}

fn request(client: &Client, url: Url) -> Result<Value> {
    let response = client
        .get(url.clone())
        .send()
        .with_context(|| format!("Cannot reach {url}"))?;
    let status = response.status();
    let body = response.text().context("Cannot read API response")?;
    ensure!(
        status.is_success(),
        "API returned HTTP {status}: {}",
        body.chars().take(500).collect::<String>()
    );
    let value: Value = serde_json::from_str(&body).context("API returned invalid JSON")?;
    if let Some(errors) = value.get("error-list").and_then(Value::as_array)
        && !errors.is_empty()
    {
        bail!("SDK Store error: {}", Value::Array(errors.clone()));
    }
    Ok(value)
}

impl Store {
    pub fn new(base: &str) -> Result<Self> {
        Ok(Self {
            client: client().build()?,
            base: base_url(base)?,
        })
    }

    pub fn find(&self, query: &str) -> Result<Vec<StoreSdk>> {
        let mut url = endpoint(&self.base, &["v2", "sdks", "find"])?;
        url.query_pairs_mut().append_pair(
            "fields",
            "default-release,description,license,publisher,summary",
        );
        if !query.is_empty() {
            url.query_pairs_mut().append_pair("q", query);
        }
        #[derive(Deserialize)]
        struct Results {
            results: Vec<StoreSdk>,
        }
        let mut sdks: Results = serde_json::from_value(request(&self.client, url)?)
            .context("Invalid SDK Store search response")?;
        sdks.results.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(sdks.results)
    }

    pub fn info(&self, name: &str) -> Result<StoreInfo> {
        let mut url = endpoint(&self.base, &["v2", "sdks", "info", name])?;
        url.query_pairs_mut().append_pair(
            "fields",
            "channel-map,description,publisher,summary,title,revision,version",
        );
        let info: StoreInfo = serde_json::from_value(request(&self.client, url)?)
            .context("Invalid SDK Store info response")?;
        ensure!(
            info.name == name,
            "SDK Store returned details for {} instead of {name}",
            info.name
        );
        Ok(info)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    #[test]
    fn local_try_artifacts_follow_workshop_precedence_and_require_sidecars() {
        let root = tempfile::tempdir().unwrap();
        assert!(TrySdk::discover(&root.path().join("missing")).unwrap().is_empty());
        let directory = root.path().join("tools");
        fs::create_dir(&directory).unwrap();
        fs::write(root.path().join("not-an-sdk"), "").unwrap();
        let sdks = TrySdk::discover(root.path()).unwrap();
        assert_eq!(sdks.len(), 1);
        assert_eq!(sdks[0].name, "try-tools");
        assert!(sdks[0].metadata("ubuntu@24.04", "amd64").is_err());
        for filename in ["tools_all.sdk", "tools_all_ubuntu@24.04.sdk", "tools_amd64.sdk", "tools_amd64_ubuntu@24.04.sdk"] {
            fs::write(directory.join(filename), "artifact").unwrap();
            fs::write(directory.join(format!("{filename}.yaml")), format!("name: tools\nsummary: {filename}\ndescription: Local tools\n")).unwrap();
            assert!(sdks[0].metadata("ubuntu@24.04", "amd64").is_err());
            fs::write(directory.join(format!("{filename}.sha3-384")), "digest").unwrap();
            let metadata = sdks[0].metadata("ubuntu@24.04", "amd64").unwrap();
            assert_eq!(metadata.summary, filename);
            assert_eq!(metadata.description, "Local tools");
        }
        assert_eq!(sdks[0].metadata("ubuntu@26.04", "arm64").unwrap().summary, "tools_all.sdk");
        fs::remove_file(directory.join("tools_all.sdk")).unwrap();
        assert!(sdks[0].metadata("ubuntu@26.04", "arm64").is_err());
    }

    fn server(body: &'static str, expected: &'static str) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut bytes = [0; 8192];
            let n = stream.read(&mut bytes).unwrap();
            let request = std::str::from_utf8(&bytes[..n]).unwrap();
            assert!(request.starts_with(expected), "{request}");
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });
        (format!("http://{addr}"), handle)
    }

    #[test]
    fn store_search_uses_v2_and_encodes_query() {
        let (url, handle) = server(
            r#"{"results":[{"name":"rust","metadata":{"summary":"Toolchain"}}]}"#,
            "GET /v2/sdks/find?",
        );
        let sdks = Store::new(&url).unwrap().find("rust & tools").unwrap();
        assert_eq!(sdks[0].name, "rust");
        assert_eq!(sdks[0].metadata.summary, "Toolchain");
        handle.join().unwrap();
        let mut url = endpoint(&base_url(&url).unwrap(), &["v2", "sdks", "info", "a/b"]).unwrap();
        assert!(url.path().ends_with("a%2Fb"));
        url.query_pairs_mut().append_pair("q", "rust & tools");
        assert_eq!(
            url.query_pairs().find(|(k, _)| k == "q").unwrap().1,
            "rust & tools"
        );
    }

    #[test]
    fn channel_filtering_and_deduplication() {
        let info: StoreInfo = serde_json::from_str(r#"{"name":"rust","channel-map":[
            {"channel":{"name":"latest/stable","platform":{"name":"ubuntu","channel":"24.04","architecture":"amd64"}}},
            {"channel":{"name":"latest/stable","platform":{"name":"ubuntu","channel":"24.04","architecture":"amd64"}}},
            {"channel":{"name":"latest/edge","platform":{"name":"ubuntu","channel":"22.04","architecture":"amd64"}}}
        ]}"#).unwrap();
        let releases = info.compatible_releases("ubuntu@24.04", "amd64");
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].channel.name, "latest/stable");
        assert!(info.compatible_releases("ubuntu@24.04", "arm64").is_empty());
    }

    #[test]
    fn defaults_to_latest_stable_with_newest_revision_and_accepts_platform_wildcards() {
        let info: StoreInfo = serde_json::from_str(r#"{
            "name":"opencode","default-track":null,
            "channel-map":[
                {"channel":{"name":"latest/edge","platform":{"name":"all","channel":"all","architecture":"amd64"}},
                 "revision":{"version":"9.0","revision":90}},
                {"channel":{"name":"latest/stable","platform":{"name":"all","channel":"all","architecture":"amd64"}},
                 "revision":{"version":"1.0","revision":10}},
                {"channel":{"name":"latest/stable","platform":{"name":"all","channel":"all","architecture":"amd64"}},
                 "revision":{"version":"2.0","revision":20}},
                {"channel":{"name":"latest/stable","platform":{"name":"all","channel":"all","architecture":"arm64"}},
                 "revision":{"version":"3.0","revision":30}}
            ]}"#).unwrap();
        for base in ["ubuntu@22.04", "ubuntu@24.04", "ubuntu@26.04"] {
            let releases = info.compatible_releases(base, "amd64");
            assert_eq!(releases.len(), 2);
            assert_eq!(releases[0].channel.name, "latest/stable");
            assert_eq!(releases[0].revision.version, "2.0");
            assert_eq!(releases[0].revision.revision, 20);
        }
        assert_eq!(
            info.compatible_releases("ubuntu@24.04", "arm64")[0]
                .revision
                .version,
            "3.0"
        );
        assert!(
            info.compatible_releases("ubuntu@24.04", "riscv64")
                .is_empty()
        );
    }

    #[test]
    fn falls_back_to_store_default_track_then_newest_compatible_revision() {
        let mut info: StoreInfo = serde_json::from_str(r#"{
            "name":"example","default-track":"2",
            "channel-map":[
                {"channel":{"name":"2/stable","platform":{"name":"all","channel":"all","architecture":"all"}},
                 "revision":{"version":"2.0","revision":20}},
                {"channel":{"name":"latest/edge","platform":{"name":"all","channel":"all","architecture":"all"}},
                 "revision":{"version":"3.0","revision":30}},
                {"channel":{"name":"latest/beta","platform":{"name":"all","channel":"all","architecture":"all"}},
                 "revision":{"version":"2.5","revision":25}}
            ]}"#).unwrap();
        assert_eq!(
            info.compatible_releases("ubuntu@24.04", "amd64")[0]
                .channel
                .name,
            "2/stable"
        );
        info.channels.remove(0);
        assert_eq!(
            info.compatible_releases("ubuntu@24.04", "amd64")[0]
                .channel
                .name,
            "latest/edge"
        );
    }

    #[test]
    fn surfaces_api_and_protocol_errors() {
        for body in [
            r#"{"error-list":[{"message":"Unavailable"}]}"#,
            r#"{"unexpected":true}"#,
            "not JSON",
        ] {
            let (url, handle) = server(body, "GET /v2/sdks/find?");
            assert!(Store::new(&url).unwrap().find("").is_err());
            handle.join().unwrap();
        }
    }

    #[test]
    fn http_failure_is_not_treated_as_success() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut bytes = [0; 8192];
            let n = stream.read(&mut bytes).unwrap();
            assert!(
                std::str::from_utf8(&bytes[..n])
                    .unwrap()
                    .starts_with("GET /v2/sdks/find?")
            );
            write!(stream, "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 4\r\nConnection: close\r\n\r\ndown").unwrap();
        });
        let error = Store::new(&format!("http://{addr}"))
            .unwrap()
            .find("")
            .unwrap_err();
        assert!(error.to_string().contains("503"));
        handle.join().unwrap();
    }

    #[test]
    #[ignore = "requires access to the public SDK Store"]
    fn live_store_search_and_compatible_channels() {
        let store = Store::new("https://api.pkg.store").unwrap();
        assert!(
            store
                .find("rust")
                .unwrap()
                .iter()
                .any(|sdk| sdk.name == "rust")
        );
        let info = store.info("rust").unwrap();
        assert_eq!(info.name, "rust");
        let releases = info.compatible_releases("ubuntu@24.04", "amd64");
        assert!(!releases.is_empty());
        assert!(!releases[0].revision.version.is_empty());
        let opencode = store.info("opencode").unwrap();
        assert!(!opencode.metadata.description.is_empty());
        let releases = opencode.compatible_releases("ubuntu@24.04", "amd64");
        assert!(!releases.is_empty());
        assert_eq!(releases[0].channel.name, "latest/stable");
        assert!(!releases[0].revision.version.is_empty());
    }
}
