use criterion::{black_box, criterion_group, criterion_main, Criterion};

use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex,
};

#[inline]
fn _atomic(a: Arc<AtomicU64>) -> u64 {
    a.fetch_add(1, Ordering::SeqCst)
}

fn _primitive(a: u64) -> u64 {
    a + 1
}

fn _mutex(a: Arc<Mutex<u64>>) -> u64 {
    let mut a = a.lock().unwrap();
    *a += 1;
    *a
}

#[inline]
fn locking(a: u64) -> u64 {
    _atomic(Arc::new(AtomicU64::new(a)))
}

fn bench_locking(c: &mut Criterion) {
    c.bench_function("atomic", |b| b.iter(|| locking(black_box(20))));
}

criterion_group! {
    name = benches;
    config = Criterion::default();
    targets = bench_locking
}

criterion_main!(benches);
