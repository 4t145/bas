use super::*;
use std::sync::Arc;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::Notify,
};

pub struct Fixture {
    pub url: String,
    pub started: Arc<Notify>,
    pub release: Arc<Notify>,
    task: tokio::task::JoinHandle<()>,
}

impl Fixture {
    pub async fn serve(checksum: Option<String>, slow: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("fixture listener");
        let url = format!("http://{}", listener.local_addr().expect("fixture address"));
        let registry = serde_json::to_vec(&serde_json::json!({"version":"1.0.0","agents":[{
            "id":"fixture-agent","name":"Fixture Agent","version":"1.0.0","description":"managed fixture",
            "distribution":{"binary":{registry::platform():{"archive":format!("{url}/agent"),"cmd":"./agent","sha256":checksum,"args":["literal argument", "$NOT_EXPANDED"],"env":{"FIXTURE_ENV":"literal $value"}}}}
        }]})).expect("fixture registry JSON");
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let serve_started = started.clone();
        let serve_release = release.clone();
        let task = tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let registry = registry.clone();
                let started = serve_started.clone();
                let release = serve_release.clone();
                tokio::spawn(async move {
                    let mut request = vec![0_u8; 4096];
                    let size = stream.read(&mut request).await.expect("fixture request");
                    let request = String::from_utf8_lossy(&request[..size]);
                    let agent = request.starts_with("GET /agent ");
                    let body = if agent {
                        b"#!/bin/sh\nexit 0\n".to_vec()
                    } else {
                        registry
                    };
                    let header = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    if stream.write_all(header.as_bytes()).await.is_err() {
                        return;
                    }
                    if agent && slow {
                        started.notify_one();
                        release.notified().await;
                    }
                    let _ = stream.write_all(&body).await;
                });
            }
        });
        Self {
            url,
            started,
            release,
            task,
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
        self.release.notify_waiters();
    }
}

pub fn manager(directory: &tempfile::TempDir, url: &str) -> Arc<Manager> {
    manager_at(directory.path(), url)
}

pub fn manager_at(directory: &std::path::Path, url: &str) -> Arc<Manager> {
    let root = directory.join("data");
    fs::create_dir_all(&root).expect("manager data directory");
    Arc::new(Manager {
        store: Some(Store::at(directory.join("settings.json"))),
        profile: uuid::Uuid::new_v4().to_string(),
        root,
        url: format!("{url}/registry.json"),
        client: reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .expect("fixture HTTP client"),
        registry: Mutex::new(RegistryMemory::default()),
        mutation: Mutex::new(()),
        installations: TaskTracker::new(),
        stopping: CancellationToken::new(),
        admission: std::sync::Mutex::new(()),
    })
}

pub async fn install(manager: &Arc<Manager>) -> Result<AgentSpec> {
    let (sender, mut receiver) = mpsc::channel(32);
    let drain = tokio::spawn(async move { while receiver.recv().await.is_some() {} });
    let result = manager.install("fixture-agent", sender).await;
    drain.await.expect("progress drain");
    result
}

pub fn managed(manager: &Manager) -> bas::config::ManagedAgent {
    manager
        .store()
        .expect("settings store")
        .load()
        .expect("saved settings")
        .settings
        .daemon
        .agents
        .into_iter()
        .find_map(|registration| registration.managed().cloned())
        .expect("managed registration")
}
