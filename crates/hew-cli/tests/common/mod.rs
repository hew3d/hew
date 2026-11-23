//! Shared helpers for the `catalog` specs (package S4): a hand-rolled loopback
//! HTTP server and a fixture source made of REAL Library items.
//!
//! Fixture reference: `fixture.reference()` is
//! `web:https://127.0.0.1:<port>/fixture/hew.json`. The server speaks plain
//! `http`; the specs run it with `FetchPolicy { allow_http_loopback: true }`,
//! under which a loopback `https` reference is requested over `http`.
#![allow(dead_code)]

use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

// ------------------------------------------------------------ the server

/// How one path answers.
#[derive(Clone)]
pub struct Route {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// Send `Access-Control-Allow-Origin: *`.
    pub cors: bool,
}

impl Route {
    pub fn ok(body: Vec<u8>) -> Route {
        Route {
            status: 200,
            headers: vec![],
            body,
            cors: true,
        }
    }
}

/// What the server saw.
#[derive(Clone, Debug)]
pub struct Req {
    pub path: String,
    pub origin: Option<String>,
    pub authorization: Option<String>,
    pub at: Instant,
}

pub struct Server {
    pub port: u16,
    routes: Arc<Mutex<BTreeMap<String, Route>>>,
    log: Arc<Mutex<Vec<Req>>>,
    stop: Arc<AtomicBool>,
}

impl Server {
    pub fn start(files: BTreeMap<String, Vec<u8>>) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let routes: Arc<Mutex<BTreeMap<String, Route>>> = Arc::new(Mutex::new(
            files.into_iter().map(|(p, b)| (p, Route::ok(b))).collect(),
        ));
        let log = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (r, l, s) = (routes.clone(), log.clone(), stop.clone());
        std::thread::spawn(move || {
            for conn in listener.incoming() {
                if s.load(Ordering::SeqCst) {
                    break;
                }
                if let Ok(c) = conn {
                    serve(c, &r, &l);
                }
            }
        });
        Server {
            port,
            routes,
            log,
            stop,
        }
    }

    pub fn set(&self, path: &str, route: Route) {
        self.routes.lock().unwrap().insert(path.to_string(), route);
    }
    pub fn body(&self, path: &str) -> Vec<u8> {
        self.routes.lock().unwrap()[path].body.clone()
    }
    pub fn set_body(&self, path: &str, body: Vec<u8>) {
        self.routes.lock().unwrap().get_mut(path).unwrap().body = body;
    }
    pub fn set_status(&self, path: &str, status: u16) {
        self.routes.lock().unwrap().get_mut(path).unwrap().status = status;
    }
    pub fn set_cors(&self, path: &str, cors: bool) {
        self.routes.lock().unwrap().get_mut(path).unwrap().cors = cors;
    }
    /// Answers with this `Access-Control-Allow-Origin` value instead of `*`.
    pub fn set_acao(&self, path: &str, value: &str) {
        let mut routes = self.routes.lock().unwrap();
        let r = routes.get_mut(path).unwrap();
        r.cors = false;
        r.headers
            .push(("Access-Control-Allow-Origin".into(), value.into()));
    }
    pub fn set_cors_all(&self, cors: bool) {
        for r in self.routes.lock().unwrap().values_mut() {
            r.cors = cors;
        }
    }
    /// Every path answers 429 with `Retry-After` (a rate limit).
    pub fn rate_limit_all(&self) {
        for r in self.routes.lock().unwrap().values_mut() {
            r.status = 429;
            r.headers.push(("Retry-After".into(), "60".into()));
        }
    }
    /// Every path answers 503: the source is down.
    pub fn down(&self) {
        for r in self.routes.lock().unwrap().values_mut() {
            r.status = 503;
        }
    }
    pub fn up(&self) {
        for r in self.routes.lock().unwrap().values_mut() {
            r.status = 200;
        }
    }
    pub fn requests(&self) -> Vec<Req> {
        self.log.lock().unwrap().clone()
    }
    pub fn clear_requests(&self) {
        self.log.lock().unwrap().clear();
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(("127.0.0.1", self.port));
    }
}

fn serve(mut c: TcpStream, routes: &Mutex<BTreeMap<String, Route>>, log: &Mutex<Vec<Req>>) {
    let at = Instant::now();
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
        match c.read(&mut chunk) {
            Ok(0) | Err(_) => return,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    }
    let text = String::from_utf8_lossy(&buf).to_string();
    let mut lines = text.split("\r\n");
    let path = lines
        .next()
        .and_then(|l| l.split(' ').nth(1))
        .unwrap_or("/")
        .to_string();
    let header = |name: &str| {
        text.split("\r\n").skip(1).find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.eq_ignore_ascii_case(name).then(|| v.trim().to_string())
        })
    };
    log.lock().unwrap().push(Req {
        path: path.clone(),
        origin: header("origin"),
        authorization: header("authorization"),
        at,
    });
    let route = routes.lock().unwrap().get(&path).cloned();
    let route = route.unwrap_or(Route {
        status: 404,
        headers: vec![],
        body: vec![],
        cors: true,
    });
    let reason = match route.status {
        200 => "OK",
        404 => "Not Found",
        429 => "Too Many Requests",
        _ => "Status",
    };
    let mut head = format!(
        "HTTP/1.1 {} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n",
        route.status,
        route.body.len()
    );
    if route.cors {
        head.push_str("Access-Control-Allow-Origin: *\r\n");
    }
    for (k, v) in &route.headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    let _ = c.write_all(head.as_bytes());
    let _ = c.write_all(&route.body);
}

// ------------------------------------------------------ real Library items

fn scratch_dir(tag: &str) -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "hew-cli-catalog-{tag}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

pub fn scratch(tag: &str) -> PathBuf {
    scratch_dir(tag)
}

fn run_save(dir: &Path, name: &str, category: &str, size: f64) {
    let frames = serde_json::json!([
        { "jsonrpc": "2.0", "id": 1, "method": "hew.meta.hello", "params": { "protocol": 1 } },
        { "jsonrpc": "2.0", "id": 2, "method": "hew.doc.new", "params": {} },
        { "jsonrpc": "2.0", "id": 3, "method": "hew.doc.transact", "params": { "label": name, "commands": [
            { "method": "hew.sketch.draw_rect", "as": "rect", "params": {
                "plane": { "ground": true }, "corner_a": [0.0, 0.0, 0.0], "corner_b": [size, size, 0.0] } },
            { "method": "hew.solid.extrude", "as": "box", "params": {
                "region": { "$ref": "rect#/region_id" }, "distance": size } },
            { "method": "hew.library.save", "params": {
                "selection": [{ "$ref": "box#/object_id" }], "name": name, "category": category } },
        ] } },
    ]);
    let script = dir.join(format!("{name}.json"));
    std::fs::write(&script, serde_json::to_vec(&frames).unwrap()).unwrap();
    let outcome = hew_cli::run::run_script(&script, None, None);
    assert_eq!(
        outcome.exit_code,
        0,
        "{name}: {:?}",
        outcome.responses.last()
    );
}

/// Two real Library items (a component and a model) in a folder tree, made
/// once per process. `HEW_LIBRARY_DIR` is set inside the initializer; callers
/// go through `item_tree()` first.
pub fn item_tree() -> &'static Path {
    static T: OnceLock<PathBuf> = OnceLock::new();
    T.get_or_init(|| {
        let root = scratch_dir("items");
        let lib = root.join("library");
        std::fs::create_dir_all(&lib).unwrap();
        // SAFETY: runs inside the OnceLock initializer before any fixture is
        // read; no test in these binaries touches the environment otherwise.
        unsafe { std::env::set_var("HEW_LIBRARY_DIR", &lib) };
        run_save(&root, "Hinge", "component", 1.0);
        run_save(&root, "Cabinet", "model", 2.0);
        lib
    })
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let dst = to.join(e.file_name());
        if e.path().is_dir() {
            copy_tree(&e.path(), &dst);
        } else {
            std::fs::copy(e.path(), dst).unwrap();
        }
    }
}

fn read_tree(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            read_tree(root, &p, out);
        } else {
            let rel = p
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            out.insert(rel, std::fs::read(&p).unwrap());
        }
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

// ----------------------------------------------------------- fixture source

/// A source served over loopback: `hew.json`, two items and two thumbnails
/// under `/fixture/`.
pub struct Fixture {
    pub server: Server,
    pub index: serde_json::Value,
}

impl Fixture {
    /// `license`: the SPDX id to write, or `None` for an unlicensed source.
    pub fn new(name: &str, license: Option<&str>) -> Fixture {
        let tree = item_tree();
        let dir = scratch_dir("source");
        copy_tree(tree, &dir);
        let mut args: Vec<String> = vec![
            dir.to_string_lossy().into_owned(),
            "--name".into(),
            name.into(),
        ];
        if let Some(l) = license {
            args.push("--license".into());
            args.push(l.into());
        }
        hew_cli::library_index::run(&args).expect("library index writes the fixture");
        let mut files = BTreeMap::new();
        read_tree(&dir, &dir, &mut files);
        let index: serde_json::Value = serde_json::from_slice(&files["hew.json"]).unwrap();
        let served = files
            .into_iter()
            .map(|(p, b)| (format!("/fixture/{p}"), b))
            .collect();
        Fixture {
            server: Server::start(served),
            index,
        }
    }

    /// The canonical reference of this source.
    pub fn reference(&self) -> String {
        format!(
            "web:https://127.0.0.1:{}/fixture/hew.json",
            self.server.port
        )
    }
    pub fn index_path(&self) -> String {
        "/fixture/hew.json".into()
    }
    /// Served path of item `n` (index order).
    pub fn item_path(&self, n: usize) -> String {
        format!(
            "/fixture/{}",
            self.index["items"][n]["path"].as_str().unwrap()
        )
    }
    pub fn thumb_path(&self, n: usize) -> String {
        format!(
            "/fixture/{}",
            self.index["items"][n]["thumbnail"].as_str().unwrap()
        )
    }
    /// Rewrites the served `hew.json` after `edit` changes the parsed index.
    pub fn edit_index(&self, edit: impl FnOnce(&mut serde_json::Value)) {
        let mut v = self.index.clone();
        edit(&mut v);
        self.server
            .set_body(&self.index_path(), serde_json::to_vec(&v).unwrap());
    }
}

/// A catalog checkout directory with the three list files.
pub fn checkout(library: &[String], plugins: &[String], removed: serde_json::Value) -> PathBuf {
    let dir = scratch_dir("checkout");
    std::fs::write(
        dir.join("library.json"),
        serde_json::to_vec(library).unwrap(),
    )
    .unwrap();
    std::fs::write(
        dir.join("plugins.json"),
        serde_json::to_vec(plugins).unwrap(),
    )
    .unwrap();
    std::fs::write(
        dir.join("removed.json"),
        serde_json::to_vec(&removed).unwrap(),
    )
    .unwrap();
    dir
}
