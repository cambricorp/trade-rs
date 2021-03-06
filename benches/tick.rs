use criterion::{criterion_main, criterion_group, Criterion};
use trade::tick::Tick;

fn criterion_benchmark(c: &mut Criterion) {
    let tick = Tick::new(1000);

    c.bench_function(
        "ticked",
        move |b| b.iter(|| tick.ticked("1278.853").unwrap())
    );

    c.bench_function(
        "unticked",
        move |b| b.iter(|| tick.unticked(1278853).unwrap())
    );
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
