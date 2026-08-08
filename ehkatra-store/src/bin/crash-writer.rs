//! A process that writes to a container and expects to be killed.
//!
//! BOOTSTRAP row 11's proof is "kill -9 mid-write test recovers", and that
//! cannot be faked in-process: a `drop` that runs, a destructor that flushes,
//! or a `File` that closes cleanly all hide exactly the failure being tested.
//! So this is a real executable, and the test really terminates it.
//!
//!     crash-writer <container-path> <total-ops> <commit-every>
//!
//! It prints `COMMITTED <n>` to stdout after each durability point and flushes
//! immediately, so the parent knows precisely how many ops the container has
//! *acknowledged* at the moment it pulls the trigger. Everything up to the last
//! `COMMITTED` line must survive; anything after it may or may not, and the
//! test asserts only the promise docs/16 actually makes.

use std::io::Write;

use ehkatra_store::{crash_corpus, Container};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).cloned().unwrap_or_else(|| {
        eprintln!("usage: crash-writer <path> <total-ops> <commit-every>");
        std::process::exit(2);
    });
    let total: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(2_000);
    let every: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(50);

    let mut container = match Container::open_or_create(&path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("open failed: {e}");
            std::process::exit(1);
        }
    };

    let ops = crash_corpus(total);
    let mut written = 0usize;
    let mut clock = 0u64;

    for chunk in ops.chunks(every) {
        clock += 1000; // well past the 250 ms cadence, so every batch commits
        if let Err(e) = container.append_ops(chunk, clock) {
            eprintln!("append failed: {e}");
            std::process::exit(1);
        }
        if let Err(e) = container.maybe_commit(clock + 1000, false) {
            eprintln!("commit failed: {e}");
            std::process::exit(1);
        }
        written += chunk.len();
        println!("COMMITTED {written}");
        let _ = std::io::stdout().flush();
        // Slow enough that the parent can reliably catch it mid-write, and
        // deliberately *not* a clean exit: this process is meant to die here.
        std::thread::sleep(std::time::Duration::from_millis(4));
    }

    println!("DONE {written}");
    let _ = std::io::stdout().flush();
    // Hold the file open, still in WAL mode, until killed. A clean shutdown
    // would checkpoint and defeat the point of the test.
    loop {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}
