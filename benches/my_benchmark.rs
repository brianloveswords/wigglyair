use criterion::{black_box, criterion_group, criterion_main, Criterion};

#[inline]
fn copy_buf(data: &mut [f32], buf: Vec<f32>) {
    let size = data.len();
    let mut buf: Vec<f32> = buf.iter().map(|s| s * 0.5).collect();
    data.copy_from_slice(&buf[..size]);
    buf.drain(..size);
}

fn bench_copy_buf(c: &mut Criterion) {
    c.bench_function("copy_buf", |b| {
        b.iter(|| {
            copy_buf(
                black_box(&mut [0.0f32; 8192]),
                black_box(vec![1.0f32; 9000]),
            )
        })
    });
}

criterion_group! {
    name = benches;
    config = Criterion::default();
    targets = bench_copy_buf
}

criterion_main!(benches);
