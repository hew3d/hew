//! The developer-folder watcher: a poller over the folders of enabled
//! folder plugins, run only while developer mode is on.
//!
//! It polls with the standard library alone, so it behaves the same on
//! every desktop and needs no file-notification crate. Every
//! [`Timing::poll`] it lists each watched folder through
//! [`plugin_host::store::folder_snapshot_until`] (the recorded root checked
//! again, no link followed, the package caps as walk bounds; names and
//! metadata only, never contents) and compares the listing with the last
//! one. A folder's first listing is reported at once as
//! [`WatchEvent::Entered`], so the handler compares its manifest with the
//! stored one before anything else happens: an edit made while it was not
//! watched is not taken as the starting point. A later change waits for
//! [`Timing::quiet`] with no further change, so a burst of saves gives one
//! event; then the handler runs with [`WatchEvent::Changed`]. A folder that
//! cannot be listed (gone, a link put in its place, over the caps) is
//! reported once as [`WatchEvent::Problem`] until it can be listed again.
//!
//! One thread serves every watched folder and decides; each walk runs on a
//! short-lived thread of its own and is waited for at most
//! [`Timing::budget`]. A folder whose walk takes longer (a network mount
//! that stopped answering) is reported once as `folder_missing` and is not
//! walked again until that walk returns, so it holds up the others for one
//! budget at most. Every walk checks the stop flag between entries, and
//! [`DevWatch::stop`] waits for the thread at most [`Timing::join`]: past
//! that it is let go, and what it finds later is never reported, since it
//! checks the stop flag before every report. Turning developer mode off and
//! quitting are therefore never held by a folder. The thread holds no lock
//! while it walks, and none while the handler runs; the handler (the
//! shell's) takes the store lock itself, briefly.

use plugin_host::store::FolderSnapshot;
use plugin_host::InstallError;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// How often the folders are listed, how long a change must be followed
/// by no other before it is reported, how long one folder's walk is waited
/// for in a pass, and how long stopping waits for the thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timing {
    pub poll: Duration,
    pub quiet: Duration,
    pub budget: Duration,
    pub join: Duration,
}

/// The app's timing.
pub const TIMING: Timing = Timing {
    poll: Duration::from_millis(500),
    quiet: Duration::from_millis(300),
    budget: Duration::from_secs(1),
    join: Duration::from_secs(2),
};

/// What the watcher reports about one plugin's folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchEvent {
    /// The folder was listed for the first time since it joined the
    /// watched set: compare its manifest with the stored one now.
    Entered { id: String },
    /// Something in the folder changed, and has been quiet since.
    Changed { id: String },
    /// The folder cannot be listed; `refusal` names why
    /// (`folder_missing`, which a walk past the budget is too,
    /// `package_path_invalid`, `package_too_large`).
    Problem { id: String, refusal: &'static str },
}

/// What the handler wants next for the plugin an event was about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Watching {
    /// Keep watching it.
    Keep,
    /// Stop watching it until the watched set is next replaced.
    Drop,
    /// Keep watching it, and send [`WatchEvent::Entered`] again on the next
    /// pass whose listing succeeds: the comparison did not complete.
    Again,
}

impl From<bool> for Watching {
    fn from(keep: bool) -> Watching {
        if keep {
            Watching::Keep
        } else {
            Watching::Drop
        }
    }
}

/// Runs for each event, on the watcher's thread, and says what next.
pub type Handler = Arc<dyn Fn(WatchEvent) -> Watching + Send + Sync>;

/// Lists one folder, asking the callback before each entry whether to go
/// on (`None` when told to stop). The app's is
/// [`plugin_host::store::folder_snapshot_until`]; tests stand in for it.
pub type Lister = Arc<
    dyn Fn(&Path, &dyn Fn() -> bool) -> Result<Option<FolderSnapshot>, InstallError> + Send + Sync,
>;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// What the commands and the thread share.
struct Shared {
    state: Mutex<SharedState>,
    wake: Condvar,
    /// The stop flag the walks read without the lock.
    stopping: Arc<AtomicBool>,
    /// Set by the thread as it ends, so `stop` can wait with a limit.
    done: Mutex<bool>,
    /// The problem each folder has now, by id, for a reader that must not
    /// wait for a walk.
    problems: Mutex<BTreeMap<String, &'static str>>,
    ended: Condvar,
}

struct SharedState {
    stop: bool,
    /// Plugin id to its recorded folder.
    targets: BTreeMap<String, PathBuf>,
    /// Bumped whenever `targets` is replaced.
    generation: u64,
    /// Plugins whose entry is to be reported again on the next good
    /// listing ([`DevWatch::recheck`]).
    recheck: BTreeSet<String>,
}

struct Running {
    shared: Arc<Shared>,
    thread: std::thread::JoinHandle<()>,
}

/// The watcher: no thread while nothing is watched.
pub struct DevWatch {
    timing: Timing,
    lister: Lister,
    handler: Mutex<Option<Handler>>,
    running: Mutex<Option<Running>>,
    /// Set at app exit: nothing starts again.
    closed: AtomicBool,
}

impl DevWatch {
    pub fn new(timing: Timing) -> DevWatch {
        DevWatch::with_lister(
            timing,
            Arc::new(|root: &Path, keep_going: &dyn Fn() -> bool| {
                plugin_host::store::folder_snapshot_until(root, keep_going)
            }),
        )
    }

    /// A watcher that lists folders with `lister`.
    pub fn with_lister(timing: Timing, lister: Lister) -> DevWatch {
        DevWatch {
            timing,
            lister,
            handler: Mutex::new(None),
            running: Mutex::new(None),
            closed: AtomicBool::new(false),
        }
    }

    /// Sets what runs for each event. Without one, nothing is ever
    /// watched.
    pub fn set_handler(&self, handler: Handler) {
        *lock(&self.handler) = Some(handler);
    }

    /// Replaces the watched set. An empty set stops the thread (waiting
    /// for it at most [`Timing::join`]); a non-empty one starts it if it is
    /// not running. A plugin newly in the set is reported as
    /// [`WatchEvent::Entered`] after its first listing.
    pub fn watch(&self, targets: BTreeMap<String, PathBuf>) {
        if targets.is_empty() || self.closed.load(Ordering::Acquire) {
            self.stop();
            return;
        }
        let Some(handler) = lock(&self.handler).clone() else {
            return;
        };
        let mut running = lock(&self.running);
        if let Some(r) = running.as_ref().filter(|r| !r.thread.is_finished()) {
            let mut state = lock(&r.shared.state);
            if state.targets != targets {
                state.targets = targets;
                state.generation += 1;
                r.shared.wake.notify_all();
            }
            return;
        }
        let shared = Arc::new(Shared {
            state: Mutex::new(SharedState {
                stop: false,
                targets,
                generation: 1,
                recheck: BTreeSet::new(),
            }),
            wake: Condvar::new(),
            stopping: Arc::new(AtomicBool::new(false)),
            done: Mutex::new(false),
            ended: Condvar::new(),
            problems: Mutex::new(BTreeMap::new()),
        });
        let timing = self.timing;
        let lister = self.lister.clone();
        let for_thread = shared.clone();
        let spawned = std::thread::Builder::new()
            .name("hew-plugin-dev-watch".to_string())
            .spawn(move || {
                run(&for_thread, timing, &lister, &handler);
                *lock(&for_thread.done) = true;
                for_thread.ended.notify_all();
            });
        match spawned {
            Ok(thread) => *running = Some(Running { shared, thread }),
            Err(e) => eprintln!("hew: the plugin folder watcher could not start: {e}"),
        }
    }

    /// Asks for [`WatchEvent::Entered`] again for each of `ids` on its next
    /// good listing, so a comparison that did not complete is tried again
    /// every pass until one does. Ids not watched are ignored.
    pub fn recheck(&self, ids: &[String]) {
        if ids.is_empty() {
            return;
        }
        if let Some(r) = lock(&self.running).as_ref() {
            lock(&r.shared.state).recheck.extend(ids.iter().cloned());
            r.shared.wake.notify_all();
        }
    }

    /// Stops the thread and waits for it, at most [`Timing::join`]; a
    /// thread that does not finish by then is let go, and reports nothing
    /// more. Called on the watcher's own thread (it never is by this
    /// module), it only asks it to stop.
    pub fn stop(&self) {
        let Some(running) = lock(&self.running).take() else {
            return;
        };
        running.shared.stopping.store(true, Ordering::Release);
        lock(&running.shared.state).stop = true;
        running.shared.wake.notify_all();
        if running.thread.thread().id() == std::thread::current().id() {
            return;
        }
        let done = lock(&running.shared.done);
        let (done, _) = running
            .shared
            .ended
            .wait_timeout_while(done, self.timing.join, |d| !*d)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if *done {
            drop(done);
            let _ = running.thread.join();
        } else {
            eprintln!("hew: the plugin folder watcher did not stop in time and was left to finish");
        }
    }

    /// Stops the thread for good: used at app exit.
    pub fn shutdown(&self) {
        self.closed.store(true, Ordering::Release);
        self.stop();
    }

    /// The problem each watched folder has now (`folder_missing`,
    /// `package_path_invalid`, `package_too_large`), by plugin id; none
    /// while nothing is watched. Never waits for a walk.
    pub fn problems(&self) -> BTreeMap<String, &'static str> {
        lock(&self.running)
            .as_ref()
            .filter(|r| !r.thread.is_finished())
            .map(|r| lock(&r.shared.problems).clone())
            .unwrap_or_default()
    }

    /// The ids being watched, sorted; none while the thread is not running.
    #[cfg(test)]
    pub fn watching(&self) -> Vec<String> {
        lock(&self.running)
            .as_ref()
            .filter(|r| !r.thread.is_finished())
            .map(|r| lock(&r.shared.state).targets.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// Whether the thread is running.
    #[cfg(test)]
    pub fn is_running(&self) -> bool {
        lock(&self.running)
            .as_ref()
            .is_some_and(|r| !r.thread.is_finished())
    }
}

impl Drop for DevWatch {
    fn drop(&mut self) {
        self.shutdown();
    }
}

type Walk = Result<Option<FolderSnapshot>, InstallError>;

/// One watched folder, as the thread last saw it.
struct Watched {
    root: PathBuf,
    /// The last listing; `None` before the first one and while it fails.
    seen: Option<FolderSnapshot>,
    /// Whether the first listing has been taken (and reported).
    started: bool,
    /// When the last change was seen, while one waits to be reported.
    changed_at: Option<Instant>,
    /// The problem last reported, until the folder lists again.
    problem: Option<&'static str>,
    /// A walk that ran past the budget and has not returned yet.
    walking: Option<mpsc::Receiver<Walk>>,
    /// The entry is to be reported again on the next good listing.
    again: bool,
}

/// Lists `root` on a thread of its own, waiting at most `budget`. `Err`
/// with the receiver when the walk is still going.
fn walk(
    root: &Path,
    lister: &Lister,
    stopping: &Arc<AtomicBool>,
    budget: Duration,
) -> Result<Walk, mpsc::Receiver<Walk>> {
    let (tx, rx) = mpsc::channel();
    let root = root.to_path_buf();
    let lister = lister.clone();
    let stopping = stopping.clone();
    let spawned = std::thread::Builder::new()
        .name("hew-plugin-dev-walk".to_string())
        .spawn(move || {
            let keep_going = || !stopping.load(Ordering::Acquire);
            let _ = tx.send(lister(&root, &keep_going));
        });
    if spawned.is_err() {
        // No thread to spare: try again next pass.
        return Ok(Ok(None));
    }
    match rx.recv_timeout(budget) {
        Ok(result) => Ok(result),
        Err(mpsc::RecvTimeoutError::Timeout) => Err(rx),
        Err(mpsc::RecvTimeoutError::Disconnected) => Ok(Ok(None)),
    }
}

fn run(shared: &Shared, timing: Timing, lister: &Lister, handler: &Handler) {
    let mut watched: BTreeMap<String, Watched> = BTreeMap::new();
    let mut dropped: BTreeSet<String> = BTreeSet::new();
    let mut generation = 0u64;
    let stopped = || shared.stopping.load(Ordering::Acquire);
    loop {
        let (targets, recheck) = {
            let mut state = lock(&shared.state);
            if state.stop {
                return;
            }
            let recheck = std::mem::take(&mut state.recheck);
            if state.generation != generation {
                generation = state.generation;
                dropped.clear();
            }
            (state.targets.clone(), recheck)
        };
        watched.retain(|id, w| targets.get(id) == Some(&w.root) && !dropped.contains(id));
        lock(&shared.problems).retain(|id, _| watched.contains_key(id));
        for (id, root) in &targets {
            if dropped.contains(id) {
                continue;
            }
            watched.entry(id.clone()).or_insert_with(|| Watched {
                root: root.clone(),
                seen: None,
                started: false,
                changed_at: None,
                problem: None,
                walking: None,
                again: false,
            });
        }
        for id in recheck {
            if let Some(w) = watched.get_mut(&id) {
                w.again = true;
            }
        }
        let ids: Vec<String> = watched.keys().cloned().collect();
        for id in ids {
            if stopped() {
                return;
            }
            let Some(w) = watched.get_mut(&id) else {
                continue;
            };
            // A walk still out from an earlier pass: take its answer if it
            // has one, and do not start another.
            let listed = match w.walking.take() {
                Some(rx) => match rx.try_recv() {
                    Ok(result) => Some(result),
                    Err(mpsc::TryRecvError::Empty) => {
                        w.walking = Some(rx);
                        None
                    }
                    Err(mpsc::TryRecvError::Disconnected) => None,
                },
                None => match walk(&w.root, lister, &shared.stopping, timing.budget) {
                    Ok(result) => Some(result),
                    Err(rx) => {
                        w.walking = Some(rx);
                        None
                    }
                },
            };
            if stopped() {
                return;
            }
            let now = Instant::now();
            let mut report = None;
            match listed {
                // Still out past the budget: once, as a folder that cannot
                // be reached.
                None if w.walking.is_some() => {
                    w.seen = None;
                    w.changed_at = None;
                    if w.problem != Some("folder_missing") {
                        w.problem = Some("folder_missing");
                        report = Some(WatchEvent::Problem {
                            id: id.clone(),
                            refusal: "folder_missing",
                        });
                    }
                }
                None | Some(Ok(None)) => {}
                Some(Ok(Some(listing))) => {
                    w.problem = None;
                    if !w.started || w.again {
                        w.started = true;
                        w.again = false;
                        w.changed_at = None;
                        w.seen = Some(listing);
                        report = Some(WatchEvent::Entered { id: id.clone() });
                    } else if w.seen.as_ref() != Some(&listing) {
                        w.changed_at = Some(now);
                        w.seen = Some(listing);
                    }
                }
                Some(Err(e)) => {
                    w.seen = None;
                    w.changed_at = None;
                    // Its first listing failed: what comes back is a
                    // change, compared then.
                    w.started = true;
                    if w.problem != Some(e.name()) {
                        w.problem = Some(e.name());
                        report = Some(WatchEvent::Problem {
                            id: id.clone(),
                            refusal: e.name(),
                        });
                    }
                }
            }
            {
                let mut problems = lock(&shared.problems);
                match w.problem {
                    Some(p) => problems.insert(id.clone(), p),
                    None => problems.remove(&id),
                };
            }
            if report.is_none()
                && w.changed_at
                    .is_some_and(|at| now.duration_since(at) >= timing.quiet)
            {
                w.changed_at = None;
                report = Some(WatchEvent::Changed { id: id.clone() });
            }
            if let Some(event) = report {
                if stopped() {
                    return;
                }
                match handler(event) {
                    Watching::Keep => {}
                    Watching::Again => {
                        if let Some(w) = watched.get_mut(&id) {
                            w.again = true;
                        }
                    }
                    Watching::Drop => {
                        dropped.insert(id.clone());
                        watched.remove(&id);
                        lock(&shared.problems).remove(&id);
                    }
                }
            }
        }
        // While a change waits to be reported, look again sooner, so the
        // report comes about `quiet` after the last change.
        let wait = if watched.values().any(|w| w.changed_at.is_some()) {
            timing.quiet.min(timing.poll)
        } else {
            timing.poll
        };
        let state = lock(&shared.state);
        if state.stop {
            return;
        }
        let _ = shared
            .wake
            .wait_timeout_while(state, wait, |s| !s.stop && s.generation == generation);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    /// A folder removed when the test ends; canonical, as a pick is.
    struct Temp(PathBuf);

    impl Temp {
        fn new(name: &str) -> Temp {
            use std::sync::atomic::{AtomicUsize, Ordering};
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "hew-dev-watch-{}-{name}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("temp folder");
            Temp(std::fs::canonicalize(&path).expect("canonical"))
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    const FAST: Timing = Timing {
        poll: Duration::from_millis(20),
        quiet: Duration::from_millis(120),
        budget: Duration::from_millis(200),
        join: Duration::from_millis(400),
    };

    /// A watcher whose handler sends every event; it keeps watching after
    /// the entry event, and after the others when `keep`.
    fn watcher(keep: bool) -> (DevWatch, mpsc::Receiver<WatchEvent>) {
        watcher_with(DevWatch::new(FAST), keep)
    }

    fn watcher_with(w: DevWatch, keep: bool) -> (DevWatch, mpsc::Receiver<WatchEvent>) {
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        w.set_handler(Arc::new(move |e| {
            let entered = matches!(e, WatchEvent::Entered { .. });
            let _ = lock(&tx).send(e);
            Watching::from(keep || entered)
        }));
        (w, rx)
    }

    /// The first event of a folder joining the set is its entry.
    fn entered(rx: &mpsc::Receiver<WatchEvent>, id: &str) {
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(5)).expect("the entry"),
            WatchEvent::Entered { id: id.into() }
        );
    }

    fn one(id: &str, root: &std::path::Path) -> BTreeMap<String, PathBuf> {
        BTreeMap::from([(id.to_string(), root.to_path_buf())])
    }

    fn plugin_folder(temp: &Temp) -> PathBuf {
        let src = temp.0.join("src");
        std::fs::create_dir_all(src.join("lib")).unwrap();
        std::fs::write(src.join("hew-plugin.json"), b"{}").unwrap();
        std::fs::write(src.join("lib/main.js"), b"//").unwrap();
        src
    }

    /// Lets the watcher take its first listing.
    fn settle() {
        std::thread::sleep(FAST.poll * 4);
    }

    #[test]
    fn a_burst_of_saves_is_one_event_after_the_quiet() {
        let temp = Temp::new("burst");
        let src = plugin_folder(&temp);
        let (w, rx) = watcher(true);
        w.watch(one("com.example.a", &src));
        entered(&rx, "com.example.a");
        settle();
        assert!(rx.try_recv().is_err(), "the first listing is reported once");
        let mut last = Instant::now();
        for n in 0..8 {
            // Each save changes the size, so it shows whatever the clock's
            // resolution. Saves come faster than the quiet, so none is
            // reported during the burst (a report there would be followed
            // by a second one, which the check below refuses).
            // The quiet is measured from the save itself, not from the
            // pause after it: the watcher can see the last save at once.
            last = Instant::now();
            std::fs::write(src.join("lib/main.js"), "/".repeat(n + 3)).unwrap();
            std::thread::sleep(Duration::from_millis(30));
        }
        let event = rx.recv_timeout(Duration::from_secs(5)).expect("an event");
        assert_eq!(
            event,
            WatchEvent::Changed {
                id: "com.example.a".into()
            }
        );
        assert!(
            last.elapsed() + FAST.poll >= FAST.quiet,
            "reported about the quiet after the last save"
        );
        assert!(
            rx.recv_timeout(FAST.quiet * 4).is_err(),
            "one event for the burst"
        );
        // A later save is a new event.
        std::fs::write(src.join("new.js"), b"x").unwrap();
        assert!(rx.recv_timeout(Duration::from_secs(5)).is_ok());
        w.stop();
    }

    #[test]
    fn a_folder_that_cannot_be_listed_is_reported_once() {
        let temp = Temp::new("problem");
        let src = plugin_folder(&temp);
        let (w, rx) = watcher(true);
        w.watch(one("com.example.a", &src));
        entered(&rx, "com.example.a");
        settle();
        std::fs::rename(&src, temp.0.join("moved")).unwrap();
        let event = rx.recv_timeout(Duration::from_secs(5)).expect("a problem");
        assert_eq!(
            event,
            WatchEvent::Problem {
                id: "com.example.a".into(),
                refusal: "folder_missing"
            }
        );
        assert!(rx.recv_timeout(FAST.poll * 8).is_err(), "reported once");
        // Back again: a change, so the host reloads.
        std::fs::rename(temp.0.join("moved"), &src).unwrap();
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(5)).expect("a change"),
            WatchEvent::Changed {
                id: "com.example.a".into()
            }
        );
        // Over the caps: reported once, and the walk stops there.
        let big = std::fs::File::create(src.join("big.bin")).unwrap();
        big.set_len(plugin_host::MAX_ENTRY_BYTES as u64 + 1)
            .unwrap();
        drop(big);
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(5)).expect("too large"),
            WatchEvent::Problem {
                id: "com.example.a".into(),
                refusal: "package_too_large"
            }
        );
        assert!(rx.recv_timeout(FAST.poll * 8).is_err(), "reported once");
        w.stop();
    }

    #[cfg(unix)]
    #[test]
    fn a_change_behind_a_link_is_not_seen() {
        let temp = Temp::new("link");
        let src = plugin_folder(&temp);
        let outside = temp.0.join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, src.join("out")).unwrap();
        let (w, rx) = watcher(true);
        w.watch(one("com.example.a", &src));
        entered(&rx, "com.example.a");
        settle();
        std::fs::write(outside.join("x.js"), b"changed").unwrap();
        assert!(
            rx.recv_timeout(FAST.quiet * 4).is_err(),
            "the link is not followed"
        );
        w.stop();
    }

    #[test]
    fn the_thread_starts_with_a_folder_and_stops_and_joins_without_one() {
        let temp = Temp::new("life");
        let src = plugin_folder(&temp);
        let (w, rx) = watcher(true);
        assert!(!w.is_running());
        w.watch(BTreeMap::new());
        assert!(!w.is_running(), "nothing to watch, no thread");
        w.watch(one("com.example.a", &src));
        assert!(w.is_running());
        assert_eq!(w.watching(), ["com.example.a"]);
        // A second plugin joins the same thread.
        let other = temp.0.join("other");
        std::fs::create_dir_all(&other).unwrap();
        let mut both = one("com.example.a", &src);
        both.insert("com.example.b".into(), other.clone());
        w.watch(both);
        assert_eq!(w.watching(), ["com.example.a", "com.example.b"]);
        // Emptied: the thread is stopped and joined before this returns.
        w.watch(BTreeMap::new());
        assert!(!w.is_running());
        assert!(w.watching().is_empty());
        while rx.try_recv().is_ok() {}
        std::fs::write(src.join("lib/main.js"), b"// changed").unwrap();
        assert!(
            rx.recv_timeout(FAST.quiet * 4).is_err(),
            "nothing after stop"
        );
        // It starts again, and shutdown stops it for good.
        w.watch(one("com.example.a", &src));
        assert!(w.is_running());
        w.shutdown();
        assert!(!w.is_running());
        w.watch(one("com.example.a", &src));
        assert!(!w.is_running(), "nothing starts after shutdown");
    }

    #[test]
    fn a_handler_that_lets_go_is_not_called_again_until_the_set_changes() {
        let temp = Temp::new("drop");
        let src = plugin_folder(&temp);
        let (w, rx) = watcher(false);
        w.watch(one("com.example.a", &src));
        entered(&rx, "com.example.a");
        settle();
        std::fs::write(src.join("lib/main.js"), b"// one").unwrap();
        assert!(rx.recv_timeout(Duration::from_secs(5)).is_ok());
        std::fs::write(src.join("lib/main.js"), b"// two, longer").unwrap();
        assert!(rx.recv_timeout(FAST.quiet * 4).is_err(), "let go");
        w.stop();
    }

    #[test]
    fn without_a_handler_nothing_is_watched() {
        let temp = Temp::new("nohandler");
        let src = plugin_folder(&temp);
        let w = DevWatch::new(FAST);
        w.watch(one("com.example.a", &src));
        assert!(!w.is_running());
    }

    /// A lister that never answers for folders named `stuck` until the
    /// gate is dropped, and lists every other folder as the app does.
    fn stalling(gate: Arc<Mutex<mpsc::Receiver<()>>>) -> DevWatch {
        DevWatch::with_lister(
            FAST,
            Arc::new(
                move |root: &std::path::Path, keep_going: &dyn Fn() -> bool| {
                    if root.ends_with("stuck") {
                        let _ = lock(&gate).recv();
                        return Ok(None);
                    }
                    plugin_host::store::folder_snapshot_until(root, keep_going)
                },
            ),
        )
    }

    #[test]
    fn a_folder_that_stops_answering_holds_up_the_others_for_one_budget_at_most() {
        let temp = Temp::new("stall");
        let src = plugin_folder(&temp);
        let stuck = temp.0.join("stuck");
        std::fs::create_dir_all(&stuck).unwrap();
        let (release, gate) = mpsc::channel::<()>();
        let (w, rx) = watcher_with(stalling(Arc::new(Mutex::new(gate))), true);
        let mut both = one("com.example.a", &src);
        both.insert("com.example.stuck".into(), stuck);
        w.watch(both);
        let mut seen = Vec::new();
        let start = Instant::now();
        while seen.len() < 2 && start.elapsed() < Duration::from_secs(5) {
            if let Ok(e) = rx.recv_timeout(Duration::from_millis(50)) {
                seen.push(e);
            }
        }
        assert!(seen.contains(&WatchEvent::Entered {
            id: "com.example.a".into()
        }));
        assert!(seen.contains(&WatchEvent::Problem {
            id: "com.example.stuck".into(),
            refusal: "folder_missing"
        }));
        // The other folder is still watched while the walk is out.
        std::fs::write(src.join("lib/main.js"), b"// a longer save").unwrap();
        let changed = rx.recv_timeout(Duration::from_secs(5)).expect("a change");
        assert_eq!(
            changed,
            WatchEvent::Changed {
                id: "com.example.a".into()
            }
        );
        assert!(
            rx.recv_timeout(FAST.budget * 3).is_err(),
            "the stuck folder is reported once"
        );
        // Stopping does not wait for the stuck walk.
        let stop = Instant::now();
        w.stop();
        assert!(stop.elapsed() < FAST.join + Duration::from_millis(500));
        drop(release);
    }

    #[test]
    fn stopping_waits_for_the_thread_at_most_the_join_limit() {
        let temp = Temp::new("join");
        let src = plugin_folder(&temp);
        let (release, gate) = mpsc::channel::<()>();
        let gate = Mutex::new(gate);
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let w = DevWatch::new(FAST);
        // A handler that does not return until released.
        w.set_handler(Arc::new(move |e| {
            let _ = lock(&tx).send(e);
            let _ = lock(&gate).recv();
            Watching::Keep
        }));
        w.watch(one("com.example.a", &src));
        entered(&rx, "com.example.a");
        let stop = Instant::now();
        w.stop();
        let took = stop.elapsed();
        assert!(took >= FAST.join, "waited for the thread: {took:?}");
        assert!(took < FAST.join + Duration::from_millis(500), "{took:?}");
        assert!(!w.is_running());
        // Let go: once released it reports nothing more.
        std::fs::write(src.join("lib/main.js"), b"// changed after stop").unwrap();
        drop(release);
        assert!(rx.recv_timeout(FAST.quiet * 4).is_err());
    }

    #[test]
    fn a_walk_stops_between_entries_when_told() {
        let temp = Temp::new("between");
        let src = plugin_folder(&temp);
        let asked = std::cell::Cell::new(0);
        let listed = plugin_host::store::folder_snapshot_until(&src, &|| {
            asked.set(asked.get() + 1);
            asked.get() < 2
        })
        .unwrap();
        assert_eq!(listed, None);
        assert_eq!(asked.get(), 2);
    }

    #[test]
    fn the_current_problem_is_there_to_read() {
        let temp = Temp::new("problems");
        let src = plugin_folder(&temp);
        let (w, rx) = watcher(true);
        w.watch(one("com.example.a", &src));
        entered(&rx, "com.example.a");
        assert!(w.problems().is_empty());
        std::fs::rename(&src, temp.0.join("moved")).unwrap();
        rx.recv_timeout(Duration::from_secs(5)).expect("a problem");
        assert_eq!(
            w.problems(),
            BTreeMap::from([("com.example.a".to_string(), "folder_missing")])
        );
        std::fs::rename(temp.0.join("moved"), &src).unwrap();
        rx.recv_timeout(Duration::from_secs(5)).expect("back");
        assert!(w.problems().is_empty());
        w.stop();
        assert!(w.problems().is_empty(), "nothing while not watching");
    }

    #[test]
    fn an_entry_whose_comparison_did_not_complete_is_tried_again_each_pass() {
        let temp = Temp::new("again");
        let src = plugin_folder(&temp);
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let tries = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let w = DevWatch::new(FAST);
        let counted = tries.clone();
        w.set_handler(Arc::new(move |e| {
            let _ = lock(&tx).send(e);
            // The first three comparisons do not complete.
            if counted.fetch_add(1, Ordering::AcqRel) < 3 {
                Watching::Again
            } else {
                Watching::Keep
            }
        }));
        w.watch(one("com.example.a", &src));
        for _ in 0..4 {
            entered(&rx, "com.example.a");
        }
        assert!(
            rx.recv_timeout(FAST.poll * 6).is_err(),
            "done once it completes"
        );
        // Asked from outside: once more.
        w.recheck(&["com.example.a".to_string()]);
        entered(&rx, "com.example.a");
        w.stop();
    }
}
