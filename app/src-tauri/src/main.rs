// Hide the console window in release builds on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, Ordering};

/// The system allocator, which reports where any single request over 1 GB comes from
/// (a backtrace on the terminal) before passing it on: such a request is a bug, and
/// otherwise the app just stops with "memory allocation failed" and no clue where.
struct Watchful;

static REPORTING: AtomicBool = AtomicBool::new(false);
const HUGE: usize = 1 << 30;

fn report(size: usize) {
    if REPORTING.swap(true, Ordering::SeqCst) {
        return;
    }
    let bt = std::backtrace::Backtrace::force_capture();
    eprintln!("\n*** request for {size} bytes of memory, from:\n{bt}\n");
    REPORTING.store(false, Ordering::SeqCst);
}

unsafe impl GlobalAlloc for Watchful {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        if l.size() > HUGE {
            report(l.size());
        }
        System.alloc(l)
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        if l.size() > HUGE {
            report(l.size());
        }
        System.alloc_zeroed(l)
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        System.dealloc(p, l)
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
        if new > HUGE {
            report(new);
        }
        System.realloc(p, l, new)
    }
}

#[global_allocator]
static ALLOC: Watchful = Watchful;

fn main() {
    racenav_app_lib::migrate_from_racenav();
    racenav_app_lib::run()
}
