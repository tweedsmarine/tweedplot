//! The plugin's side: answer requests on standard input and output.

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use crate::frame::{read_frame, write_frame};

/// Sends events to the app while a request is being handled (progress of a long job).
#[derive(Clone)]
pub struct Events {
    out: Arc<Mutex<Box<dyn Write + Send>>>,
}

impl Events {
    pub fn send(&self, event: &str, data: Value) {
        let mut w = self.out.lock().unwrap();
        let _ = write_frame(&mut *w, &json!({ "event": event, "data": data }), &[]);
    }
}

/// What a handler returns: a JSON result and a body, or an error message.
pub type Answer = Result<(Value, Vec<u8>), String>;

/// Answer requests one at a time until the input ends: `handle(method, params, body,
/// events)`. Returns when the app closes the pipe (or stops).
pub fn serve_streams(input: impl Read, output: impl Write + Send + 'static, mut handle: impl FnMut(&str, Value, Vec<u8>, &Events) -> Answer) {
    let events = Events { out: Arc::new(Mutex::new(Box::new(std::io::BufWriter::with_capacity(1 << 16, output)))) };
    let mut r = std::io::BufReader::with_capacity(1 << 16, input);
    while let Ok(Some((h, body))) = read_frame(&mut r) {
        let Some(id) = h["id"].as_u64() else { continue };
        let method = h["method"].as_str().unwrap_or("").to_string();
        let params = h["params"].clone();
        // a panic in one request must not take the charts down: say so and carry on
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handle(&method, params, body, &events)))
            .unwrap_or_else(|_| Err(format!("the plugin failed on '{method}'")));
        let mut w = events.out.lock().unwrap();
        let _ = match r {
            Ok((result, body)) => write_frame(&mut *w, &json!({ "id": id, "ok": true, "result": result }), &body),
            Err(e) => write_frame(&mut *w, &json!({ "id": id, "ok": false, "error": e }), &[]),
        };
    }
}

/// [`serve_streams`] on this process's standard input and output.
pub fn serve(handle: impl FnMut(&str, Value, Vec<u8>, &Events) -> Answer) {
    serve_streams(std::io::stdin().lock(), std::io::stdout(), handle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PluginClient;
    use std::time::Duration;

    /// A server in a thread, a client on the other ends of two pipes.
    fn pair(handle: impl FnMut(&str, Value, Vec<u8>, &Events) -> Answer + Send + 'static) -> PluginClient {
        let (a_r, a_w) = std::io::pipe().unwrap();
        let (b_r, b_w) = std::io::pipe().unwrap();
        std::thread::spawn(move || serve_streams(a_r, b_w, handle));
        PluginClient::from_streams(b_r, a_w)
    }

    #[test]
    fn requests_replies_and_events() {
        let c = pair(|m, p, body, ev| match m {
            "echo" => Ok((p, body)),
            "job" => {
                ev.send("progress", json!({"done": 1}));
                Ok((json!("done"), vec![]))
            }
            "boom" => panic!("boom"),
            _ => Err(format!("unknown method {m}")),
        });
        let got = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let g = got.clone();
        c.on_event(move |e, d| g.lock().unwrap().push((e.to_string(), d.clone())));
        let (v, b) = c.request("echo", json!({"x": 1}), &[9, 8], Duration::from_secs(5)).unwrap();
        assert_eq!((v["x"].as_i64(), b), (Some(1), vec![9, 8]));
        assert_eq!(c.call("job", json!({}), Duration::from_secs(5)).unwrap(), json!("done"));
        assert_eq!(got.lock().unwrap().len(), 1);
        assert!(c.call("nope", json!({}), Duration::from_secs(5)).unwrap_err().to_string().contains("unknown method"));
        assert!(c.call("boom", json!({}), Duration::from_secs(5)).unwrap_err().to_string().contains("failed on 'boom'"));
        // many threads at once each get their own answer
        let c = std::sync::Arc::new(c);
        let hs: Vec<_> = (0..16)
            .map(|i| {
                let c = c.clone();
                std::thread::spawn(move || c.call("echo", json!({"i": i}), Duration::from_secs(5)).unwrap()["i"].as_i64())
            })
            .collect();
        let mut seen: Vec<i64> = hs.into_iter().map(|h| h.join().unwrap().unwrap()).collect();
        seen.sort();
        assert_eq!(seen, (0..16).collect::<Vec<_>>());
    }
}
