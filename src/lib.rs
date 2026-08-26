use genawaiter::{
    sync::{Gen, r#gen},
    yield_,
};

use core::{fmt::Debug, ops::Deref};
use std::collections::{BTreeMap, HashMap, VecDeque};

pub type PartyId = u64;
pub type PlayerId = u64;

#[derive(Debug, Clone)]
pub struct Player {
    pub name: PlayerId,
    pub elo: u32,
}

impl Player {
    pub fn new(id: PlayerId, elo: u32) -> Player {
        Self { name: id, elo }
    }
}

#[derive(Debug, Clone, Default)]
pub struct PlayerList {
    pub players: Vec<Player>,
}

impl PlayerList {
    pub fn new(players: Vec<Player>) -> Self {
        Self { players }
    }

    pub fn single(player: Player) -> Self {
        Self {
            players: vec![player],
        }
    }

    pub fn with_capacity(size: usize) -> Self {
        Self {
            players: Vec::with_capacity(size),
        }
    }

    pub fn average_elo(&self) -> u64 {
        let sum: u64 = self.players.iter().map(|p| p.elo as u64).sum();

        sum / self.players.len() as u64
    }

    pub fn extend(&mut self, arr2: PlayerList) {
        self.players.extend(arr2.players);
    }

    /**
     * Whether this list's average elo is close enough to another's
     * to be grouped together (into the same team, or the same match).
     */
    pub fn is_elo_compatible(&self, other: &PlayerList, max_diff: u32) -> bool {
        self.average_elo().abs_diff(other.average_elo()) <= max_diff as u64
    }
}

impl Deref for PlayerList {
    type Target = Vec<Player>;

    fn deref(&self) -> &Self::Target {
        &self.players
    }
}

#[derive(Debug, Clone)]
pub struct Party {
    pub id: PartyId,
    pub players: PlayerList,
}

impl Party {
    pub fn new(id: PartyId, players: PlayerList) -> Self {
        Self { id, players }
    }

    pub fn size(&self) -> usize {
        self.players.len()
    }
}

#[derive(Debug)]
pub struct Team {
    pub players: PlayerList,
}

#[derive(Debug)]
pub struct Match {
    pub teams: Vec<Team>,
}

#[derive(Debug)]
struct PartialTeam {
    players: PlayerList,
}

impl PartialTeam {
    fn add_party(&mut self, party: Party) {
        self.players.extend(party.players);
    }

    fn complete(self) -> Team {
        Team {
            players: self.players,
        }
    }
}

/**
 * Push `value` onto the back of the bucket at `key`, creating the
 * bucket if it doesn't exist yet.
 */
macro_rules! push_into_bucket {
    ($map:expr, $key:expr, $value:expr) => {
        $map.entry($key).or_default().push_back($value);
    };
}

/**
 * Pop the front of the bucket at `key`, removing the bucket entirely
 * if that empties it.
 */
macro_rules! pop_from_bucket {
    ($map:expr, $key:expr) => {{
        match $map.get_mut(&$key) {
            None => None,
            Some(queue) => {
                let value = queue.pop_front();

                #[cfg(feature = "dealloc")]
                if queue.is_empty() {
                    $map.remove(&$key);
                }

                value
            }
        }
    }};
}

/**
 * Find the bucket whose elo key is within `max_diff` of `target_elo`
 * via a range lookup, and pop its front value. Returns the bucket's
 * elo key alongside the value, since it may differ from `target_elo`.
 */
fn pop_elo_compatible<V>(
    map: &mut BTreeMap<u64, VecDeque<V>>,
    target_elo: u64,
    max_diff: u32,
) -> Option<(u64, V)> {
    let low = target_elo.saturating_sub(max_diff as u64);
    let high = target_elo + max_diff as u64;

    let elo = map.range(low..=high).next().map(|(&elo, _)| elo)?;
    let value = pop_from_bucket!(map, elo)?;

    Some((elo, value))
}

pub struct Matchmaker {
    /**
     * Parties waiting to be placed into a team, queued by party size
     * so the smallest parties can be matched first.
     *
     * party_size -> parties
     *
     * Example for 5v5:
     *
     * 1 -> [party, party]
     * 2 -> [party]
     * 3 -> [party, party]
     * 4 -> [party]
     */
    parties_by_size: HashMap<usize, VecDeque<Party>>,

    /**
     * Teams under construction, keyed by how many more players each
     * still needs, then by average elo so a compatible team can be
     * found with a range lookup instead of a linear scan.
     *
     * remaining_slots -> (average_elo -> partial teams)
     *
     * Example for 5-player teams:
     *
     * 1 -> teams needing 1 player, by elo
     * 2 -> teams needing 2 players, by elo
     * 3 -> teams needing 3 players, by elo
     * 4 -> teams needing 4 players, by elo
     */
    partial_teams: HashMap<usize, BTreeMap<u64, VecDeque<PartialTeam>>>,

    /**
     * Teams that have been completely constructed, awaiting a match,
     * keyed by average elo so a compatible team can be found with a
     * range lookup instead of a linear scan.
     */
    completed_teams: BTreeMap<u64, VecDeque<Team>>,
    completed_team_count: usize,

    teams_per_match: usize,
    players_per_team: usize,

    /**
     * Maximum average elo difference allowed: between a party and a
     * partial team when forming a team, and between teams when
     * grouping them into a match.
     */
    max_elo_diff: u32,

    queued_players: usize,
}

impl Matchmaker {
    pub fn new(teams_per_match: usize, players_per_team: usize, max_elo_diff: u32) -> Self {
        #[cfg(debug_assertions)]
        {
            assert_ne!(teams_per_match, 0);
            assert_ne!(players_per_team, 0);
        }

        Self {
            parties_by_size: HashMap::with_capacity(players_per_team),
            partial_teams: HashMap::with_capacity(players_per_team),
            completed_teams: BTreeMap::new(),
            completed_team_count: 0,

            teams_per_match,
            players_per_team,
            max_elo_diff,

            queued_players: 0,
        }
    }

    pub fn enqueue(&mut self, party: Party) {
        let size = party.size();

        #[cfg(debug_assertions)]
        assert!(
            size <= self.players_per_team && !party.players.is_empty(),
            "party is empty or is larger than a team"
        );

        self.queued_players += size;

        push_into_bucket!(self.parties_by_size, size, party);
    }

    pub fn queued_players(&self) -> usize {
        self.queued_players
    }

    pub fn tick(&mut self) -> Option<Gen<Match, (), impl Future<Output = ()>>> {
        /*
         * First construct as many teams as possible.
         */
        if self.queued_players < self.players_per_team {
            return None;
        }

        self.build_teams();

        /*
         * Once we have complete teams, matchmaking becomes much
         * simpler: take compatible teams and form matches.
         */
        Some(self.build_matches())
    }

    fn build_teams(&mut self) {
        /*
         * Move every queued party into a partial-team bucket, merging
         * it into a compatible partial team or starting a new one.
         */
        while let Some(party) = self.pop_first_party() {
            let size = party.size();

            self.queued_players -= size;

            /*
             * The party already fills a whole team on its own.
             */
            if size == self.players_per_team {
                self.push_completed_team(Team {
                    players: party.players,
                });

                continue;
            }

            let party_elo = party.players.average_elo();

            /*
             * Look for an existing team with exactly enough space and a
             * compatible average elo, via a range lookup on elo instead
             * of scanning every team needing that many players.
             */
            if let Some(mut team) = self.pop_from_partial_teams(size, party_elo) {
                team.add_party(party);

                self.push_completed_team(team.complete());

                continue;
            }

            /*
             * No fit with a compatible elo.
             *
             * Start a new partial team.
             */
            let remaining = self.players_per_team - size;

            let partial = PartialTeam {
                players: party.players,
            };

            push_into_bucket!(
                self.partial_teams.entry(remaining).or_default(),
                party_elo,
                partial
            );
        }
    }

    // ------------------------------------------------------------
    // Match construction
    // ------------------------------------------------------------

    fn build_matches(&mut self) -> Gen<Match, (), impl Future<Output = ()>> {
        r#gen!({
            while self.completed_team_count >= self.teams_per_match {
                let Some((&first_elo, _)) = self.completed_teams.iter().next() else {
                    break;
                };

                let first = pop_from_bucket!(self.completed_teams, first_elo).unwrap();
                self.completed_team_count -= 1;

                let mut teams = Vec::with_capacity(self.teams_per_match);
                teams.push((first_elo, first));

                /*
                 * Find compatible teams via a range lookup on elo
                 * instead of scanning every completed team.
                 */
                while teams.len() < self.teams_per_match {
                    let Some(x) =
                        pop_elo_compatible(&mut self.completed_teams, first_elo, self.max_elo_diff)
                    else {
                        break;
                    };

                    self.completed_team_count -= 1;

                    teams.push(x);
                }

                if teams.len() == self.teams_per_match {
                    yield_!(Match {
                        teams: teams.into_iter().map(|(_, team)| team).collect(),
                    });
                } else {
                    // Not enough compatible teams, restore the selected teams.

                    for (elo, team) in teams {
                        push_into_bucket!(self.completed_teams, elo, team);
                        self.completed_team_count += 1;
                    }

                    break;
                }
            }
        })
    }

    // ------------------------------------------------------------
    // Queue helpers
    // ------------------------------------------------------------

    fn pop_first_party(&mut self) -> Option<Party> {
        self.pop_party(*self.parties_by_size.keys().next()?)
    }

    fn pop_party(&mut self, size: usize) -> Option<Party> {
        pop_from_bucket!(self.parties_by_size, size)
    }

    fn push_completed_team(&mut self, team: Team) {
        let elo = team.players.average_elo();

        push_into_bucket!(self.completed_teams, elo, team);
        self.completed_team_count += 1;
    }

    fn pop_from_partial_teams(&mut self, size: usize, party_elo: u64) -> Option<PartialTeam> {
        let teams = self.partial_teams.get_mut(&size)?;
        let (_, team) = pop_elo_compatible(teams, party_elo, self.max_elo_diff)?;

        #[cfg(feature = "dealloc")]
        if teams.is_empty() {
            self.partial_teams.remove(&size);
        }

        Some(team)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn player(id: PlayerId, elo: u32) -> Player {
        Player::new(id, elo)
    }

    fn party(id: PartyId, players: Vec<Player>) -> Party {
        Party::new(id, PlayerList::new(players))
    }

    // ------------------------------------------------------------
    // Player / PlayerList
    // ------------------------------------------------------------

    #[test]
    fn player_new_sets_fields() {
        let p = player(1, 1200);
        assert_eq!(p.name, 1);
        assert_eq!(p.elo, 1200);
    }

    #[test]
    fn player_list_average_elo_multiple_players() {
        let list = PlayerList::new(vec![player(1, 1000), player(2, 2000)]);
        assert_eq!(list.average_elo(), 1500);
    }

    #[test]
    fn player_list_average_elo_truncates_towards_zero() {
        let list = PlayerList::new(vec![player(1, 1000), player(2, 1001)]);
        assert_eq!(list.average_elo(), 1000);
    }

    #[test]
    fn player_list_average_elo_single_player() {
        let list = PlayerList::single(player(1, 1234));
        assert_eq!(list.average_elo(), 1234);
    }

    #[test]
    fn player_list_extend_appends_players() {
        let mut list = PlayerList::new(vec![player(1, 1000)]);
        list.extend(PlayerList::new(vec![player(2, 2000)]));

        assert_eq!(list.players.len(), 2);
        assert_eq!(list.players[1].name, 2);
    }

    #[test]
    fn player_list_with_capacity_starts_empty() {
        let list = PlayerList::with_capacity(10);
        assert!(list.players.is_empty());
    }

    #[test]
    fn player_list_deref_exposes_vec_methods() {
        let list = PlayerList::new(vec![player(1, 1000), player(2, 2000)]);

        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, 1);
    }

    #[test]
    fn is_elo_compatible_within_range() {
        let a = PlayerList::single(player(1, 1000));
        let b = PlayerList::single(player(2, 1040));
        assert!(a.is_elo_compatible(&b, 50));
    }

    #[test]
    fn is_elo_compatible_at_exact_boundary() {
        let a = PlayerList::single(player(1, 1000));
        let b = PlayerList::single(player(2, 1050));
        assert!(a.is_elo_compatible(&b, 50));
    }

    #[test]
    fn is_elo_compatible_outside_range() {
        let a = PlayerList::single(player(1, 1000));
        let b = PlayerList::single(player(2, 1051));
        assert!(!a.is_elo_compatible(&b, 50));
    }

    // ------------------------------------------------------------
    // Party
    // ------------------------------------------------------------

    #[test]
    fn party_size_matches_player_count() {
        let p = party(1, vec![player(1, 1000), player(2, 1000)]);
        assert_eq!(p.size(), 2);
    }

    // ------------------------------------------------------------
    // Bucket helpers
    // ------------------------------------------------------------

    #[test]
    fn bucket_push_pop_is_fifo() {
        let mut map: BTreeMap<usize, VecDeque<i32>> = BTreeMap::new();
        push_into_bucket!(&mut map, 1, 10);
        push_into_bucket!(&mut map, 1, 20);

        assert_eq!(pop_from_bucket!(&mut map, 1), Some(10));
        assert_eq!(pop_from_bucket!(&mut map, 1), Some(20));
    }

    #[test]
    fn bucket_removed_once_emptied() {
        let mut map: BTreeMap<usize, VecDeque<i32>> = BTreeMap::new();
        push_into_bucket!(&mut map, 1, 10);
        pop_from_bucket!(&mut map, 1);

        assert!(!map.contains_key(&1));
    }

    #[test]
    fn pop_from_missing_bucket_returns_none() {
        let mut map: BTreeMap<usize, VecDeque<i32>> = BTreeMap::new();
        assert_eq!(pop_from_bucket!(&mut map, 1), None);
    }

    // ------------------------------------------------------------
    // Matchmaker
    // ------------------------------------------------------------

    #[test]
    fn new_matchmaker_has_no_queued_players() {
        let mm = Matchmaker::new(2, 3, 50);
        assert_eq!(mm.queued_players(), 0);
    }

    #[test]
    #[cfg_attr(debug_assertions, should_panic)]
    fn new_with_zero_teams_per_match_panics_in_debug() {
        Matchmaker::new(0, 3, 50);
    }

    #[test]
    #[cfg_attr(debug_assertions, should_panic)]
    fn new_with_zero_players_per_team_panics_in_debug() {
        Matchmaker::new(2, 0, 50);
    }

    #[test]
    fn enqueue_increments_queued_players() {
        let mut mm = Matchmaker::new(2, 3, 50);
        mm.enqueue(party(1, vec![player(1, 1000)]));
        assert_eq!(mm.queued_players(), 1);

        mm.enqueue(party(2, vec![player(2, 1000), player(3, 1000)]));
        assert_eq!(mm.queued_players(), 3);
    }

    #[test]
    #[cfg_attr(debug_assertions, should_panic)]
    fn enqueue_party_larger_than_team_panics_in_debug() {
        let mut mm = Matchmaker::new(2, 3, 50);
        mm.enqueue(party(
            1,
            vec![
                player(1, 1000),
                player(2, 1000),
                player(3, 1000),
                player(4, 1000),
            ],
        ));
    }

    #[test]
    fn tick_returns_none_when_not_enough_players_queued() {
        let mut mm = Matchmaker::new(2, 3, 50);
        mm.enqueue(party(1, vec![player(1, 1000)]));

        assert!(mm.tick().is_none());
    }

    #[test]
    fn tick_consumes_queue_even_without_forming_a_match() {
        let mut mm = Matchmaker::new(2, 3, 50);
        mm.enqueue(party(
            1,
            vec![player(1, 1000), player(2, 1000), player(3, 1000)],
        ));

        // One full team is formed, but two are needed for a match.
        let matches = mm.tick();
        assert!(matches.is_some());
        assert_eq!(matches.unwrap().into_iter().count(), 0);
        assert_eq!(mm.queued_players(), 0);
    }

    #[test]
    fn tick_merges_compatible_parties_into_a_team_and_match() {
        let mut mm = Matchmaker::new(2, 2, 50);

        // Team A: two size-1 parties with compatible elo.
        mm.enqueue(party(1, vec![player(1, 1000)]));
        mm.enqueue(party(2, vec![player(2, 1010)]));

        // Team B: one full-size party with elo compatible with Team A.
        mm.enqueue(party(3, vec![player(3, 1005), player(4, 1005)]));

        let matches: Vec<Match> = mm.tick().unwrap().into_iter().collect();

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].teams.len(), 2);
        for team in &matches[0].teams {
            assert_eq!(team.players.len(), 2);
        }
    }

    #[test]
    fn tick_does_not_match_elo_incompatible_teams() {
        let mut mm = Matchmaker::new(2, 1, 50);

        mm.enqueue(party(1, vec![player(1, 1000)]));
        mm.enqueue(party(2, vec![player(2, 3000)]));

        let matches: Vec<Match> = mm.tick().unwrap().into_iter().collect();

        assert!(matches.is_empty());
    }

    #[test]
    fn incompatible_elo_does_not_merge_partial_teams() {
        let mut mm = Matchmaker::new(2, 2, 50);

        // Both parties need one more player to complete a team, but their
        // elo is too far apart to be merged into the same team.
        mm.enqueue(party(1, vec![player(1, 1000)]));
        mm.enqueue(party(2, vec![player(2, 3000)]));

        let matches = mm.tick().unwrap().into_iter().count();

        assert_eq!(matches, 0);
        assert_eq!(mm.queued_players(), 0);
    }

    fn matched_player_ids(m: &Match) -> Vec<PlayerId> {
        m.teams
            .iter()
            .flat_map(|t| t.players.iter().map(|p| p.name))
            .collect()
    }

    #[test]
    fn tick_matches_compatible_completed_teams_and_leaves_incompatible_one_queued() {
        let mut mm = Matchmaker::new(2, 1, 50);

        mm.enqueue(party(1, vec![player(1, 1000)]));
        mm.enqueue(party(2, vec![player(2, 1010)]));
        mm.enqueue(party(3, vec![player(3, 3000)]));

        let matches: Vec<Match> = mm.tick().unwrap().into_iter().collect();

        assert_eq!(matches.len(), 1);
        assert_eq!(matched_player_ids(&matches[0]), vec![1, 2]);
    }

    #[test]
    fn unmatched_completed_team_persists_for_a_later_tick() {
        let mut mm = Matchmaker::new(2, 1, 50);

        mm.enqueue(party(1, vec![player(1, 1000)]));
        mm.enqueue(party(2, vec![player(2, 1010)]));
        mm.enqueue(party(3, vec![player(3, 3000)]));

        // Player 3's team has no compatible partner yet, so it should stay
        // queued rather than being dropped once the first match is formed.
        let _ = mm.tick().unwrap().into_iter().count();

        mm.enqueue(party(4, vec![player(4, 3010)]));
        let matches: Vec<Match> = mm.tick().unwrap().into_iter().collect();

        assert_eq!(matches.len(), 1);
        assert_eq!(matched_player_ids(&matches[0]), vec![3, 4]);
    }
}
