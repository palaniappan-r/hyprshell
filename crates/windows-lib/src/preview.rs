mod frame;
pub mod layout;

use frame::{Frame, read_frame};
use relm4::gtk::{self, gdk, glib, prelude::*};
use std::{
    cell::RefCell,
    collections::HashMap,
    io::BufReader,
    path::Path,
    process::{Child, Command, Stdio},
    rc::Rc,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Instant,
};

/// Shared by all cards for one opening, including cards rebuilt after a close.
/// Cache failed captures too, so a relayout never retries a frozen preview.
pub type SnapshotSession = Rc<RefCell<HashMap<u64, Option<gdk::MemoryTexture>>>>;

/// Keep the last frame across openings; include PID to avoid reusing a frame
/// if the compositor recycles a window address for a different application.
pub type SnapshotCache = Rc<RefCell<HashMap<(u64, i32), gdk::MemoryTexture>>>;

#[derive(Default)]
struct Capture {
    child: Arc<Mutex<Option<Child>>>,
    cancelled: Arc<AtomicBool>,
    task: Option<glib::JoinHandle<()>>,
}

impl Capture {
    fn stop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
        if let Some(task) = self.task.take() {
            task.abort();
        }
        let child = self.child.lock().expect("capture process lock").take();
        if let Some(mut child) = child {
            let _ = child.kill();
            // Reaping must not block keyboard handling.
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Start capture as soon as a card is prepared, while the popup is still hidden.
/// A previous frame fills the card immediately, then one fresh frame replaces
/// it for this opening. Dropping/unmapping the card stops capture.
pub fn attach(
    picture: &gtk::Picture,
    address: u64,
    pid: i32,
    size: (i32, i32),
    session: SnapshotSession,
    cache: SnapshotCache,
) {
    let state = Rc::new(RefCell::new(Capture::default()));
    let unmap_state = Rc::clone(&state);
    picture.connect_unmap(move |picture| {
        unmap_state.borrow_mut().stop();
        picture.set_paintable(None::<&gdk::Paintable>);
    });

    if let Some(texture) = cache.borrow().get(&(address, pid)) {
        picture.set_paintable(Some(texture));
        tracing::debug!(address, "Using retained window preview");
    }
    let cached = session.borrow().get(&address).cloned();
    if let Some(snapshot) = cached {
        if let Some(texture) = snapshot {
            picture.set_paintable(Some(&texture));
        }
        return;
    }
    let helper = std::env::var_os("HYPRSHELL_PREVIEW_HELPER").map_or_else(
        || {
            std::env::current_exe()
                .unwrap_or_default()
                .with_file_name("hyprshell-capture")
        },
        std::path::PathBuf::from,
    );
    let started = Instant::now();
    let (notify, ready) = relm4::channel::<Option<Frame>>();
    let child = Arc::clone(&state.borrow().child);
    let cancelled = Arc::clone(&state.borrow().cancelled);
    // Keep process startup and full-frame reads off the GTK thread. Bound the
    // compositor's concurrent GPU readbacks instead of flooding it on opening.
    capture_jobs()
        .send(Box::new(move || {
            let frame = capture_frame(&helper, address, size, &child, &cancelled);
            if !cancelled.load(Ordering::Relaxed) {
                let _ = notify.send(frame);
            }
        }))
        .expect("capture workers are running");
    let weak_state = Rc::downgrade(&state);
    let picture = picture.downgrade();
    let session = Rc::clone(&session);
    let task = glib::MainContext::default().spawn_local(async move {
        if let Some(snapshot) = ready.recv().await {
            if let Some(frame) = snapshot {
                tracing::debug!(
                    address,
                    elapsed_ms = started.elapsed().as_millis(),
                    "Captured frozen window preview"
                );
                let bytes = glib::Bytes::from_owned(frame.pixels);
                let texture = gdk::MemoryTexture::new(
                    frame.width as i32,
                    frame.height as i32,
                    gdk::MemoryFormat::R8g8b8a8Premultiplied,
                    &bytes,
                    frame.width as usize * 4,
                );
                if let Some(picture) = picture.upgrade() {
                    picture.set_paintable(Some(&texture));
                }
                cache.borrow_mut().insert((address, pid), texture.clone());
                session.borrow_mut().insert(address, Some(texture));
            } else {
                session.borrow_mut().insert(address, None);
            }
        }
        // Keep the snapshot in GTK, release all capture resources now.
        if let Some(state) = weak_state.upgrade() {
            let mut state = state.borrow_mut();
            state.task.take();
            state.stop();
        }
    });
    state.borrow_mut().task = Some(task);
}

const CAPTURE_WORKERS: usize = 4;
type CaptureJob = Box<dyn FnOnce() + Send>;

fn capture_jobs() -> &'static mpsc::Sender<CaptureJob> {
    static JOBS: OnceLock<mpsc::Sender<CaptureJob>> = OnceLock::new();
    JOBS.get_or_init(|| {
        let (send, receive) = mpsc::channel::<CaptureJob>();
        let receive = Arc::new(Mutex::new(receive));
        for _ in 0..CAPTURE_WORKERS {
            let receive = Arc::clone(&receive);
            std::thread::spawn(move || {
                loop {
                    let job = receive.lock().expect("capture queue lock").recv();
                    let Ok(job) = job else { break };
                    job();
                }
            });
        }
        send
    })
}

fn capture_frame(
    helper: &Path,
    address: u64,
    size: (i32, i32),
    process: &Mutex<Option<Child>>,
    cancelled: &AtomicBool,
) -> Option<Frame> {
    if cancelled.load(Ordering::Relaxed) {
        return None;
    }
    let mut child = match Command::new(helper)
        .arg(format!("{address:x}"))
        .arg((size.0 * 2).clamp(1, 720).to_string())
        .arg((size.1 * 2).clamp(1, 450).to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            tracing::warn!("Cannot start window preview: {error}");
            return None;
        }
    };
    let stdout = child.stdout.take();
    {
        let mut process = process.lock().expect("capture process lock");
        // A card may have been dropped while its helper was spawning.
        if cancelled.load(Ordering::Relaxed) {
            drop(process);
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        *process = Some(child);
    }
    // Read exactly one frame, including from an older streaming helper.
    let frame = stdout.and_then(|stdout| read_frame(&mut BufReader::new(stdout)).ok());
    let child = process.lock().expect("capture process lock").take();
    if let Some(mut child) = child {
        let _ = child.kill();
        let _ = child.wait();
    }
    frame
}

#[cfg(test)]
mod tests {
    use super::{CAPTURE_WORKERS, Capture, capture_jobs};
    use std::{
        io::Read,
        process::{Command, Stdio},
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
            mpsc,
        },
        time::Duration,
    };

    #[test]
    fn concurrent_capture_work_is_bounded() {
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let (send, receive) = mpsc::channel();
        for _ in 0..CAPTURE_WORKERS * 3 {
            let active = Arc::clone(&active);
            let peak = Arc::clone(&peak);
            let send = send.clone();
            capture_jobs()
                .send(Box::new(move || {
                    let count = active.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(count, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(10));
                    active.fetch_sub(1, Ordering::SeqCst);
                    let _ = send.send(());
                }))
                .expect("queue capture");
        }
        for _ in 0..CAPTURE_WORKERS * 3 {
            receive
                .recv_timeout(Duration::from_secs(2))
                .expect("capture finished");
        }
        assert!(peak.load(Ordering::SeqCst) <= CAPTURE_WORKERS);
        assert_eq!(active.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn dropping_an_unfinished_capture_closes_its_reader_promptly() {
        let mut child = Command::new("sleep")
            .arg("30")
            .stdout(Stdio::piped())
            .spawn()
            .expect("start stand-in capture process");
        let mut stdout = child.stdout.take().expect("capture output pipe");
        let capture = Capture::default();
        *capture.child.lock().expect("capture process lock") = Some(child);
        let (send, receive) = mpsc::channel();
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let result = stdout.read_to_end(&mut bytes);
            let _ = send.send(result);
        });
        // Cards may be dropped on a quick release without ever mapping.
        drop(capture);
        assert_eq!(
            receive
                .recv_timeout(Duration::from_secs(2))
                .expect("reader must not linger after cancellation")
                .expect("read capture output"),
            0
        );
    }
}
