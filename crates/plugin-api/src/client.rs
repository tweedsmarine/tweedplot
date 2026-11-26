//! The app's side: start a plugin and talk to it.
//!
//! Requests may come from many threads at once (the chart store draws tiles in
//! parallel); each gets its own id and waits for its own reply. A reader thread hands
//! replies to the waiting requests and events to the event handler.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

use crate::frame::{read_frame, write_frame};
use crate::Error;

type Reply = Result<(Value, Vec<u8>), String>;
type Handler = Box<dyn Fn(&str, &Value) + Send + Sync>;

pub struct PluginClient {
    child: Mutex<Option<std::process::Child>>,
    writer: Mutex<Box<dyn Write + Send>>,
    next: AtomicU64,
    pending: Arc<Mutex<HashMap<u64, mpsc::Sender<Reply>>>>,
    alive: Arc<AtomicBool>,
    events: Arc<Mutex<Option<Handler>>>,
}

impl PluginClient {
    /// Start the plugin program `exe` with `args`. Its error output goes to the app's.
    pub fn spawn(exe: &Path, args: &[&str]) -> Result<PluginClient, Error> {
        let mut cmd = std::process::Command::new(exe);
        cmd.args(args).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::inherit());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // no console window flashing up from the GUI app
            cmd.creation_flags(0x0800_0000);
        }
        let mut child = cmd.spawn()?;
        let stdin = child.stdin.take().ok_or(Error::Closed)?;
        let stdout = child.stdout.take().ok_or(Error::Closed)?;
        let c = Self::from_streams(stdout, stdin);
        *c.child.lock().unwrap_or_else(|e| e.into_inner()) = Some(child);
        Ok(c)
    }

    /// A client over any pair of streams (tests run a server in a thread).
    pub fn from_streams(reader: impl Read + Send + 'static, writer: impl Write + Send + 'static) -> PluginClient {
        let pending: Arc<Mutex<HashMap<u64, mpsc::Sender<Reply>>>> = Arc::default();
        let alive = Arc::new(AtomicBool::new(true));
        let events: Arc<Mutex<Option<Handler>>> = Arc::default();
        let (p, a, ev) = (pending.clone(), alive.clone(), events.clone());
        std::thread::Builder::new()
            .name("plugin-reader".into())
            .spawn(move || {
                let mut r = std::io::BufReader::with_capacity(1 << 16, reader);
                while let Ok(Some((h, body))) = read_frame(&mut r) {
                    if let Some(id) = h["id"].as_u64() {
                        let reply = if h["ok"].as_bool() == Some(true) {
                            Ok((h["result"].clone(), body))
                        } else {
                            Err(h["error"].as_str().unwrap_or("the plugin refused").to_string())
                        };
                        if let Some(tx) = p.lock().unwrap_or_else(|e| e.into_inner()).remove(&id) {
                            let _ = tx.send(reply);
                        }
                    } else if let Some(name) = h["event"].as_str() {
                        if let Some(f) = ev.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
                            f(name, &h["data"]);
                        }
                    }
                }
                a.store(false, Ordering::SeqCst);
                // anyone still waiting hears that it stopped
                for (_, tx) in p.lock().unwrap_or_else(|e| e.into_inner()).drain() {
                    let _ = tx.send(Err("the plugin stopped".into()));
                }
            })
            .expect("reader thread");
        PluginClient {
            child: Mutex::new(None),
            writer: Mutex::new(Box::new(std::io::BufWriter::with_capacity(1 << 16, writer))),
            next: AtomicU64::new(1),
            pending,
            alive,
            events,
        }
    }

    /// Whether the plugin is still running.
    pub fn alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }

    /// Call `f(event, data)` for each event the plugin sends.
    pub fn on_event(&self, f: impl Fn(&str, &Value) + Send + Sync + 'static) {
        *self.events.lock().unwrap_or_else(|e| e.into_inner()) = Some(Box::new(f));
    }

    /// Send a request and wait for its reply (result and body).
    pub fn request(&self, method: &str, params: Value, body: &[u8], timeout: Duration) -> Result<(Value, Vec<u8>), Error> {
        if !self.alive() {
            return Err(Error::Closed);
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel();
        self.pending.lock().unwrap_or_else(|e| e.into_inner()).insert(id, tx);
        {
            let mut w = self.writer.lock().unwrap_or_else(|e| e.into_inner());
            if let Err(e) = write_frame(&mut *w, &json!({ "id": id, "method": method, "params": params }), body) {
                self.pending.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
                return Err(e.into());
            }
        }
        match rx.recv_timeout(timeout) {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(e)) if !self.alive() => Err(if e == "the plugin stopped" { Error::Closed } else { Error::Plugin(e) }),
            Ok(Err(e)) => Err(Error::Plugin(e)),
            Err(_) => {
                self.pending.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
                Err(Error::Timeout(method.to_string()))
            }
        }
    }

    /// [`request`](Self::request) for a JSON result only.
    pub fn call(&self, method: &str, params: Value, timeout: Duration) -> Result<Value, Error> {
        self.request(method, params, &[], timeout).map(|(v, _)| v)
    }

    /// Stop the plugin (it is also stopped when the client is dropped).
    pub fn stop(&self) {
        if let Some(mut c) = self.child.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

impl Drop for PluginClient {
    fn drop(&mut self) {
        self.stop();
    }
}
