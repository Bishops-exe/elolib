use criterion::{Criterion, criterion_group, criterion_main};
use elolib::{Matchmaker, Party, PartyId, Player, PlayerList};
use rand::RngExt;
use rand::prelude::*;
use std::hint::black_box;
use std::time::Duration;

fn bench(amount: u32) {
    // Keep results predictable
    let mut rng = StdRng::seed_from_u64(1337);

    // 3v3
    let mut matchmaker = Matchmaker::new(2, 3, 50);

    for i in 0..=amount {
        let player_count = rng.random_range(1..=3) as usize;

        let players = vec![0; player_count]
            .into_iter()
            .map(|_| {
                let player_elo = rng.random_range(100..3200);

                Player::new(0, player_elo)
            })
            .collect();

        matchmaker.enqueue(Party::new(i as PartyId, PlayerList::new(players)));
    }

    if let Some(matches) = matchmaker.tick() {
        for m in matches {
            black_box(m);
        }
    }
}

fn bench_main(c: &mut Criterion) {
    // Group name or individual bench identity
    c.bench_function("matchmake 1_000", |b| b.iter(|| bench(black_box(1_000))));
    c.bench_function("matchmake 10_000", |b| b.iter(|| bench(black_box(10_000))));
    c.bench_function("matchmake 100_000", |b| {
        b.iter(|| bench(black_box(100_000)))
    });
    c.bench_function("matchmake 1_000_000", |b| {
        b.iter(|| bench(black_box(1_000_000)))
    });
}

// Macro to generate the benchmark suite runner
criterion_group! {
    name = benches;
    config = Criterion::default().warm_up_time(Duration::from_secs(5)).measurement_time(Duration::from_secs(5));
    targets = bench_main
}
criterion_main!(benches);
