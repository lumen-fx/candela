//! The event queue: how work that finishes on another thread reaches a
//! program.
//!
//! A native library started from a `dylib` call, a JavaScript promise, or an
//! embedder posts an event under a source: an `int` the program handed out or
//! the library asked for. Each event carries one string, JSON by convention.
//! Posting never blocks on the program, and may happen from any thread.
//!
//! An event goes to the program that registered a handler for its source with
//! `event::on`, into that program's own queue, so several programs in one
//! process each see their own. The program runs the handler on its own thread
//! when it dispatches. Dispatch happens in `std/event`'s own candela code,
//! which reads the queue through the `event_next` primitive, so a handler is
//! called the way any closure is. An event for a source nobody registered a
//! handler for is dropped.
//!
//! A source is held while work under it is still running. `candela` and
//! `candela-vm` keep dispatching after `main` returns for as long as a source
//! the program handles is held, and stop once none is and its queue is empty.
//! Releasing a source queues the release behind the events posted before it,
//! so a handler sees every one of them before the source is forgotten.
//!
//! A native library reaches the queue through the table [`init_library`] hands
//! its optional `candela_dylib_init` export when the library loads. An embedder
//! that runs its own loop learns that something arrived through the waker it
//! sets with [`set_event_waker`].

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::Condvar;
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::sync::atomic::AtomicI64;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

/// One entry of a program's queue.
enum Item {
    /// A payload for the source's handler.
    Post(i64, String),
    /// The end of one hold on the source.
    Release(i64),
}

/// Every program's queue, what is held, and which program each source's
/// events go to, under one lock.
struct State {
    /// Each program's queue, by the id of its [`Inbox`].
    queues: Vec<(u64, VecDeque<Item>)>,
    /// The program each source with a handler belongs to.
    routes: Vec<(i64, u64)>,
    /// How many holds each source has that no handled release ended yet.
    held: Vec<(i64, u32)>,
}

impl State {
    const fn new() -> Self {
        Self {
            queues: Vec::new(),
            routes: Vec::new(),
            held: Vec::new(),
        }
    }

    fn route(&self, source: i64) -> Option<u64> {
        self.routes
            .iter()
            .find(|(s, _)| *s == source)
            .map(|(_, inbox)| *inbox)
    }

    fn queue(&mut self, inbox: u64) -> Option<&mut VecDeque<Item>> {
        self.queues
            .iter_mut()
            .find(|(id, _)| *id == inbox)
            .map(|(_, q)| q)
    }

    /// Ends one hold on `source`, answering whether it was the last.
    fn unhold(&mut self, source: i64) -> bool {
        let Some(at) = self.held.iter().position(|(s, _)| *s == source) else {
            return false;
        };
        self.held[at].1 -= 1;
        if self.held[at].1 == 0 {
            self.held.swap_remove(at);
            return true;
        }
        false
    }

    /// Whether a source whose events go to `inbox` is still held.
    fn holds_for(&self, inbox: u64) -> bool {
        self.held
            .iter()
            .any(|(source, _)| self.route(*source) == Some(inbox))
    }
}

static STATE: Mutex<State> = Mutex::new(State::new());
static ARRIVED: Condvar = Condvar::new();
static NEXT_SOURCE: AtomicI64 = AtomicI64::new(1);
static NEXT_INBOX: AtomicU64 = AtomicU64::new(1);

/// A function an embedder wants called whenever an event or a release is
/// queued, from whichever thread queued it.
pub type EventWaker = Arc<dyn Fn() + Send + Sync>;

static WAKER: Mutex<Option<EventWaker>> = Mutex::new(None);

fn state() -> MutexGuard<'static, State> {
    STATE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Tells a waiting dispatcher and the embedder's waker that a queue changed.
fn wake() {
    ARRIVED.notify_all();
    let waker = WAKER
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    if let Some(waker) = waker {
        waker();
    }
}

/// Sets the function called whenever an event or a release is queued, and
/// returns the one it replaces.
///
/// An embedder whose own loop sleeps until there is work sets this to wake
/// that loop, then calls `dispatch_events` on its program from the loop's
/// thread. The waker runs on whichever thread posted, possibly while the
/// program is running, so it should only signal: queue a wake-up, never call
/// into the program. There is one waker per process.
pub fn set_event_waker(waker: Option<EventWaker>) -> Option<EventWaker> {
    std::mem::replace(
        &mut *WAKER
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
        waker,
    )
}

/// A source no other source shares, for the life of the process.
#[must_use]
pub fn new_source() -> i64 {
    NEXT_SOURCE.fetch_add(1, Ordering::Relaxed)
}

/// Queues `payload` for the handler of `source`. An event for a source no
/// program registered a handler for is dropped.
pub fn post(source: i64, payload: String) {
    {
        let mut st = state();
        let Some(inbox) = st.route(source) else {
            return;
        };
        if let Some(queue) = st.queue(inbox) {
            queue.push_back(Item::Post(source, payload));
        }
    }
    wake();
}

/// Marks `source` as having work still running, which keeps a program that
/// finished `main` and handles the source dispatching.
pub fn hold(source: i64) {
    let mut st = state();
    if let Some(entry) = st.held.iter_mut().find(|(s, _)| *s == source) {
        entry.1 += 1;
    } else {
        st.held.push((source, 1));
    }
}

/// Ends one hold on `source`, behind every event already posted under it.
pub fn release(source: i64) {
    {
        let mut st = state();
        match st.route(source) {
            Some(inbox) if st.queue(inbox).is_some() => {
                if let Some(queue) = st.queue(inbox) {
                    queue.push_back(Item::Release(source));
                }
            }
            // Nobody handles the source, so nothing is waiting behind the
            // release.
            _ => {
                st.unhold(source);
            }
        }
    }
    wake();
}

/// One program's view of the queue: the events of the sources it registered
/// handlers for. Dropping it forgets them, and later events for them are
/// dropped.
#[derive(Debug)]
pub struct Inbox(u64);

impl Default for Inbox {
    fn default() -> Self {
        let id = NEXT_INBOX.fetch_add(1, Ordering::Relaxed);
        state().queues.push((id, VecDeque::new()));
        Self(id)
    }
}

impl Drop for Inbox {
    fn drop(&mut self) {
        let mut st = state();
        let id = self.0;
        st.queues.retain(|(q, _)| *q != id);
        st.routes.retain(|(_, inbox)| *inbox != id);
    }
}

/// What [`Inbox::next`] found.
pub enum Next {
    /// A payload for the source's handler.
    Event(i64, String),
    /// The last hold on the source ended; its handler can go.
    Ended(i64),
    /// Nothing is queued, and either the caller would not wait or nothing
    /// this program handles is held.
    Idle,
}

impl Inbox {
    /// Sends the events of `source` to this program from now on.
    pub fn route(&self, source: i64) {
        let mut st = state();
        let id = self.0;
        if let Some(entry) = st.routes.iter_mut().find(|(s, _)| *s == source) {
            entry.1 = id;
        } else {
            st.routes.push((source, id));
        }
    }

    /// Takes the next entry off this program's queue. With `wait`, blocks
    /// while the queue is empty and a source this program handles is held.
    #[must_use]
    pub fn next(&self, wait: bool) -> Next {
        let id = self.0;
        let mut st = state();
        loop {
            let item = st.queue(id).and_then(VecDeque::pop_front);
            match item {
                Some(Item::Post(source, payload)) => return Next::Event(source, payload),
                Some(Item::Release(source)) => {
                    if st.unhold(source) {
                        st.routes.retain(|(s, _)| *s != source);
                        return Next::Ended(source);
                    }
                }
                None => {
                    // A browser has one thread, so nothing could post while
                    // this one waited.
                    if !wait || !st.holds_for(id) || cfg!(target_arch = "wasm32") {
                        return Next::Idle;
                    }
                    st = ARRIVED
                        .wait(st)
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The table a native library receives
// ---------------------------------------------------------------------------

/// The version of [`EventTable`] this runtime hands out. A library compares it
/// with the version it was written against before using any field.
pub const EVENT_ABI: u32 = 1;

/// What a native library receives through its `candela_dylib_init` export:
/// the event queue and the embedder's base directory, as C functions.
///
/// ```c
/// typedef struct candela_runtime {
///     uint32_t abi;                                 /* 1 */
///     int64_t (*new_source)(void);
///     void (*post)(int64_t source, const char *payload);
///     void (*hold)(int64_t source);
///     void (*release)(int64_t source);
///     const char *(*base_dir)(void);
/// } candela_runtime;
///
/// void candela_dylib_init(const candela_runtime *runtime);
/// ```
///
/// The table lives as long as the process, so a library keeps the pointer.
/// Every function may be called from any thread. `post` copies the payload,
/// which is UTF-8 text. `base_dir` answers the directory relative paths
/// resolve against, or an empty string when the embedder set none, in which
/// case they resolve against the working directory.
#[repr(C)]
pub struct EventTable {
    pub abi: u32,
    pub new_source: extern "C" fn() -> i64,
    pub post: extern "C" fn(i64, *const std::ffi::c_char),
    pub hold: extern "C" fn(i64),
    pub release: extern "C" fn(i64),
    pub base_dir: extern "C" fn() -> *const std::ffi::c_char,
}

#[cfg(not(target_arch = "wasm32"))]
extern "C" fn c_new_source() -> i64 {
    new_source()
}

#[cfg(not(target_arch = "wasm32"))]
extern "C" fn c_post(source: i64, payload: *const std::ffi::c_char) {
    let text = if payload.is_null() {
        String::new()
    } else {
        unsafe { std::ffi::CStr::from_ptr(payload) }
            .to_string_lossy()
            .into_owned()
    };
    post(source, text);
}

#[cfg(not(target_arch = "wasm32"))]
extern "C" fn c_hold(source: i64) {
    hold(source);
}

#[cfg(not(target_arch = "wasm32"))]
extern "C" fn c_release(source: i64) {
    release(source);
}

#[cfg(not(target_arch = "wasm32"))]
extern "C" fn c_base_dir() -> *const std::ffi::c_char {
    crate::rt::base_dir_c().as_ptr()
}

#[cfg(not(target_arch = "wasm32"))]
static TABLE: EventTable = EventTable {
    abi: EVENT_ABI,
    new_source: c_new_source,
    post: c_post,
    hold: c_hold,
    release: c_release,
    base_dir: c_base_dir,
};

/// The name of the optional export a library receives the table through.
pub const INIT_SYMBOL: &[u8] = b"candela_dylib_init";

/// Hands `library` the table, when it exports `candela_dylib_init`. A library
/// without the export is left as it is. Called every time a program opens the
/// library, so the export has to be safe to call more than once.
#[cfg(not(target_arch = "wasm32"))]
pub fn init_library(library: &libloading::Library) {
    type Init = unsafe extern "C" fn(*const EventTable);
    if let Ok(init) = unsafe { library.get::<Init>(INIT_SYMBOL) } {
        unsafe { init(&raw const TABLE) };
    }
}
