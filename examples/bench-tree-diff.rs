//! Fixed-record host-tree diff used by bench-workloads.tsx.
//!
//! Snapshot construction and file I/O happen outside the timed region. Each
//! record is already in the representation a binary napi protocol could hand
//! Rust: sorted by stable host id, with parent/order and host values reduced to
//! fixed-width fields. The benchmark measures only the linear Rust diff.

use std::env;
use std::fs;
use std::hint::black_box;
use std::time::Instant;

const RECORD_BYTES: usize = 40;

fn id(record: &[u8]) -> u32 {
    u32::from_le_bytes(record[0..4].try_into().unwrap())
}

fn diff(before: &[u8], after: &[u8]) -> usize {
    let mut left = before.chunks_exact(RECORD_BYTES).peekable();
    let mut right = after.chunks_exact(RECORD_BYTES).peekable();
    let mut changed = 0;

    while let (Some(old), Some(new)) = (left.peek(), right.peek()) {
        match id(old).cmp(&id(new)) {
            std::cmp::Ordering::Less => {
                changed += 1;
                left.next();
            }
            std::cmp::Ordering::Greater => {
                changed += 1;
                right.next();
            }
            std::cmp::Ordering::Equal => {
                if old != new {
                    changed += 1;
                }
                left.next();
                right.next();
            }
        }
    }
    changed + left.count() + right.count()
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let before = fs::read(&args[1]).expect("read before snapshot");
    let after = fs::read(&args[2]).expect("read after snapshot");
    let iterations: usize = args[3].parse().expect("iterations");
    assert_eq!(before.len() % RECORD_BYTES, 0);
    assert_eq!(after.len() % RECORD_BYTES, 0);

    for _ in 0..10 {
        black_box(diff(&before, &after));
    }

    let mut samples = Vec::with_capacity(iterations);
    let mut changed = 0;
    for _ in 0..iterations {
        let start = Instant::now();
        changed = black_box(diff(&before, &after));
        samples.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    samples.sort_by(f64::total_cmp);
    let median_ms = samples[samples.len() / 2];
    println!(
        "{{\"medianMs\":{median_ms:.6},\"changed\":{changed},\"before\":{},\"after\":{}}}",
        before.len() / RECORD_BYTES,
        after.len() / RECORD_BYTES,
    );
}
