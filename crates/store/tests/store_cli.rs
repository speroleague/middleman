//! Child-process helper for the lock tests: holds an exclusive file
//! lock on the given path for the given number of milliseconds, then
//! releases it by exiting. A separate process is required because
//! advisory lock semantics differ per platform (and often within one
//! process), while a fresh process excludes itself everywhere.

#![allow(clippy::expect_used)]

use std::fs::File;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use fs2::FileExt;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    // Run by `cargo test` without child arguments: nothing to do.
    let (path, ms) = match (args.get(1), args.get(2)) {
        (Some(path), Some(ms)) => (PathBuf::from(path), ms.parse::<u64>().expect("ms")),
        _ => return,
    };
    let file = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .expect("lock file opens");
    file.lock_exclusive().expect("takes the lock");
    // Tell the parent that the lock is now held.
    std::fs::write(path.with_extension("ready"), b"locked").expect("ready marker");
    thread::sleep(Duration::from_millis(ms));
}
