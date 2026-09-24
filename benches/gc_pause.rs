//! How long the collector holds up a frame.
//!
//! Runs `gc_pause.cdl`, a script shaped like a user interface, through the
//! VM-only embedding path and times every frame on the host side, so a
//! collection that lands inside a frame shows up in that frame's time
//! whichever collector the runtime has.
//!
//! Two shapes:
//!
//! - `retained`: one call keeps a tree of about 20,000 nodes alive and runs
//!   every frame against it, reporting each frame to the host. This is the
//!   pause a large live heap costs.
//! - `callbacks`: the host calls into the script once per frame, the way an
//!   event loop runs a handler, and each call builds and drops a section of
//!   200 nodes.
//! - `idle`: the same frames, with the host collecting between them, a
//!   budget of [`IDLE_BUDGET`] units at a time, where an event loop would
//!   wait. The frames and the collections are timed apart, since the
//!   collections run while nothing waits on the program.
//!
//! Run it with `cargo bench --bench gc_pause`. Pass a frame count to change
//! the default of 5000.

use candela::HostRegistry;
use candela::ImportResolver;
use candela::Value;
use candela::build_bytecode;
use candela::load_program;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;
use std::time::Instant;

const SCRIPT: &str = include_str!("gc_pause.cdl");

/// The units of collection work the `idle` shape asks for between frames.
const IDLE_BUDGET: u32 = 2000;

fn main() {
    let frames: usize = std::env::args()
        .skip(1)
        .find_map(|arg| arg.parse().ok())
        .unwrap_or(5000);

    let bytes = build_bytecode(SCRIPT.to_owned(), "gc_pause.cdl", &ImportResolver::new())
        .expect("the benchmark script builds");

    let ticks: Rc<RefCell<Vec<Instant>>> = Rc::new(RefCell::new(Vec::with_capacity(frames + 1)));
    let mut hosts = HostRegistry::new();
    {
        let ticks = Rc::clone(&ticks);
        hosts.register_host_fn("bench", "frame_done", move || {
            ticks.borrow_mut().push(Instant::now());
        });
    }

    // One call, a large live tree, a host tick per frame.
    let mut program = load_program(&bytes, &hosts).expect("the artifact loads");
    program.run();
    program
        .call("retained", &[Value::Int(frames as i64)])
        .expect("retained runs");
    let frame_times: Vec<Duration> = ticks
        .borrow()
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .collect();
    report("retained", &frame_times);

    // One call per frame, the way an event loop drives a handler.
    let mut program = load_program(&bytes, &hosts).expect("the artifact loads");
    program.run();
    let mut frame_times = Vec::with_capacity(frames);
    for f in 0..frames {
        let start = Instant::now();
        program
            .call("frame", &[Value::Int(f as i64)])
            .expect("frame runs");
        frame_times.push(start.elapsed());
    }
    report("callbacks", &frame_times);

    // The same frames, collecting while the loop would otherwise wait.
    let mut program = load_program(&bytes, &hosts).expect("the artifact loads");
    program.run();
    let mut frame_times = Vec::with_capacity(frames);
    let mut collect_times = Vec::with_capacity(frames);
    let mut idle_units = 0;
    for f in 0..frames {
        let start = Instant::now();
        program
            .call("frame", &[Value::Int(f as i64)])
            .expect("frame runs");
        frame_times.push(start.elapsed());
        let before = program.gc_stats().units;
        let start = Instant::now();
        program.collect(IDLE_BUDGET);
        collect_times.push(start.elapsed());
        idle_units += program.gc_stats().units - before;
    }
    report("idle", &frame_times);
    report("idle gc", &collect_times);
    let stats = program.gc_stats();
    println!(
        "idle gc      cycles {}  units {} ({idle_units} between frames)  largest slice {} units",
        stats.cycles, stats.units, stats.largest_slice
    );
}

/// Prints the median, the 99th percentile and the longest of `times`.
fn report(label: &str, times: &[Duration]) {
    let mut sorted = times.to_vec();
    sorted.sort_unstable();
    // The rank of permille `p`, rounded to the nearest frame.
    let at = |p: usize| sorted[((sorted.len() - 1) * p + 500) / 1000];
    let total: Duration = sorted.iter().sum();
    println!(
        "{label:<12} frames {:>6}  p50 {:>9.1?}  p99 {:>9.1?}  max {:>9.1?}  total {:>8.1?}",
        sorted.len(),
        at(500),
        at(990),
        sorted[sorted.len() - 1],
        total,
    );
}
