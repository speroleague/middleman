//! The `middleman` command-line surface.
//!
//! Thin over the core/packet/store/indexer libraries: parse arguments,
//! load or write files, call operations, print or render results.
//! Business rules and state transitions live in `middleman-core`, not
//! here.

fn main() {
    eprintln!(
        "middleman 0.1.0: scaffold only; the command surface lands in the \
         init/status/doctor slice"
    );
    std::process::exit(2);
}
