//! Mutation fuzzing that runs inside `cargo test`, shared by the readers'
//! test files (`#[path = ".../fuzz_support/mod.rs"] mod fuzz_support;`).
//!
//! Each reader is fed its seed files cut short, bit-flipped, with fields set
//! to huge or negative values, spans repeated or deleted, then every file of
//! its regression corpus. A reader passes when it returns, whatever it
//! returns: a panic fails the test and the input is written under
//! `target/tmp/fuzz-crashes/` to be added to the corpus, and an allocation over
//! [`ALLOC_CAP`] stops the test binary after writing the input the same way.
//!
//! `MASSIF_FUZZ_ITERS` sets the mutations per seed (default 300), and
//! `MASSIF_FUZZ_SEED` the first random seed, to search longer than CI does.
#![allow(dead_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// The largest single allocation a reader may make on hostile input.
pub const ALLOC_CAP: usize = 1 << 30;

/// Refuses allocations over [`ALLOC_CAP`]; installed by each fuzz test
/// binary with `#[global_allocator]`.
pub struct Capped;

static WRITING: AtomicBool = AtomicBool::new(false);

thread_local! {
    /// The input being read on this thread, and where to write it if it
    /// asks for too much memory.
    static CURRENT: RefCell<Option<(Vec<u8>, PathBuf)>> = const { RefCell::new(None) };
    static INSIDE: Cell<bool> = const { Cell::new(false) };
}

unsafe impl GlobalAlloc for Capped {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if layout.size() > ALLOC_CAP {
            refused(layout.size());
            return std::ptr::null_mut();
        }
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if layout.size() > ALLOC_CAP {
            refused(layout.size());
            return std::ptr::null_mut();
        }
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if new_size > ALLOC_CAP {
            refused(new_size);
            return std::ptr::null_mut();
        }
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

/// Write the input that asked for `size` bytes, once, before the refusal
/// aborts the process (a fallible `try_reserve` survives it).
fn refused(size: usize) {
    if INSIDE.with(|i| i.replace(true)) {
        return;
    }
    if !WRITING.swap(true, Ordering::SeqCst) {
        CURRENT.with(|c| {
            if let Ok(c) = c.try_borrow()
                && let Some((bytes, path)) = c.as_ref()
            {
                let _ = std::fs::create_dir_all(path.parent().unwrap_or(Path::new(".")));
                let _ = std::fs::write(path, bytes);
                eprintln!("an allocation of {size} bytes was refused; the input is {}", path.display());
            }
        });
        WRITING.store(false, Ordering::SeqCst);
    }
    INSIDE.with(|i| i.set(false));
}

/// A small, fast, reproducible random source.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 { 0 } else { (self.next() % n as u64) as usize }
    }
}

const INTERESTING: [u64; 12] = [0, 1, 0x7F, 0x80, 0xFF, 0x7FFF, 0xFFFF, 0x7FFF_FFFF, 0x8000_0000, 0xFFFF_FFFF, 0x7FFF_FFFF_FFFF_FFFF, u64::MAX];

/// One to four random mutations of `seed`.
pub fn mutate(seed: &[u8], rng: &mut Rng) -> Vec<u8> {
    let mut b = seed.to_vec();
    for _ in 0..1 + rng.below(4) {
        let len = b.len();
        match rng.below(9) {
            0 => b.truncate(rng.below(len + 1)),
            1 | 2 => {
                for _ in 0..1 + rng.below(8) {
                    if !b.is_empty() {
                        let at = rng.below(b.len());
                        b[at] ^= 1 << rng.below(8);
                    }
                }
            }
            3 => {
                if !b.is_empty() {
                    let at = rng.below(b.len());
                    b[at] = INTERESTING[rng.below(5)] as u8;
                }
            }
            4 | 5 => {
                // A length, a count or an offset set to something hostile,
                // in either byte order.
                let width = [2, 4, 8][rng.below(3)];
                if len >= width {
                    let at = rng.below(len - width + 1);
                    let v = INTERESTING[rng.below(INTERESTING.len())];
                    let bytes = if rng.below(2) == 0 { v.to_le_bytes() } else { v.to_be_bytes() };
                    let bytes = if rng.below(2) == 0 { &bytes[..width] } else { &bytes[8 - width..] };
                    b[at..at + width].copy_from_slice(bytes);
                }
            }
            6 => {
                if len > 1 {
                    let at = rng.below(len);
                    let n = 1 + rng.below((len - at).min(256));
                    b.drain(at..at + n);
                }
            }
            7 => {
                if len > 1 {
                    let at = rng.below(len);
                    let n = 1 + rng.below((len - at).min(256));
                    let span: Vec<u8> = b[at..at + n].to_vec();
                    let to = rng.below(b.len() + 1);
                    b.splice(to..to, span);
                }
            }
            _ => {
                let at = rng.below(len + 1);
                let n = 1 + rng.below(16);
                let junk: Vec<u8> = (0..n).map(|_| rng.next() as u8).collect();
                b.splice(at..at, junk);
            }
        }
    }
    b
}

fn env(name: &str, default: u64) -> u64 {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn crash_dir() -> PathBuf {
    // CARGO_TARGET_TMPDIR lies under the target folder of whoever builds;
    // unit tests do not have it.
    option_env!("CARGO_TARGET_TMPDIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("fuzz-crashes")
}

fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3))
}

/// Run `read` on one input, catching a panic; `Some(path)` names where the
/// input was written when it panicked.
pub fn try_one(name: &str, bytes: &[u8], read: &(dyn Fn(&[u8]) + Sync)) -> Option<PathBuf> {
    let path = crash_dir().join(format!("{name}-{:016x}.bin", fnv(bytes)));
    CURRENT.with(|c| *c.borrow_mut() = Some((bytes.to_vec(), path.clone())));
    let ok = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| read(bytes))).is_ok();
    CURRENT.with(|c| *c.borrow_mut() = None);
    if ok {
        return None;
    }
    let _ = std::fs::create_dir_all(crash_dir());
    let _ = std::fs::write(&path, bytes);
    Some(path)
}

/// The regression corpus of `name`: every file in `dir` (if it exists).
pub fn corpus(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut files: Vec<_> = std::fs::read_dir(dir).map(|d| d.flatten().map(|e| e.path()).collect()).unwrap_or_default();
    files.sort();
    files.into_iter().filter_map(|p| Some((p.file_name()?.to_string_lossy().into_owned(), std::fs::read(&p).ok()?))).collect()
}

/// Fuzz `read` from `seeds`, then replay the corpus in `corpus_dir`.
/// Panics listing every input that made `read` panic.
pub fn fuzz(name: &str, seeds: &[Vec<u8>], corpus_dir: &Path, read: impl Fn(&[u8]) + Sync) {
    // Many seeds share the budget of a few.
    let iters = env("MASSIF_FUZZ_ITERS", 300);
    let iters = (iters * 4 / seeds.len().max(1) as u64).clamp(1, iters);
    let first = env("MASSIF_FUZZ_SEED", 1);
    // Panics are expected and caught: keep their messages short.
    let quiet = std::panic::take_hook();
    std::panic::set_hook(Box::new(|info| {
        let at = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
        eprintln!("  panic at {at}: {}", info.payload().downcast_ref::<String>().map(String::as_str).or(info.payload().downcast_ref::<&str>().copied()).unwrap_or("?"));
    }));
    let mut crashes = Vec::new();
    for (name_, bytes) in corpus(corpus_dir) {
        if let Some(p) = try_one(name, &bytes, &read) {
            crashes.push(format!("corpus {name_} ({})", p.display()));
        }
    }
    for (s, seed) in seeds.iter().enumerate() {
        if let Some(p) = try_one(name, seed, &read) {
            crashes.push(format!("seed {s} itself ({})", p.display()));
        }
        // Every prefix length class once, then random mutations.
        for cut in [0, 1, 4, 8, 16, 64, 256, seed.len() / 2, seed.len().saturating_sub(1)] {
            if cut < seed.len()
                && let Some(p) = try_one(name, &seed[..cut], &read)
            {
                crashes.push(format!("seed {s} cut to {cut} bytes ({})", p.display()));
            }
        }
        for i in 0..iters {
            let mut rng = Rng::new(first.wrapping_add(i).wrapping_add((s as u64) << 32));
            let bytes = mutate(seed, &mut rng);
            if let Some(p) = try_one(name, &bytes, &read) {
                crashes.push(format!("seed {s} mutation {i} ({})", p.display()));
                if crashes.len() > 20 {
                    break;
                }
            }
        }
    }
    std::panic::set_hook(quiet);
    assert!(crashes.is_empty(), "{name} panicked on {} inputs:\n{}", crashes.len(), crashes.join("\n"));
}
