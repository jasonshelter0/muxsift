use std::{
    env,
    net::TcpListener,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use reqwest::Client;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::{
    process::{Child, Command},
    sync::Mutex,
    time::sleep,
};

use crate::{DownloadControl, DownloadProgress, Error, Result, Stream};

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/112.0.0.0 Safari/537.36";
const REFERER: &str = "https://www.bilibili.com";
static RPC_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone)]
pub struct Aria2Options {
    pub executable: PathBuf,
    pub max_concurrent_downloads: usize,
    pub split: u8,
    pub min_split_size: String,
}

impl Default for Aria2Options {
    fn default() -> Self {
        Self {
            executable: PathBuf::from("auto"),
            max_concurrent_downloads: 3,
            split: 10,
            min_split_size: "15M".into(),
        }
    }
}

/// One local aria2c RPC process shared by every media stream in a CLI/TUI run.
#[derive(Clone)]
pub struct Aria2Manager {
    inner: Arc<Inner>,
}

struct Inner {
    options: Aria2Options,
    rpc: Client,
    state: Mutex<Option<ProcessState>>,
}
struct ProcessState {
    endpoint: String,
    secret: String,
    child: Child,
}

#[derive(Deserialize)]
struct RpcResponse<T> {
    result: Option<T>,
    error: Option<RpcError>,
}
#[derive(Deserialize)]
struct RpcError {
    code: i64,
    message: String,
}
#[derive(Deserialize)]
struct Status {
    status: String,
    #[serde(rename = "completedLength", default)]
    completed_length: String,
    #[serde(rename = "totalLength", default)]
    total_length: String,
    #[serde(rename = "errorCode", default)]
    error_code: String,
    #[serde(rename = "errorMessage", default)]
    error_message: String,
}

impl Aria2Manager {
    pub fn new(options: Aria2Options) -> Result<Self> {
        if options.max_concurrent_downloads == 0 {
            return Err(Error::InvalidInput(
                "aria2 max_concurrent_downloads must be at least 1".into(),
            ));
        }
        let rpc = Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(3))
            .build()
            .map_err(|error| Error::Aria2(format!("could not create local RPC client: {error}")))?;
        Ok(Self {
            inner: Arc::new(Inner {
                options,
                rpc,
                state: Mutex::new(None),
            }),
        })
    }

    pub fn options(&self) -> &Aria2Options {
        &self.inner.options
    }

    pub async fn download_stream<F>(
        &self,
        stream: &Stream,
        path: &Path,
        cookie: &str,
        mut progress: F,
        control: Option<&DownloadControl>,
    ) -> Result<()>
    where
        F: FnMut(DownloadProgress) + Send,
    {
        let (endpoint, secret) = self.ensure_running().await?;
        let temporary = path.with_extension("part");
        let parent = temporary
            .parent()
            .ok_or_else(|| Error::Aria2("output path has no parent directory".into()))?;
        let file_name = temporary
            .file_name()
            .ok_or_else(|| Error::Aria2("output path has no file name".into()))?;
        let mut headers = vec![format!("Referer: {REFERER}"), format!("Origin: {REFERER}")];
        if !cookie.is_empty() {
            headers.push(format!("Cookie: {cookie}"));
        }
        let gid: String = rpc_call(&self.inner.rpc, &endpoint, "aria2.addUri", json!([
            format!("token:{secret}"), stream.urls().collect::<Vec<_>>(), {
                "dir": parent.to_string_lossy(), "out": file_name.to_string_lossy(), "header": headers,
                "continue": "true", "allow-overwrite": "true", "auto-file-renaming": "false",
                "split": self.inner.options.split.clamp(1, 16).to_string(), "min-split-size": self.inner.options.min_split_size,
            }
        ])).await?;
        let mut paused = false;
        loop {
            if control.is_some_and(DownloadControl::is_cancelled) {
                let _ = rpc_call::<Value>(
                    &self.inner.rpc,
                    &endpoint,
                    "aria2.remove",
                    json!([format!("token:{secret}"), gid]),
                )
                .await;
                let _ = tokio::fs::remove_file(&temporary).await;
                return Err(Error::Cancelled);
            }
            let should_pause = control.is_some_and(DownloadControl::is_paused);
            if should_pause != paused {
                let method = if should_pause {
                    "aria2.pause"
                } else {
                    "aria2.unpause"
                };
                rpc_call::<String>(
                    &self.inner.rpc,
                    &endpoint,
                    method,
                    json!([format!("token:{secret}"), gid]),
                )
                .await?;
                paused = should_pause;
            }
            let status: Status = rpc_call(
                &self.inner.rpc,
                &endpoint,
                "aria2.tellStatus",
                json!([
                    format!("token:{secret}"),
                    gid,
                    [
                        "status",
                        "completedLength",
                        "totalLength",
                        "errorCode",
                        "errorMessage"
                    ]
                ]),
            )
            .await?;
            progress(DownloadProgress {
                downloaded: status.completed_length.parse().unwrap_or(0),
                total: status.total_length.parse().ok().filter(|value| *value > 0),
            });
            match status.status.as_str() {
                "complete" => {
                    tokio::fs::rename(&temporary, path)
                        .await
                        .map_err(|source| Error::Io {
                            path: path.to_owned(),
                            source,
                        })?;
                    return Ok(());
                }
                "error" | "removed" => {
                    return Err(Error::Aria2(if status.error_message.is_empty() {
                        format!("status={} code={}", status.status, status.error_code)
                    } else {
                        status.error_message
                    }));
                }
                _ => sleep(Duration::from_millis(if paused { 150 } else { 300 })).await,
            }
        }
    }

    pub async fn shutdown(&self) {
        let mut state = self.inner.state.lock().await;
        if let Some(mut process) = state.take() {
            let _ = rpc_call::<String>(
                &self.inner.rpc,
                &process.endpoint,
                "aria2.forceShutdown",
                json!([format!("token:{}", process.secret)]),
            )
            .await;
            let _ = process.child.kill().await;
            let _ = process.child.wait().await;
        }
    }

    async fn ensure_running(&self) -> Result<(String, String)> {
        let mut state = self.inner.state.lock().await;
        if let Some(process) = state.as_mut() {
            if process
                .child
                .try_wait()
                .map_err(|error| Error::Aria2(format!("could not inspect aria2c: {error}")))?
                .is_none()
            {
                return Ok((process.endpoint.clone(), process.secret.clone()));
            }
            *state = None;
        }
        let port = reserve_local_port()?;
        let secret = rpc_secret();
        let endpoint = format!("http://127.0.0.1:{port}/jsonrpc");
        let options = &self.inner.options;
        let executable = resolve_executable(&options.executable)?;
        let child = Command::new(&executable)
            .kill_on_drop(true)
            .args([
                "--enable-rpc=true",
                "--rpc-listen-all=false",
                &format!("--rpc-listen-port={port}"),
                &format!("--rpc-secret={secret}"),
                "--continue=true",
                "--file-allocation=none",
                "--auto-file-renaming=false",
                "--check-certificate=true",
                "--user-agent",
                USER_AGENT,
                "--referer",
                REFERER,
                &format!(
                    "--max-concurrent-downloads={}",
                    options.max_concurrent_downloads
                ),
                "--max-connection-per-server=10",
                &format!("--split={}", options.split.clamp(1, 16)),
                &format!("--min-split-size={}", options.min_split_size),
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| match error.kind() {
                std::io::ErrorKind::NotFound => {
                    Error::Aria2Missing(executable.display().to_string())
                }
                _ => Error::Io {
                    path: executable.clone(),
                    source: error,
                },
            })?;
        for _ in 0..30 {
            if rpc_call::<Value>(
                &self.inner.rpc,
                &endpoint,
                "aria2.getVersion",
                json!([format!("token:{secret}")]),
            )
            .await
            .is_ok()
            {
                *state = Some(ProcessState {
                    endpoint: endpoint.clone(),
                    secret: secret.clone(),
                    child,
                });
                return Ok((endpoint, secret));
            }
            sleep(Duration::from_millis(100)).await;
        }
        Err(Error::Aria2(
            "aria2c did not start its local RPC server within 3 seconds".into(),
        ))
    }
}

fn resolve_executable(configured: &Path) -> Result<PathBuf> {
    let bundled_preferred = matches!(
        configured.to_string_lossy().as_ref(),
        "auto" | "aria2c" | "aria2c.exe"
    );
    if !bundled_preferred {
        return Ok(configured.to_owned());
    }
    let executable_name = if cfg!(windows) {
        "aria2c.exe"
    } else {
        "aria2c"
    };
    let current = env::current_exe().map_err(|source| {
        Error::Aria2(format!(
            "could not determine CLI executable location: {source}"
        ))
    })?;
    let parent = current
        .parent()
        .ok_or_else(|| Error::Aria2("CLI executable location has no parent directory".into()))?;
    for candidate in [
        parent.join("bin").join(executable_name),
        parent.join(executable_name),
    ] {
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    if configured != Path::new("auto") {
        // An explicit bare command still works as a PATH fallback for developers.
        return Ok(configured.to_owned());
    }
    Err(Error::Aria2Missing(format!(
        "bundled {} was not found beside {}; expected bin{}{}. Supply --aria2 /path/to/aria2c or install aria2",
        executable_name,
        current.display(),
        std::path::MAIN_SEPARATOR,
        executable_name,
    )))
}

fn reserve_local_port() -> Result<u16> {
    TcpListener::bind("127.0.0.1:0")
        .map_err(|source| Error::Aria2(format!("could not reserve local RPC port: {source}")))?
        .local_addr()
        .map(|address| address.port())
        .map_err(|source| Error::Aria2(format!("could not read local RPC port: {source}")))
}
fn rpc_secret() -> String {
    let clock = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("muxsift-{}-{clock:x}", std::process::id())
}
async fn rpc_call<T: for<'de> Deserialize<'de>>(
    rpc: &Client,
    endpoint: &str,
    method: &str,
    params: Value,
) -> Result<T> {
    let id = RPC_ID.fetch_add(1, Ordering::Relaxed);
    let response = rpc
        .post(endpoint)
        .json(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
        .send()
        .await
        .map_err(|error| Error::Aria2(format!("{method}: {error}")))?;
    let body: RpcResponse<T> = response
        .json()
        .await
        .map_err(|error| Error::Aria2(format!("{method}: invalid RPC response: {error}")))?;
    if let Some(result) = body.result {
        return Ok(result);
    }
    if let Some(error) = body.error {
        return Err(Error::Aria2(format!(
            "{method}: RPC {}: {}",
            error.code, error.message
        )));
    }
    Err(Error::Aria2(format!(
        "{method}: RPC response has no result"
    )))
}
