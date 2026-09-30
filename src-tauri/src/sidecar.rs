//! Starts and stops a local server process (P2 dev spec §8, shared with whisper in P5 §3).
//! The model is loaded only when used, unloaded after an idle timeout or on Hibernate, and never
//! left running after Quit. What differs per server lives in a [`SidecarSpec`].

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::db::Db;
use crate::db::repo::app_state;
use crate::error::{AppError, AppResult};
use crate::events::{self, EventSink};
use crate::mode::{Mode, ModeManager};
use crate::settings::{Settings, SettingsStore};

const HEALTH_POLL: Duration = Duration::from_millis(250);

// ---------------------------------------------------------------- process abstraction

#[derive(Debug, Clone)]
pub struct LaunchSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub port: u16,
    pub log_file: Option<PathBuf>,
}

pub trait ChildHandle: Send {
    fn pid(&self) -> Option<u32>;
    fn is_alive(&mut self) -> bool;
    /// SIGTERM, wait up to `grace`, then SIGKILL.
    fn stop(&mut self, grace: Duration);
}

pub trait ProcessLauncher: Send + Sync {
    fn launch(&self, spec: &LaunchSpec) -> AppResult<Box<dyn ChildHandle>>;
    fn base_url(&self, port: u16) -> String {
        format!("http://127.0.0.1:{port}")
    }
}

pub struct RealLauncher;

struct RealChild(std::process::Child);

impl ChildHandle for RealChild {
    fn pid(&self) -> Option<u32> {
        Some(self.0.id())
    }
    fn is_alive(&mut self) -> bool {
        matches!(self.0.try_wait(), Ok(None))
    }
    fn stop(&mut self, grace: Duration) {
        terminate(self.0.id(), grace);
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl ProcessLauncher for RealLauncher {
    fn launch(&self, spec: &LaunchSpec) -> AppResult<Box<dyn ChildHandle>> {
        let mut cmd = std::process::Command::new(&spec.program);
        cmd.args(&spec.args).stdin(std::process::Stdio::null());
        match spec.log_file.as_ref().and_then(|p| std::fs::File::create(p).ok()) {
            Some(f) => {
                let f2 = f.try_clone().map_err(|e| AppError::Internal(e.to_string()))?;
                cmd.stdout(f).stderr(f2);
            }
            None => {
                cmd.stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null());
            }
        }
        let name = spec.program.file_name().unwrap_or_default().to_string_lossy();
        let child = cmd
            .spawn()
            .map_err(|e| AppError::ai("start_failed", format!("Could not start {name}: {e}")))?;
        Ok(Box::new(RealChild(child)))
    }
}

fn pid_alive(pid: u32) -> bool {
    // Signal 0 only checks that the process exists.
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

/// SIGTERM, then SIGKILL after `grace` if it is still running.
pub fn terminate(pid: u32, grace: Duration) {
    unsafe {
        libc::kill(pid as i32, libc::SIGTERM);
    }
    let until = Instant::now() + grace;
    while Instant::now() < until {
        if !pid_alive(pid) {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    unsafe {
        libc::kill(pid as i32, libc::SIGKILL);
    }
}

fn process_name(pid: u32) -> Option<String> {
    let out = std::process::Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "comm="])
        .output()
        .ok()?;
    let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!name.is_empty()).then_some(name)
}

// ---------------------------------------------------------------- binary resolution

/// First match wins (P2 dev spec §8.3): the Settings override, next to the app (bundled),
/// `<data>/bin/*/` (newest folder first), PATH, Homebrew.
pub fn resolve_binary(name: &str, override_path: Option<&str>, data_dir: &Path) -> Option<PathBuf> {
    let file = |p: PathBuf| p.is_file().then_some(p);
    if let Some(p) = override_path.map(PathBuf::from).and_then(file) {
        return Some(p);
    }
    if let Some(p) = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|d| d.join(name)))
        .and_then(file)
    {
        return Some(p);
    }
    if let Ok(entries) = std::fs::read_dir(data_dir.join("bin")) {
        let mut dirs: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
        dirs.sort();
        if let Some(p) = dirs.into_iter().rev().find_map(|d| file(d.join(name))) {
            return Some(p);
        }
    }
    if let Some(path) = std::env::var_os("PATH")
        && let Some(p) = std::env::split_paths(&path).find_map(|d| file(d.join(name)))
    {
        return Some(p);
    }
    ["/opt/homebrew/bin", "/usr/local/bin"]
        .into_iter()
        .find_map(|d| file(Path::new(d).join(name)))
}

fn free_port() -> AppResult<u16> {
    let l = TcpListener::bind("127.0.0.1:0").map_err(|e| AppError::Internal(format!("port: {e}")))?;
    Ok(l.local_addr().map_err(|e| AppError::Internal(e.to_string()))?.port())
}

fn random_key() -> String {
    use rand::Rng;
    let bytes: [u8; 16] = rand::rng().random();
    crate::ai::models::hex(&bytes)
}

pub fn available_memory() -> u64 {
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    sys.available_memory()
}

// ---------------------------------------------------------------- per-server spec

/// What to start: found by [`SidecarSpec::prepare`].
#[derive(Debug, Clone)]
pub struct Prepared {
    pub program: PathBuf,
    pub model_path: PathBuf,
    pub model_id: String,
    pub model_bytes: u64,
}

pub trait SidecarSpec: Send + Sync {
    /// "llm" | "stt" (carried by `ai://status`).
    fn component(&self) -> &'static str;
    /// Binary name, for the leftover-process check and the log file name.
    fn binary(&self) -> &'static str;
    /// For messages: "The AI engine", "The speech engine".
    fn label(&self) -> &'static str;
    /// `app_state` key holding the running pid.
    fn pid_key(&self) -> &'static str;
    /// Find the binary and the model (errors carry codes the UI handles).
    fn prepare(&self, settings: &Settings, data_dir: &Path) -> AppResult<Prepared>;
    fn args(&self, p: &Prepared, port: u16, api_key: &str, settings: &Settings) -> Vec<String>;
    /// RAM needed on top of the model file.
    fn memory_headroom(&self) -> u64;
    fn idle_timeout(&self, settings: &Settings) -> Duration;
}

// ---------------------------------------------------------------- manager

#[derive(Debug, Clone)]
pub struct Endpoint {
    pub base_url: String,
    pub api_key: String,
    pub model_id: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AiStatus {
    /// llm | stt
    pub component: String,
    /// unloaded | loading | ready | busy | error
    pub state: String,
    pub model_id: Option<String>,
    pub message: Option<String>,
}

struct Running {
    child: Box<dyn ChildHandle>,
    endpoint: Endpoint,
}

pub struct SidecarManager {
    spec: Box<dyn SidecarSpec>,
    pub(crate) db: Db,
    data_dir: PathBuf,
    settings: Arc<SettingsStore>,
    mode: Arc<ModeManager>,
    events: Arc<dyn EventSink>,
    launcher: Arc<dyn ProcessLauncher>,
    http: reqwest::Client,
    running: tokio::sync::Mutex<Option<Running>>,
    status: Mutex<AiStatus>,
    last_used: Mutex<Instant>,
    holds: AtomicUsize,
    busy: AtomicUsize,
    start_timeout: Duration,
}

/// Keeps the model loaded while alive (voice sessions).
pub struct HoldGuard(Arc<SidecarManager>);
impl Drop for HoldGuard {
    fn drop(&mut self) {
        self.0.holds.fetch_sub(1, Ordering::SeqCst);
        self.0.touch();
    }
}

/// Marks the model busy while a request runs.
pub struct BusyGuard(Arc<SidecarManager>);
impl Drop for BusyGuard {
    fn drop(&mut self) {
        if self.0.busy.fetch_sub(1, Ordering::SeqCst) == 1 {
            self.0.touch();
            self.0.set_state_if("busy", "ready");
        }
    }
}

impl SidecarManager {
    pub fn new(
        spec: Box<dyn SidecarSpec>,
        db: Db,
        data_dir: PathBuf,
        settings: Arc<SettingsStore>,
        mode: Arc<ModeManager>,
        events: Arc<dyn EventSink>,
        launcher: Arc<dyn ProcessLauncher>,
    ) -> Arc<Self> {
        let status = AiStatus {
            component: spec.component().into(),
            state: "unloaded".into(),
            model_id: None,
            message: None,
        };
        Arc::new(Self {
            spec,
            db,
            data_dir,
            settings,
            mode,
            events,
            launcher,
            http: reqwest::Client::builder().no_proxy().build().expect("http client"),
            running: tokio::sync::Mutex::new(None),
            status: Mutex::new(status),
            last_used: Mutex::new(Instant::now()),
            holds: AtomicUsize::new(0),
            busy: AtomicUsize::new(0),
            start_timeout: Duration::from_secs(90),
        })
    }

    pub fn with_start_timeout(self: Arc<Self>, d: Duration) -> Arc<Self> {
        let mut me = Arc::try_unwrap(self).ok().expect("with_start_timeout before sharing");
        me.start_timeout = d;
        Arc::new(me)
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub fn status(&self) -> AiStatus {
        self.status.lock().unwrap().clone()
    }

    fn set_status(&self, state: &str, model_id: Option<String>, message: Option<String>) {
        let s = AiStatus {
            component: self.spec.component().into(),
            state: state.into(),
            model_id,
            message,
        };
        *self.status.lock().unwrap() = s.clone();
        events::emit(self.events.as_ref(), events::AI_STATUS, &s);
    }

    fn set_state_if(&self, from: &str, to: &str) {
        let next = {
            let mut st = self.status.lock().unwrap();
            if st.state != from {
                return;
            }
            st.state = to.into();
            st.clone()
        };
        events::emit(self.events.as_ref(), events::AI_STATUS, &next);
    }

    pub fn touch(&self) {
        *self.last_used.lock().unwrap() = Instant::now();
    }

    pub fn hold(self: &Arc<Self>) -> HoldGuard {
        self.holds.fetch_add(1, Ordering::SeqCst);
        HoldGuard(self.clone())
    }

    pub fn busy(self: &Arc<Self>) -> BusyGuard {
        if self.busy.fetch_add(1, Ordering::SeqCst) == 0 {
            self.set_state_if("ready", "busy");
        }
        BusyGuard(self.clone())
    }

    /// Is the model loaded right now (without starting it)?
    pub fn is_ready(&self) -> bool {
        matches!(self.status().state.as_str(), "ready" | "busy")
    }

    /// Is the process running (any state)?
    pub async fn is_running(&self) -> bool {
        self.running.lock().await.is_some()
    }

    /// Start the server if needed and return where to reach it.
    /// `force` skips the low-memory check (the user said "start anyway").
    pub async fn ensure_ready(&self, force: bool) -> AppResult<Endpoint> {
        if self.mode.get() == Mode::Hibernate {
            return Err(AppError::Hibernating);
        }
        let mut guard = self.running.lock().await;
        if let Some(r) = guard.as_mut() {
            if r.child.is_alive() {
                self.touch();
                return Ok(r.endpoint.clone());
            }
            tracing::warn!(component = self.spec.component(), "sidecar exited unexpectedly");
            *guard = None;
        }
        let settings = self.settings.get();
        let prepared = self.spec.prepare(&settings, &self.data_dir)?;
        let label = self.spec.label();
        let need = prepared.model_bytes + self.spec.memory_headroom();
        let available = available_memory();
        if !force && available < need {
            return Err(AppError::ai(
                "low_memory",
                format!(
                    "Your Mac has {:.1} GB of free memory; {} needs about {:.1} GB.",
                    available as f64 / 1e9,
                    label.to_lowercase(),
                    need as f64 / 1e9
                ),
            ));
        }

        let port = free_port()?;
        let api_key = random_key();
        let model_id = prepared.model_id.clone();
        let spec = LaunchSpec {
            program: prepared.program.clone(),
            args: self.spec.args(&prepared, port, &api_key, &settings),
            port,
            log_file: Some(self.data_dir.join(format!("{}.log", self.spec.binary()))),
        };
        self.set_status("loading", Some(model_id.clone()), None);
        tracing::info!(component = self.spec.component(), model = %model_id, port, "starting sidecar");
        let mut child = match self.launcher.launch(&spec) {
            Ok(c) => c,
            Err(e) => {
                self.set_status("error", Some(model_id), Some(e.to_string()));
                return Err(e);
            }
        };
        if let Some(pid) = child.pid() {
            let key = self.spec.pid_key();
            let _ = self.db.call(move |c| app_state::set(c, key, &pid.to_string())).await;
        }
        let base_url = self.launcher.base_url(port);
        let started = Instant::now();
        loop {
            if !child.is_alive() {
                self.set_status(
                    "error",
                    Some(model_id),
                    Some(format!("{label} stopped while starting.")),
                );
                return Err(AppError::ai(
                    "start_failed",
                    format!("{label} stopped while starting. See {}.log.", self.spec.binary()),
                ));
            }
            let ok = self
                .http
                .get(format!("{base_url}/health"))
                .timeout(Duration::from_secs(2))
                .send()
                .await
                .map(|r| r.status().is_success())
                .unwrap_or(false);
            if ok {
                break;
            }
            if started.elapsed() > self.start_timeout {
                child.stop(Duration::from_secs(2));
                let msg = format!("{label} took too long to start.");
                self.set_status("error", Some(model_id), Some(msg.clone()));
                return Err(AppError::ai("start_failed", msg));
            }
            tokio::time::sleep(HEALTH_POLL).await;
        }
        tracing::info!(
            component = self.spec.component(),
            model = %model_id,
            secs = started.elapsed().as_secs_f32(),
            "sidecar ready"
        );
        let endpoint = Endpoint {
            base_url,
            api_key,
            model_id: model_id.clone(),
        };
        *guard = Some(Running {
            child,
            endpoint: endpoint.clone(),
        });
        self.touch();
        self.set_status("ready", Some(model_id), None);
        Ok(endpoint)
    }

    /// Stop the server (idle, Hibernate, Quit, model change).
    pub async fn shutdown(&self) {
        let mut guard = self.running.lock().await;
        if let Some(r) = guard.take() {
            tracing::info!(component = self.spec.component(), model = %r.endpoint.model_id, "stopping sidecar");
            let mut child = r.child;
            let _ = tokio::task::spawn_blocking(move || child.stop(Duration::from_secs(5))).await;
            let key = self.spec.pid_key();
            let _ = self.db.call(move |c| app_state::set(c, key, "")).await;
        }
        drop(guard);
        if self.status().state != "unloaded" {
            self.set_status("unloaded", None, None);
        }
    }

    /// Stop the server if nobody used it for `idle`.
    pub async fn shutdown_if_idle(&self, idle: Duration) -> bool {
        let quiet = self.holds.load(Ordering::SeqCst) == 0
            && self.busy.load(Ordering::SeqCst) == 0
            && self.last_used.lock().unwrap().elapsed() >= idle;
        if quiet && self.running.lock().await.is_some() {
            self.shutdown().await;
            return true;
        }
        false
    }

    /// Checks every 20 s whether the idle timeout has passed.
    pub fn spawn_idle_watch(self: &Arc<Self>) {
        let me = self.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(20)).await;
                let idle = me.spec.idle_timeout(&me.settings.get());
                me.shutdown_if_idle(idle).await;
            }
        });
    }

    /// Kill a server left over from a crash (its pid is stored in app_state).
    pub async fn cleanup_orphan(&self) {
        let key = self.spec.pid_key();
        let pid = self.db.call(move |c| app_state::get(c, key)).await.ok().flatten();
        if let Some(pid) = pid.and_then(|p| p.parse::<u32>().ok())
            && pid_alive(pid)
            && process_name(pid).is_some_and(|n| n.contains(self.spec.binary()))
        {
            tracing::warn!(pid, component = self.spec.component(), "killing leftover sidecar");
            terminate(pid, Duration::from_secs(3));
        }
        let _ = self.db.call(move |c| app_state::set(c, key, "")).await;
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::events::RecordingEventSink;
    use std::sync::atomic::AtomicBool;
    use wiremock::matchers::path;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    pub struct FakeChild(pub Arc<AtomicBool>);
    impl ChildHandle for FakeChild {
        fn pid(&self) -> Option<u32> {
            None
        }
        fn is_alive(&mut self) -> bool {
            self.0.load(Ordering::SeqCst)
        }
        fn stop(&mut self, _grace: Duration) {
            self.0.store(false, Ordering::SeqCst);
        }
    }

    /// Pretends to start a server; the "server" is a wiremock instance.
    pub struct FakeLauncher {
        pub base: String,
        pub launches: AtomicUsize,
        pub alive: Mutex<Vec<Arc<AtomicBool>>>,
        pub last: Mutex<Option<LaunchSpec>>,
    }
    impl ProcessLauncher for FakeLauncher {
        fn launch(&self, spec: &LaunchSpec) -> AppResult<Box<dyn ChildHandle>> {
            self.launches.fetch_add(1, Ordering::SeqCst);
            *self.last.lock().unwrap() = Some(spec.clone());
            let a = Arc::new(AtomicBool::new(true));
            self.alive.lock().unwrap().push(a.clone());
            Ok(Box::new(FakeChild(a)))
        }
        fn base_url(&self, _port: u16) -> String {
            self.base.clone()
        }
    }

    pub struct Rig {
        pub server: MockServer,
        pub launcher: Arc<FakeLauncher>,
        pub manager: Arc<SidecarManager>,
        pub mode: Arc<ModeManager>,
        pub _dir: tempfile::TempDir,
    }

    #[derive(Clone, Copy, Debug)]
    pub enum Kind {
        Llm,
        Whisper,
    }

    /// A manager with a fake server, a fake binary and a tiny "model" file for `kind`.
    pub async fn rig_for(kind: Kind, healthy: bool) -> Rig {
        let server = MockServer::start().await;
        Mock::given(path("/health"))
            .respond_with(ResponseTemplate::new(if healthy { 200 } else { 503 }))
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let fake_bin = dir.path().join("fake-server");
        std::fs::write(&fake_bin, b"#!/bin/sh").unwrap();
        let db = Db::open_in_memory().unwrap();
        let settings = SettingsStore::load(db.clone()).await.unwrap();
        let spec: Box<dyn SidecarSpec> = match kind {
            Kind::Llm => {
                let model = dir.path().join("m.gguf");
                std::fs::write(&model, b"gguf").unwrap();
                settings
                    .update(serde_json::json!({ "ai": {
                        "customModelPath": model.to_string_lossy(),
                        "llamaServerPath": fake_bin.to_string_lossy()
                    } }))
                    .await
                    .unwrap();
                Box::new(crate::ai::manager::LlmSidecar)
            }
            Kind::Whisper => {
                let models = crate::ai::models::models_dir(dir.path());
                std::fs::create_dir_all(&models).unwrap();
                std::fs::write(models.join("ggml-base.en.bin"), b"ggml").unwrap();
                settings
                    .update(serde_json::json!({ "ai": { "whisperServerPath": fake_bin.to_string_lossy() } }))
                    .await
                    .unwrap();
                Box::new(crate::voice::stt::WhisperSidecar)
            }
        };
        let events = Arc::new(RecordingEventSink::default());
        let mode = ModeManager::load(db.clone(), events.clone()).await.unwrap();
        let launcher = Arc::new(FakeLauncher {
            base: server.uri(),
            launches: AtomicUsize::new(0),
            alive: Mutex::new(vec![]),
            last: Mutex::new(None),
        });
        let manager = SidecarManager::new(
            spec,
            db,
            dir.path().to_path_buf(),
            settings,
            mode.clone(),
            events,
            launcher.clone(),
        )
        .with_start_timeout(Duration::from_millis(600));
        Rig {
            server,
            launcher,
            manager,
            mode,
            _dir: dir,
        }
    }

    const KINDS: [Kind; 2] = [Kind::Llm, Kind::Whisper];

    #[tokio::test(flavor = "multi_thread")]
    async fn concurrent_callers_share_one_start() {
        for kind in KINDS {
            let r = rig_for(kind, true).await;
            let (a, b) = tokio::join!(r.manager.ensure_ready(true), r.manager.ensure_ready(true));
            assert_eq!(a.unwrap().base_url, r.server.uri());
            b.unwrap();
            assert_eq!(r.launcher.launches.load(Ordering::SeqCst), 1, "{kind:?}");
            assert_eq!(r.manager.status().state, "ready");
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn health_timeout_is_an_error_and_stops_the_process() {
        for kind in KINDS {
            let r = rig_for(kind, false).await;
            let e = r.manager.ensure_ready(true).await.unwrap_err();
            assert_eq!(e.code(), "start_failed", "{kind:?}");
            assert!(!r.launcher.alive.lock().unwrap()[0].load(Ordering::SeqCst));
            assert_eq!(r.manager.status().state, "error");
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn idle_shutdown_respects_holds() {
        for kind in KINDS {
            let r = rig_for(kind, true).await;
            r.manager.ensure_ready(true).await.unwrap();
            let hold = r.manager.hold();
            assert!(!r.manager.shutdown_if_idle(Duration::ZERO).await, "held");
            drop(hold);
            assert!(r.manager.shutdown_if_idle(Duration::ZERO).await);
            assert_eq!(r.manager.status().state, "unloaded");
            assert!(!r.launcher.alive.lock().unwrap()[0].load(Ordering::SeqCst));
            assert!(
                !r.manager.shutdown_if_idle(Duration::from_secs(3600)).await,
                "nothing running"
            );
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn hibernate_refuses_and_restart_after_exit() {
        for kind in KINDS {
            let r = rig_for(kind, true).await;
            r.manager.ensure_ready(true).await.unwrap();
            // the server dies → next call starts a new one
            r.launcher.alive.lock().unwrap()[0].store(false, Ordering::SeqCst);
            r.manager.ensure_ready(true).await.unwrap();
            assert_eq!(r.launcher.launches.load(Ordering::SeqCst), 2);
            r.mode.set(Mode::Hibernate).await.unwrap();
            assert!(matches!(r.manager.ensure_ready(true).await, Err(AppError::Hibernating)));
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn busy_state_round_trip_and_component() {
        for (kind, component) in [(Kind::Llm, "llm"), (Kind::Whisper, "stt")] {
            let r = rig_for(kind, true).await;
            r.manager.ensure_ready(true).await.unwrap();
            {
                let _b = r.manager.busy();
                assert_eq!(r.manager.status().state, "busy");
            }
            let s = r.manager.status();
            assert_eq!((s.state.as_str(), s.component.as_str()), ("ready", component));
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn cleanup_kills_a_leftover_server_only() {
        for (kind, name) in [(Kind::Llm, "llama-server"), (Kind::Whisper, "whisper-server")] {
            let r = rig_for(kind, true).await;
            // A fake server: a copy of `sleep` with that name.
            let dir = tempfile::tempdir().unwrap();
            let fake = dir.path().join(name);
            std::fs::copy("/bin/sleep", &fake).unwrap();
            let mut orphan = std::process::Command::new(&fake).arg("30").spawn().unwrap();
            let mut other = std::process::Command::new("/bin/sleep").arg("30").spawn().unwrap();
            // A killed child stays a zombie until reaped, so check with try_wait.
            let exited = |c: &mut std::process::Child| {
                std::thread::sleep(Duration::from_millis(300));
                c.try_wait().unwrap().is_some()
            };
            let key = r.manager.spec.pid_key();
            let pid = orphan.id();
            r.manager
                .db
                .call(move |c| app_state::set(c, key, &pid.to_string()))
                .await
                .unwrap();
            r.manager.cleanup_orphan().await;
            assert!(exited(&mut orphan), "leftover {name} killed");
            let pid = other.id();
            r.manager
                .db
                .call(move |c| app_state::set(c, key, &pid.to_string()))
                .await
                .unwrap();
            r.manager.cleanup_orphan().await;
            assert!(!exited(&mut other), "a different program with that pid is left alone");
            let _ = other.kill();
            let _ = other.wait();
            let stored = r.manager.db.call(move |c| app_state::get(c, key)).await.unwrap();
            assert_eq!(stored.as_deref(), Some(""));
        }
    }

    #[test]
    fn binary_resolution_prefers_override() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("my-llama");
        std::fs::write(&bin, b"x").unwrap();
        assert_eq!(
            resolve_binary("llama-server", Some(bin.to_str().unwrap()), dir.path()),
            Some(bin)
        );
        let nested = dir.path().join("bin/llama-b2");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("llama-server"), b"x").unwrap();
        assert_eq!(
            resolve_binary("llama-server", None, dir.path()).unwrap(),
            nested.join("llama-server")
        );
        let w = dir.path().join("bin/whisper-1.9.4");
        std::fs::create_dir_all(&w).unwrap();
        std::fs::write(w.join("whisper-server"), b"x").unwrap();
        assert_eq!(
            resolve_binary("whisper-server", None, dir.path()).unwrap(),
            w.join("whisper-server")
        );
    }
}
