//! The audio callback must not allocate. A counting allocator watches this thread while
//! `Synth::render` runs with voices starting, stopping and being stolen, queued commands and a
//! generator in play.

use marionette_synth::*;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

struct Counting;
thread_local! { static COUNT: Cell<u64> = const { Cell::new(0) }; }
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        COUNT.with(|c| c.set(c.get() + 1));
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        COUNT.with(|c| c.set(c.get() + 1));
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        COUNT.with(|c| c.set(c.get() + 1));
        unsafe { System.realloc(p, l, n) }
    }
}
#[global_allocator]
static A: Counting = Counting;

struct Hum(f32);
impl Generator for Hum {
    fn render(&mut self, out: &mut [[f32; 2]], _f: u64, _r: f32) {
        for o in out {
            self.0 = (self.0 + 0.01).fract();
            o[0] += self.0 * 0.01;
        }
    }
}

#[test]
fn render_never_allocates() {
    let (mut synth, mut h) = create(48_000, 32);
    let mut out = vec![[0.0f32; 2]; 480];
    h.install(0, Box::new(Hum(0.0)));
    for i in 0..200u32 {
        let p = Patch::preset(["pluck", "pad", "bass", "lead", "thump", "wind", "bell"][(i % 7) as usize]).unwrap();
        h.note_on(u64::from(i) * 777, i, 100.0 + i as f32, 0.8, p);
        h.note_off(u64::from(i) * 777 + 5000, i);
    }
    h.master(20_000, 0.5);
    synth.render(&mut out); // first block drains the queue into the table
    let before = COUNT.with(Cell::get);
    for _ in 0..400 {
        synth.render(&mut out);
    }
    let after = COUNT.with(Cell::get);
    assert_eq!(after - before, 0, "render allocated or freed memory");
    assert!(synth.stats().stolen.load(std::sync::atomic::Ordering::Relaxed) > 0, "stealing was exercised");
}
