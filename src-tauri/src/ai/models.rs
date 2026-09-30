//! Model catalog (`resources/models.json`) and a resumable, checksummed downloader (P2 dev spec §7).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use futures::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ModelEntry {
    pub id: String,
    pub role: String,
    pub display_name: String,
    pub file: String,
    pub url: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub context_length: u32,
    pub license: String,
    pub license_url: String,
    pub recommended_ram_gb: u32,
    /// Hybrid "thinking" model: thinking is switched off for this app's short answers.
    #[serde(default)]
    pub thinking: bool,
    #[serde(default)]
    pub default: bool,
}

#[derive(Deserialize)]
struct CatalogFile {
    models: Vec<ModelEntry>,
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn catalog() -> Vec<ModelEntry> {
    serde_json::from_str::<CatalogFile>(include_str!("../../resources/models.json"))
        .expect("valid models.json")
        .models
}

pub fn models_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("models")
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    #[serde(flatten)]
    pub entry: ModelEntry,
    pub downloaded: bool,
    pub active: bool,
    pub partial_bytes: u64,
    pub downloading: bool,
}

/// The model to use: the chosen one if downloaded, else the default, else any downloaded chat model.
pub fn resolve_active(data_dir: &Path, chosen: Option<&str>) -> Option<ModelEntry> {
    let dir = models_dir(data_dir);
    let downloaded = |m: &ModelEntry| dir.join(&m.file).is_file();
    let all = catalog();
    chosen
        .and_then(|id| all.iter().find(|m| m.id == id && downloaded(m)))
        .or_else(|| all.iter().find(|m| m.default && downloaded(m)))
        .or_else(|| all.iter().find(|m| m.role == "chat" && downloaded(m)))
        .cloned()
}

pub fn free_disk_bytes(path: &Path) -> Option<u64> {
    let disks = sysinfo::Disks::new_with_refreshed_list();
    disks
        .list()
        .iter()
        .filter(|d| path.starts_with(d.mount_point()))
        .max_by_key(|d| d.mount_point().as_os_str().len())
        .map(|d| d.available_space())
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub bytes: u64,
    pub total: u64,
}

#[derive(Default)]
pub struct Downloader {
    running: Mutex<HashMap<String, CancellationToken>>,
}

impl Downloader {
    pub fn is_running(&self, id: &str) -> bool {
        self.running.lock().unwrap().contains_key(id)
    }

    pub fn cancel(&self, id: &str) {
        if let Some(t) = self.running.lock().unwrap().get(id) {
            t.cancel();
        }
    }

    /// Download `entry` into `dir`, resuming a `.part` file, verifying sha256.
    /// `progress` is called at most 4 times per second.
    pub async fn download(
        &self,
        client: &reqwest::Client,
        entry: &ModelEntry,
        dir: &Path,
        progress: impl Fn(Progress) + Send,
    ) -> AppResult<PathBuf> {
        let token = CancellationToken::new();
        {
            let mut running = self.running.lock().unwrap();
            if running.contains_key(&entry.id) {
                return Err(AppError::Invalid("This model is already downloading".into()));
            }
            running.insert(entry.id.clone(), token.clone());
        }
        let result = download_file(client, entry, dir, &token, progress).await;
        self.running.lock().unwrap().remove(&entry.id);
        result
    }
}

async fn download_file(
    client: &reqwest::Client,
    entry: &ModelEntry,
    dir: &Path,
    token: &CancellationToken,
    progress: impl Fn(Progress) + Send,
) -> AppResult<PathBuf> {
    let io = |e: std::io::Error| AppError::Internal(format!("file: {e}"));
    tokio::fs::create_dir_all(dir).await.map_err(io)?;
    let final_path = dir.join(&entry.file);
    if final_path.is_file() {
        return Ok(final_path);
    }
    let part = dir.join(format!("{}.part", entry.file));
    let mut have = tokio::fs::metadata(&part).await.map(|m| m.len()).unwrap_or(0);
    if let Some(free) = free_disk_bytes(dir)
        && free < entry.size_bytes.saturating_sub(have) + 1_000_000_000
    {
        return Err(AppError::Invalid(format!(
            "Not enough disk space: need {:.1} GB free",
            (entry.size_bytes - have) as f64 / 1e9 + 1.0
        )));
    }

    let mut req = client.get(&entry.url);
    if have > 0 {
        req = req.header(reqwest::header::RANGE, format!("bytes={have}-"));
    }
    let resp = req.send().await.map_err(|e| AppError::Network(e.to_string()))?;
    let status = resp.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(AppError::Network(format!("HTTP {status}")));
    }
    // 206 → append; 200 → the server ignored the range, start again.
    let resume = status == 206 && have > 0;
    let mut hasher = Sha256::new();
    let mut file = if resume {
        let mut existing = tokio::fs::File::open(&part).await.map_err(io)?;
        let mut buf = vec![0u8; 1 << 20];
        loop {
            let n = existing.read(&mut buf).await.map_err(io)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        tokio::fs::OpenOptions::new()
            .append(true)
            .open(&part)
            .await
            .map_err(io)?
    } else {
        have = 0;
        tokio::fs::File::create(&part).await.map_err(io)?
    };

    let mut stream = resp.bytes_stream();
    let mut last = Instant::now() - Duration::from_secs(1);
    loop {
        tokio::select! {
            _ = token.cancelled() => {
                file.flush().await.map_err(io)?;
                return Err(AppError::Invalid("Download cancelled".into()));
            }
            chunk = stream.next() => match chunk {
                None => break,
                Some(Err(e)) => return Err(AppError::Network(e.to_string())),
                Some(Ok(bytes)) => {
                    file.write_all(&bytes).await.map_err(io)?;
                    hasher.update(&bytes);
                    have += bytes.len() as u64;
                    if last.elapsed() >= Duration::from_millis(250) {
                        last = Instant::now();
                        progress(Progress { bytes: have, total: entry.size_bytes });
                    }
                }
            }
        }
    }
    file.flush().await.map_err(io)?;
    drop(file);
    progress(Progress {
        bytes: have,
        total: entry.size_bytes,
    });
    let got = hex(&hasher.finalize());
    if !got.eq_ignore_ascii_case(&entry.sha256) {
        let _ = tokio::fs::remove_file(&part).await;
        return Err(AppError::Invalid(
            "The downloaded file is damaged (checksum mismatch). Try again.".into(),
        ));
    }
    tokio::fs::rename(&part, &final_path).await.map_err(io)?;
    Ok(final_path)
}

pub fn list(data_dir: &Path, chosen: Option<&str>, downloader: &Downloader) -> Vec<ModelInfo> {
    let dir = models_dir(data_dir);
    let active = resolve_active(data_dir, chosen).map(|m| m.id);
    catalog()
        .into_iter()
        .map(|m| {
            let partial = std::fs::metadata(dir.join(format!("{}.part", m.file)))
                .map(|x| x.len())
                .unwrap_or(0);
            ModelInfo {
                downloaded: dir.join(&m.file).is_file(),
                active: active.as_deref() == Some(m.id.as_str()),
                partial_bytes: partial,
                downloading: downloader.is_running(&m.id),
                entry: m,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn entry(url: String, body: &[u8]) -> ModelEntry {
        ModelEntry {
            id: "tiny".into(),
            role: "chat".into(),
            display_name: "Tiny".into(),
            file: "tiny.gguf".into(),
            url,
            size_bytes: body.len() as u64,
            sha256: hex(&Sha256::digest(body)),
            context_length: 4096,
            license: "MIT".into(),
            license_url: "https://x".into(),
            recommended_ram_gb: 1,
            thinking: false,
            default: true,
        }
    }

    #[test]
    fn catalog_is_valid() {
        let c = catalog();
        assert!(c.iter().filter(|m| m.default).count() == 1);
        assert!(c.iter().all(|m| m.sha256.len() == 64 && m.url.starts_with("https://")));
    }

    #[tokio::test]
    async fn downloads_verifies_and_resumes() {
        let body: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        let s = MockServer::start().await;
        Mock::given(path("/m.gguf"))
            .and(header("range", "bytes=50000-"))
            .respond_with(ResponseTemplate::new(206).set_body_bytes(body[50_000..].to_vec()))
            .mount(&s)
            .await;
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("tiny.gguf.part"), &body[..50_000]).unwrap();
        let e = entry(format!("{}/m.gguf", s.uri()), &body);
        let seen = std::sync::Mutex::new(0u64);
        let out = Downloader::default()
            .download(&reqwest::Client::new(), &e, dir.path(), |p| {
                *seen.lock().unwrap() = p.bytes
            })
            .await
            .unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), body);
        assert_eq!(*seen.lock().unwrap(), body.len() as u64);
        assert!(!dir.path().join("tiny.gguf.part").exists());
    }

    #[tokio::test]
    async fn restarts_when_range_ignored_and_rejects_bad_checksum() {
        let body = b"hello model".to_vec();
        let s = MockServer::start().await;
        Mock::given(path("/ok"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(body.clone()))
            .mount(&s)
            .await;
        Mock::given(path("/bad"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"other".to_vec()))
            .mount(&s)
            .await;
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("tiny.gguf.part"), b"garbage").unwrap();
        let d = Downloader::default();
        let out = d
            .download(
                &reqwest::Client::new(),
                &entry(format!("{}/ok", s.uri()), &body),
                dir.path(),
                |_| {},
            )
            .await
            .unwrap();
        assert_eq!(std::fs::read(out).unwrap(), body);

        let dir2 = tempfile::tempdir().unwrap();
        let err = d
            .download(
                &reqwest::Client::new(),
                &entry(format!("{}/bad", s.uri()), &body),
                dir2.path(),
                |_| {},
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("damaged"));
        assert!(!dir2.path().join("tiny.gguf.part").exists());
    }

    #[tokio::test]
    async fn cancel_keeps_the_part_file() {
        let body = vec![7u8; 2_000_000];
        let s = MockServer::start().await;
        Mock::given(path("/slow"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(body.clone())
                    .set_delay(Duration::from_millis(300)),
            )
            .mount(&s)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let d = std::sync::Arc::new(Downloader::default());
        let e = entry(format!("{}/slow", s.uri()), &body);
        let (d2, p) = (d.clone(), dir.path().to_path_buf());
        let task = tokio::spawn(async move { d2.download(&reqwest::Client::new(), &e, &p, |_| {}).await });
        tokio::time::sleep(Duration::from_millis(100)).await;
        d.cancel("tiny");
        let r = task.await.unwrap();
        assert!(r.unwrap_err().to_string().contains("cancelled"));
        assert!(dir.path().join("tiny.gguf.part").exists());
    }
}
