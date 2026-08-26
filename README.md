# elolib

A Rust library for elo-based matchmaking. It groups queued parties of players into balanced teams, then groups those
teams into matches, keeping average elo difference within a configurable limit.

## How it works

- **Parties** (groups of players queueing together) are enqueued and bucketed by size, smallest first, so small parties
  get matched before large ones.
- **Teams** are built by merging parties whose average elo is close enough (within `max_elo_diff`) to an existing
  partial team of the right size, using a `BTreeMap` range lookup instead of scanning every candidate.
- **Matches** are formed the same way, by grouping completed teams with compatible average elo into `teams_per_match`
  groups.

`Matchmaker::tick` returns a `Some(genawaiter::Gen)`
that yields `Match`es as they're formed, when there aren't enough players in the queue,
`tick` returns `None`

## Usage

```rust
use elolib::{Matchmaker, Party, PartyId, Player, PlayerList};

fn main() {
    // 3v3, max average elo diff of 50.
    let mut matchmaker = Matchmaker::new(2, 3, 50);

    matchmaker.enqueue(Party::new(
        0 as PartyId,
        PlayerList::new(vec![Player::new(1, 1200), Player::new(2, 1180)]),
    ));
    matchmaker.enqueue(Party::new(1, PlayerList::single(Player::new(3, 1210))));
    matchmaker.enqueue(Party::new(2, PlayerList::new(vec![Player::new(4, 1190); 3])));

    if let Some(matches) = matchmaker.tick() {
        for m in matches {
            println!("{m:?}");
        }
    }
}
```

## Testing

```sh
cargo test
```

## Benchmarks


Benchmarks use [`criterion`] and simulate matchmaking over randomly generated parties:

```sh
cargo bench
```

[`genawaiter`]: https://crates.io/crates/genawaiter
[`criterion`]: https://crates.io/crates/criterion
