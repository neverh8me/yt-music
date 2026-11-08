//! Solving YouTube's stream-URL challenges (the "n" throttling parameter and the
//! signature cipher) without yt-dlp, so signed-in (18+) playback works on Android too.
//!
//! The challenge functions live in YouTube's player JavaScript. We run yt-dlp's own solver
//! (yt-dlp/ejs, Unlicense; vendored in ui/ejs) in the app webview's JS engine (V8 on
//! Android/Windows, JavaScriptCore on macOS) via a Web Worker, falling back to QuickJS
//! (rquickjs, already pulled in by rustypipe) when no webview is around (tests). Parsing the ~2.5 MB player is the slow part, so the solver's
//! "preprocessed" player (just the extracted functions) is cached on disk per player id;
//! after that each solve takes milliseconds.
use anyhow::{Context, Result, anyhow, bail};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

const LIB: &str = include_str!("../../ui/ejs/yt.solver.lib.min.js");
const CORE: &str = include_str!("../../ui/ejs/yt.solver.core.min.js");

// ------------------------------------------------------------------ webview runner
static APP: std::sync::OnceLock<tauri::AppHandle> = std::sync::OnceLock::new();
static PENDING: std::sync::Mutex<Option<HashMap<u64, tokio::sync::oneshot::Sender<String>>>> = std::sync::Mutex::new(None);
static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Let the solver use the app's webview (ui/solver-worker.js). Call once at startup.
pub fn attach(app: tauri::AppHandle) {
    let _ = APP.set(app);
}

/// The webview worker's answer (see app.js).
#[tauri::command]
pub fn js_solve_done(id: u64, out: String) {
    let tx = PENDING.lock().unwrap().as_mut().and_then(|m| m.remove(&id));
    if let Some(tx) = tx {
        let _ = tx.send(out);
    }
}

async fn run_webview(app: &tauri::AppHandle, input: &serde_json::Value) -> Result<serde_json::Value> {
    use tauri::Emitter;
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let (tx, rx) = tokio::sync::oneshot::channel();
    PENDING.lock().unwrap().get_or_insert_with(HashMap::new).insert(id, tx);
    app.emit("js-solve", serde_json::json!({ "id": id, "input": input.to_string() }))?;
    let out = match tokio::time::timeout(Duration::from_secs(90), rx).await {
        Ok(Ok(out)) => out,
        _ => {
            PENDING.lock().unwrap().as_mut().map(|m| m.remove(&id));
            bail!("webview solver did not answer");
        }
    };
    let v: serde_json::Value = serde_json::from_str(&out)?;
    if v["type"] == "error" {
        bail!("solver: {}", v["error"].as_str().unwrap_or("error"));
    }
    Ok(v)
}

#[derive(Clone)]
pub struct Player {
    pub id: String,
    /// Signature timestamp: tells the player API which cipher version we will solve with.
    pub sts: u32,
    preprocessed: String,
    fetched: Instant,
}

pub struct Solver {
    http: reqwest::Client,
    dir: PathBuf,
    current: tokio::sync::Mutex<Option<Player>>,
}

impl Solver {
    pub fn new(http: reqwest::Client, dir: PathBuf) -> Self {
        Self { http, dir, current: tokio::sync::Mutex::new(None) }
    }

    /// The current player (id, sts, preprocessed solver input), refreshed every few hours.
    pub async fn player(&self) -> Result<Player> {
        let mut cur = self.current.lock().await;
        if let Some(p) = cur.as_ref()
            && p.fetched.elapsed() < Duration::from_secs(4 * 3600)
        {
            return Ok(p.clone());
        }
        let id = self.player_id().await?;
        if let Some(p) = cur.as_ref().filter(|p| p.id == id) {
            let p = Player { fetched: Instant::now(), ..p.clone() };
            *cur = Some(p.clone());
            return Ok(p);
        }
        let p = match self.load_cached(&id) {
            Some(p) => p,
            None => self.prepare(&id).await?,
        };
        *cur = Some(p.clone());
        Ok(p)
    }

    async fn player_id(&self) -> Result<String> {
        let page = self.http.get("https://www.youtube.com/iframe_api").send().await?.error_for_status()?.text().await?;
        let page = page.replace("\\/", "/");
        let at = page.find("/s/player/").ok_or_else(|| anyhow!("player id not found"))? + "/s/player/".len();
        let id: String = page[at..].chars().take_while(|c| c.is_ascii_hexdigit()).collect();
        if id.len() < 8 {
            bail!("bad player id");
        }
        Ok(id)
    }

    fn cache_paths(&self, id: &str) -> (PathBuf, PathBuf) {
        (self.dir.join(format!("{id}.pre.js")), self.dir.join(format!("{id}.sts")))
    }

    fn load_cached(&self, id: &str) -> Option<Player> {
        let (pre, sts) = self.cache_paths(id);
        let preprocessed = std::fs::read_to_string(pre).ok()?;
        let sts = std::fs::read_to_string(sts).ok()?.trim().parse().ok()?;
        Some(Player { id: id.into(), sts, preprocessed, fetched: Instant::now() })
    }

    /// Download the player JS, read its signature timestamp and preprocess it once.
    async fn prepare(&self, id: &str) -> Result<Player> {
        let url = format!("https://www.youtube.com/s/player/{id}/player_ias.vflset/en_US/base.js");
        let js = self.http.get(&url).send().await?.error_for_status()?.text().await?;
        let sts = signature_timestamp(&js).ok_or_else(|| anyhow!("signatureTimestamp not found in player {id}"))?;
        let input = serde_json::json!({ "type": "player", "player": js, "requests": [], "output_preprocessed": true });
        let out = run(input).await.context("preprocessing the player")?;
        let preprocessed = out["preprocessed_player"].as_str().ok_or_else(|| anyhow!("no preprocessed player"))?.to_string();
        std::fs::create_dir_all(&self.dir).ok();
        let (pre_path, sts_path) = self.cache_paths(id);
        let _ = std::fs::write(pre_path, &preprocessed);
        let _ = std::fs::write(sts_path, sts.to_string());
        // old players are useless once YouTube moves on
        if let Ok(rd) = std::fs::read_dir(&self.dir) {
            for e in rd.flatten() {
                if !e.file_name().to_string_lossy().starts_with(id) {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
        Ok(Player { id: id.into(), sts, preprocessed, fetched: Instant::now() })
    }

    /// Solve "n" and "sig" challenges with the given player. Returns challenge → answer maps.
    pub async fn solve(&self, p: &Player, n: &[String], sig: &[String]) -> Result<(HashMap<String, String>, HashMap<String, String>)> {
        let mut requests = vec![];
        if !n.is_empty() {
            requests.push(serde_json::json!({ "type": "n", "challenges": n }));
        }
        if !sig.is_empty() {
            requests.push(serde_json::json!({ "type": "sig", "challenges": sig }));
        }
        let input = serde_json::json!({ "type": "preprocessed", "preprocessed_player": p.preprocessed, "requests": requests });
        let out = run(input).await?;
        let mut maps = vec![];
        for r in out["responses"].as_array().ok_or_else(|| anyhow!("solver: no responses"))? {
            if r["type"] == "error" {
                bail!("solver: {}", r["error"].as_str().unwrap_or("error"));
            }
            let m: HashMap<String, String> = serde_json::from_value(r["data"].clone())?;
            maps.push(m);
        }
        let mut it = maps.into_iter();
        let nm = if n.is_empty() { HashMap::new() } else { it.next().unwrap_or_default() };
        let sm = if sig.is_empty() { HashMap::new() } else { it.next().unwrap_or_default() };
        Ok((nm, sm))
    }
}

fn signature_timestamp(js: &str) -> Option<u32> {
    for key in ["signatureTimestamp:", "signatureTimestamp :", "sts:"] {
        if let Some(i) = js.find(key) {
            let digits: String = js[i + key.len()..].trim_start().chars().take_while(|c| c.is_ascii_digit()).collect();
            if digits.len() == 5
                && let Ok(v) = digits.parse()
            {
                return Some(v);
            }
        }
    }
    None
}

/// Run the solver on its own thread: QuickJS recursion needs a big native stack, and the
/// first (parsing) run takes a while — it must not block the async runtime.
async fn run(input: serde_json::Value) -> Result<serde_json::Value> {
    if let Some(app) = APP.get() {
        match run_webview(app, &input).await {
            Ok(v) => return Ok(v),
            Err(e) => eprintln!("webview solver failed ({e:#}); using QuickJS"),
        }
    }
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("js-solver".into())
        .stack_size(256 * 1024 * 1024)
        .spawn(move || {
            let _ = tx.send(run_blocking(&input));
        })?;
    rx.await.map_err(|_| anyhow!("solver thread died"))?
}

fn run_blocking(input: &serde_json::Value) -> Result<serde_json::Value> {
    let rt = rquickjs::Runtime::new()?;
    rt.set_max_stack_size(200 * 1024 * 1024);
    rt.set_memory_limit(1024 * 1024 * 1024);
    let ctx = rquickjs::Context::full(&rt)?;
    let script = format!(
        "{LIB}\n;Object.assign(globalThis, lib);\n{CORE}\n;(function(){{ try {{ return JSON.stringify(jsc({input})); }} catch (e) {{ return JSON.stringify({{ type: 'error', error: String(e) + (e && e.stack ? '\\n' + e.stack : '') }}); }} }})()",
        input = serde_json::to_string(input)?,
    );
    let out: String = ctx.with(|ctx| ctx.eval::<String, _>(script)).map_err(|e| anyhow!("quickjs: {e}"))?;
    let v: serde_json::Value = serde_json::from_str(&out)?;
    if v["type"] == "error" {
        bail!("solver: {}", v["error"].as_str().unwrap_or("error"));
    }
    Ok(v)
}
