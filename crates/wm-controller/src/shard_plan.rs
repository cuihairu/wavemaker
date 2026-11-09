//! ShardPlan (WM-0504) — the controller's frozen, deterministic
//! execution plan (ADR-006).
//!
//! D1: the controller is the ONLY plan generator. Plans travel as data
//! (`PlanEntry` sequences); Workers/Backends consume them and never
//! recompute arrival plans from `(seed, lambda, W, i)`.
//!
//! D2: PlanEntry carries exactly the frozen field set —
//! `actor_or_bot_id / backend_id / shard_id / planned_at / lifetime /
//! behavior_group / seed / attributes`. Lifetime values are INPUTS
//! here: drawing from a Spectrum `lifetime.distribution` is the
//! generator's job; freezing assignment is this module's.
//!
//! D3 honesty rules made structural:
//! - a plan is frozen at Swell creation and immutable — no API can
//!   silently change the shard rule or W mid-execution (D3-3);
//! - `plan_digest` (sha256 over the canonical entry serialization)
//!   is computed at build time and verifiable on receipt (D3-2);
//! - worker loss is recovered only through an EXPLICIT reallocation
//!   that keeps W and the rule, moves as little as possible, never
//!   duplicates an actor/bot id, and returns an audit record of every
//!   move (D3-4);
//! - the `modulo` (`i mod W`) rule is RECORDED as a candidate, not
//!   presented as proven (D3-6);
//! - weights are indicative only (ADR-005 §4.4): they enter the
//!   allocation as a deterministic proportional split (largest
//!   remainder, ties by sorted backend id), never as runtime randomness.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// One planned arrival, before backend assignment. `lifetime_ms` is a
/// CONCRETE value already drawn from the Spectrum distribution — the
/// plan builder draws nothing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlannedArrival {
    pub planned_at_unix_ms: i64,
    pub lifetime_ms: u64,
    pub behavior_group: String,
}

/// ADR-006 D2 field set — exactly these, nothing hidden.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanEntry {
    /// Actor (Native Runtime) or Bot (Engine Bot Backend) identifier,
    /// minted deterministically by the controller.
    pub actor_or_bot_id: String,
    /// Backend carrying this entry.
    pub backend_id: String,
    /// Shard this entry belongs to.
    pub shard_id: u32,
    /// Planned arrival/creation time.
    pub planned_at_unix_ms: i64,
    /// Concrete lifetime value (input, not re-drawn).
    pub lifetime_ms: u64,
    /// Behavior group reference.
    pub behavior_group: String,
    /// Deterministic per-entry random source seed.
    pub seed: u64,
    /// Extension attributes, verbatim.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attributes: BTreeMap<String, String>,
}

/// The shard rule, recorded IN the plan so the assignment is
/// auditable and rule changes can never be silent (D3-3/D3-6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShardRule {
    /// `shard = arrival_index mod W`. ADR-006 D3-6: kept as a
    /// CANDIDATE pending validation — conformance-tested for
    /// determinism here, not claimed optimal.
    Modulo,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShardPlan {
    pub swell_id: String,
    /// W — frozen at Swell creation (WM-0505 freezes the Worker count
    /// from exactly this value).
    pub worker_count: u32,
    pub rule: ShardRule,
    /// Entries in arrival order.
    pub entries: Vec<PlanEntry>,
    /// sha256 hex over the canonical JSON of `entries`.
    pub plan_digest: String,
}

/// One backend available to a swell: identity, kind, and its
/// indicative weight (ADR-005 §4.4).
#[derive(Debug, Clone, PartialEq)]
pub struct PlanBackend {
    pub backend_id: String,
    pub kind: crate::backend_registry::BackendKind,
    pub weight: f64,
}

/// Explicit reallocation result (D3-4): what moved, from where, to
/// where — nothing silent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplanRecord {
    pub previous_digest: String,
    pub new_digest: String,
    pub moved: Vec<MovedEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MovedEntry {
    pub actor_or_bot_id: String,
    pub from_backend: String,
    pub to_backend: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PlanError {
    #[error("swell id must not be empty")]
    EmptySwellId,
    #[error("worker count must be at least 1")]
    ZeroWorkers,
    #[error("at least one backend is required")]
    NoBackends,
    #[error("at least one surviving backend is required")]
    NoSurvivingBackends,
    #[error("at least one planned arrival is required")]
    EmptyArrivals,
    #[error("backend ids must be unique")]
    DuplicateBackend,
    #[error("all indicative weights are non-positive; nothing is allocatable")]
    IndicativeWeightsDepleted,
}

fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Deterministic per-entry seed: a pure function of (swell_id, arrival
/// index). Same input, same seed — no entropy anywhere.
fn entry_seed(swell_id: &str, index: usize) -> u64 {
    let fingerprint = u64::from_le_bytes(
        Sha256::digest(swell_id.as_bytes())[..8]
            .try_into()
            .expect("sha256 is at least 8 bytes"),
    );
    let mut state = fingerprint ^ (index as u64).wrapping_mul(0xD1B5_4A32_D192_ED03);
    splitmix64(&mut state)
}

/// Deterministic proportional distribution of `total` units over
/// backends by indicative weight — largest remainder, ties by sorted
/// backend id. Weight ≤ 0 receives nothing. Returns one count PER
/// INPUT BACKEND, in sorted-backend-id order (so callers can align
/// counts with the same sorted list). Pure: same inputs, same output.
fn allocate_shards(backends: &[PlanBackend], total: u32) -> Result<Vec<u32>, PlanError> {
    let mut sorted: Vec<&PlanBackend> = backends.iter().collect();
    sorted.sort_by(|a, b| a.backend_id.cmp(&b.backend_id));
    let positive: Vec<&PlanBackend> = sorted
        .iter()
        .copied()
        .filter(|backend| backend.weight > 0.0)
        .collect();
    if positive.is_empty() {
        return Err(PlanError::IndicativeWeightsDepleted);
    }
    let total_weight: f64 = positive.iter().map(|backend| backend.weight).sum();
    let total_f = f64::from(total);
    // Exact quotas, floored; zero-weight backends get an explicit 0.
    let mut counts: Vec<(String, u32, f64)> = sorted
        .iter()
        .map(|backend| {
            if backend.weight <= 0.0 {
                (backend.backend_id.clone(), 0, 0.0)
            } else {
                let quota = total_f * backend.weight / total_weight;
                (backend.backend_id.clone(), quota.floor() as u32, quota.fract())
            }
        })
        .collect();
    let assigned: u32 = counts.iter().map(|(_, count, _)| *count).sum();
    // Remaining units go to the largest fractional remainders; exact
    // ties break by the sorted backend id order `counts` already has.
    let mut remaining = total - assigned;
    let mut order: Vec<usize> = (0..counts.len()).collect();
    order.sort_by(|a, b| {
        counts[*b]
            .2
            .partial_cmp(&counts[*a].2)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    for index in order {
        if remaining == 0 {
            break;
        }
        counts[index].1 += 1;
        remaining -= 1;
    }
    Ok(counts.into_iter().map(|(_, count, _)| count).collect())
}

/// sha256 hex over the canonical JSON serialization of the entries
/// (Vec serialization preserves order, serde field order is struct
/// declaration order — both stable).
fn digest_entries(entries: &[PlanEntry]) -> String {
    let canonical = serde_json::to_string(entries).expect("entries always serialize");
    hex::encode(Sha256::digest(canonical.as_bytes()))
}

/// Build the frozen ShardPlan (D3-1). Deterministic: identical inputs
/// produce byte-identical plans, digest included (D4-1).
pub fn build_shard_plan(
    swell_id: &str,
    worker_count: u32,
    backends: &[PlanBackend],
    arrivals: &[PlannedArrival],
) -> Result<ShardPlan, PlanError> {
    if swell_id.trim().is_empty() {
        return Err(PlanError::EmptySwellId);
    }
    if worker_count == 0 {
        return Err(PlanError::ZeroWorkers);
    }
    if backends.is_empty() {
        return Err(PlanError::NoBackends);
    }
    let mut ids: Vec<&str> = backends.iter().map(|backend| backend.backend_id.as_str()).collect();
    ids.sort_unstable();
    if ids.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(PlanError::DuplicateBackend);
    }
    if arrivals.is_empty() {
        return Err(PlanError::EmptyArrivals);
    }
    let shard_counts = allocate_shards(backends, worker_count)?;

    // Shard s → owning backend, deterministically: shards are handed
    // out in backend-id-sorted order, proportionally to the indicative
    // weights.
    let mut sorted: Vec<&PlanBackend> = backends.iter().collect();
    sorted.sort_by(|a, b| a.backend_id.cmp(&b.backend_id));
    let mut shard_owner: Vec<&PlanBackend> = Vec::with_capacity(worker_count as usize);
    for (position, count) in shard_counts.iter().enumerate() {
        for _ in 0..*count {
            shard_owner.push(sorted[position]);
        }
    }

    let mut entries = Vec::with_capacity(arrivals.len());
    for (index, arrival) in arrivals.iter().enumerate() {
        let shard_id = (index % worker_count as usize) as u32;
        let owner = shard_owner[shard_id as usize];
        // The controller mints the id: prefix follows the assigned
        // backend kind (actors on Native Runtime, bots on Engine Bot).
        let actor_or_bot_id = match owner.kind {
            crate::backend_registry::BackendKind::NativeRuntime => format!("actor-{index}"),
            crate::backend_registry::BackendKind::EngineBot => format!("bot-{index}"),
        };
        entries.push(PlanEntry {
            actor_or_bot_id,
            backend_id: owner.backend_id.clone(),
            shard_id,
            planned_at_unix_ms: arrival.planned_at_unix_ms,
            lifetime_ms: arrival.lifetime_ms,
            behavior_group: arrival.behavior_group.clone(),
            seed: entry_seed(swell_id, index),
            attributes: BTreeMap::new(),
        });
    }

    let plan_digest = digest_entries(&entries);
    Ok(ShardPlan {
        swell_id: swell_id.into(),
        worker_count,
        rule: ShardRule::Modulo,
        entries,
        plan_digest,
    })
}

impl ShardPlan {
    /// D3-2: verify the digest on receipt. `false` means the entry
    /// data does not match the recorded digest — refuse, never guess.
    pub fn verify_digest(&self) -> bool {
        digest_entries(&self.entries) == self.plan_digest
    }

    /// D3-4: explicit reallocation after worker loss. Entries on
    /// surviving backends keep them (minimal movement); entries on
    /// lost backends move to their shard's new deterministic owner.
    /// W, rule, shard ids, ids, seeds and arrival times are all
    /// UNCHANGED — only `backend_id` may move, and every move is
    /// listed in the returned record. Actor/bot ids stay unique.
    pub fn replan_on_loss(
        &self,
        surviving: &[PlanBackend],
    ) -> Result<(ShardPlan, ReplanRecord), PlanError> {
        if surviving.is_empty() {
            return Err(PlanError::NoSurvivingBackends);
        }
        let mut ids: Vec<&str> = surviving
            .iter()
            .map(|backend| backend.backend_id.as_str())
            .collect();
        ids.sort_unstable();
        if ids.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(PlanError::DuplicateBackend);
        }
        let lost: Vec<&str> = self
            .entries
            .iter()
            .map(|entry| entry.backend_id.as_str())
            .filter(|backend| !ids.contains(backend))
            .collect();
        if lost.is_empty() {
            // Nothing to move: same plan content, same digest, an
            // empty record — the honest "no reallocation happened".
            return Ok((
                self.clone(),
                ReplanRecord {
                    previous_digest: self.plan_digest.clone(),
                    new_digest: self.plan_digest.clone(),
                    moved: Vec::new(),
                },
            ));
        }
        let shard_counts = allocate_shards(surviving, self.worker_count)?;
        let mut sorted: Vec<&PlanBackend> = surviving.iter().collect();
        sorted.sort_by(|a, b| a.backend_id.cmp(&b.backend_id));
        let mut shard_owner: Vec<&str> = Vec::with_capacity(self.worker_count as usize);
        for (position, count) in shard_counts.iter().enumerate() {
            for _ in 0..*count {
                shard_owner.push(sorted[position].backend_id.as_str());
            }
        }
        let mut moved = Vec::new();
        let mut entries = self.entries.clone();
        for entry in &mut entries {
            if ids.contains(&entry.backend_id.as_str()) {
                continue;
            }
            let to = shard_owner[entry.shard_id as usize].to_string();
            moved.push(MovedEntry {
                actor_or_bot_id: entry.actor_or_bot_id.clone(),
                from_backend: entry.backend_id.clone(),
                to_backend: to.clone(),
            });
            entry.backend_id = to;
        }
        let new_digest = digest_entries(&entries);
        let mut plan = self.clone();
        plan.entries = entries;
        plan.plan_digest = new_digest.clone();
        let record = ReplanRecord {
            previous_digest: self.plan_digest.clone(),
            new_digest,
            moved,
        };
        Ok((plan, record))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend_registry::BackendKind;

    fn native(id: &str, weight: f64) -> PlanBackend {
        PlanBackend {
            backend_id: id.into(),
            kind: BackendKind::NativeRuntime,
            weight,
        }
    }

    fn bot(id: &str, weight: f64) -> PlanBackend {
        PlanBackend {
            backend_id: id.into(),
            kind: BackendKind::EngineBot,
            weight,
        }
    }

    fn arrivals(count: usize) -> Vec<PlannedArrival> {
        (0..count)
            .map(|i| PlannedArrival {
                planned_at_unix_ms: 1_700_000_000_000 + i as i64 * 100,
                lifetime_ms: 30_000 + i as u64,
                behavior_group: "login-flow".into(),
            })
            .collect()
    }

    fn two_backends() -> Vec<PlanBackend> {
        vec![native("b-native", 1.0), bot("a-bot", 2.0)]
    }

    #[test]
    fn identical_inputs_produce_byte_identical_plans() {
        let first = build_shard_plan("sw-1", 6, &two_backends(), &arrivals(10))
            .expect("plan");
        let second = build_shard_plan("sw-1", 6, &two_backends(), &arrivals(10))
            .expect("plan");
        assert_eq!(first, second, "D4-1 plan determinism: same input, same plan");
        let text = serde_json::to_string(&first).expect("serialize");
        let round: ShardPlan = serde_json::from_str(&text).expect("deserialize");
        assert_eq!(round, first);
        assert!(round.verify_digest());
    }

    #[test]
    fn entries_carry_the_full_d2_field_set() {
        let plan = build_shard_plan("sw-1", 4, &two_backends(), &arrivals(3))
            .expect("plan");
        let entry = &plan.entries[0];
        assert!(!entry.actor_or_bot_id.is_empty());
        assert!(!entry.backend_id.is_empty());
        assert_eq!(entry.shard_id, 0);
        assert_eq!(entry.planned_at_unix_ms, 1_700_000_000_000);
        assert_eq!(entry.lifetime_ms, 30_000, "lifetime is the given input");
        assert_eq!(entry.behavior_group, "login-flow");
        assert_ne!(entry.seed, 0);
        assert!(entry.attributes.is_empty());
        // Seeds are per-entry distinct and deterministic in the swell.
        let seeds: Vec<u64> = plan.entries.iter().map(|entry| entry.seed).collect();
        assert_eq!(seeds.len(), 3);
        assert!(seeds.windows(2).all(|pair| pair[0] != pair[1]));
        let other = build_shard_plan("sw-2", 4, &two_backends(), &arrivals(3))
            .expect("plan");
        assert_ne!(
            plan.entries[0].seed, other.entries[0].seed,
            "seeds are swell-scoped, not global constants"
        );
    }

    #[test]
    fn modulo_rule_and_proportional_allocation_are_deterministic() {
        // Weights 2:1 over W=6 → a-bot 4 shards, b-native 2 shards
        // (largest-remainder with ties by sorted backend id).
        let plan = build_shard_plan("sw-1", 6, &two_backends(), &arrivals(12))
            .expect("plan");
        assert_eq!(plan.rule, ShardRule::Modulo);
        assert_eq!(plan.worker_count, 6);
        for (index, entry) in plan.entries.iter().enumerate() {
            assert_eq!(entry.shard_id as usize, index % 6, "shard = i mod W");
        }
        let on_bot = plan
            .entries
            .iter()
            .filter(|entry| entry.backend_id == "a-bot")
            .count();
        let on_native = plan
            .entries
            .iter()
            .filter(|entry| entry.backend_id == "b-native")
            .count();
        assert_eq!(on_bot, 8, "4 shards × arrivals per shard");
        assert_eq!(on_native, 4, "2 shards × arrivals per shard");
        // Exact quotas (no remainder): a-bot owns shards {0,1,2,3},
        // b-native {4,5}.
        assert_eq!(plan.entries[3].backend_id, "a-bot");
        assert_eq!(plan.entries[4].backend_id, "b-native");
        assert_eq!(plan.entries[5].backend_id, "b-native");
    }

    #[test]
    fn non_positive_weights_receive_nothing_and_depletion_is_named() {
        // A zero-weight backend is registered but gets no shards.
        let backends = vec![native("idle", 0.0), bot("worker", 1.0)];
        let plan = build_shard_plan("sw-1", 4, &backends, &arrivals(4)).expect("plan");
        assert!(plan
            .entries
            .iter()
            .all(|entry| entry.backend_id == "worker"));
        // All weights non-positive → nothing allocatable, named error.
        let backends = vec![native("a", 0.0), bot("b", -1.0)];
        assert_eq!(
            build_shard_plan("sw-1", 4, &backends, &arrivals(4)),
            Err(PlanError::IndicativeWeightsDepleted)
        );
    }

    #[test]
    fn id_prefix_follows_the_assigned_backend_kind() {
        let backends = vec![native("nat", 1.0)];
        let plan = build_shard_plan("sw-1", 2, &backends, &arrivals(2)).expect("plan");
        assert!(plan
            .entries
            .iter()
            .all(|entry| entry.actor_or_bot_id.starts_with("actor-")));
        let backends = vec![bot("eng", 1.0)];
        let plan = build_shard_plan("sw-1", 2, &backends, &arrivals(2)).expect("plan");
        assert!(plan
            .entries
            .iter()
            .all(|entry| entry.actor_or_bot_id.starts_with("bot-")));
    }

    #[test]
    fn tampered_entries_fail_digest_verification() {
        let mut plan = build_shard_plan("sw-1", 4, &two_backends(), &arrivals(4))
            .expect("plan");
        assert!(plan.verify_digest());
        plan.entries[0].planned_at_unix_ms += 1;
        assert!(!plan.verify_digest(), "edited plan data must not verify");
    }

    #[test]
    fn builder_validation_errors_are_named() {
        let backends = two_backends();
        assert_eq!(
            build_shard_plan("  ", 4, &backends, &arrivals(4)),
            Err(PlanError::EmptySwellId)
        );
        assert_eq!(
            build_shard_plan("sw-1", 0, &backends, &arrivals(4)),
            Err(PlanError::ZeroWorkers)
        );
        assert_eq!(
            build_shard_plan("sw-1", 4, &[], &arrivals(4)),
            Err(PlanError::NoBackends)
        );
        assert_eq!(
            build_shard_plan("sw-1", 4, &backends, &[]),
            Err(PlanError::EmptyArrivals)
        );
        let dup = vec![native("same", 1.0), native("same", 1.0)];
        assert_eq!(
            build_shard_plan("sw-1", 4, &dup, &arrivals(4)),
            Err(PlanError::DuplicateBackend)
        );
    }

    #[test]
    fn replan_moves_only_lost_entries_and_keeps_everything_else() {
        let plan = build_shard_plan("sw-1", 6, &two_backends(), &arrivals(12))
            .expect("plan");
        let survivor = vec![native("b-native", 1.0)];
        let (replanned, record) = plan.replan_on_loss(&survivor).expect("replan");
        // W, rule, shard ids, ids, seeds, arrival times all unchanged.
        assert_eq!(replanned.worker_count, plan.worker_count);
        assert_eq!(replanned.rule, plan.rule);
        for (old, new) in plan.entries.iter().zip(&replanned.entries) {
            assert_eq!(old.actor_or_bot_id, new.actor_or_bot_id);
            assert_eq!(old.shard_id, new.shard_id);
            assert_eq!(old.seed, new.seed);
            assert_eq!(old.planned_at_unix_ms, new.planned_at_unix_ms);
        }
        // Minimal movement: entries already on b-native stay put.
        for (old, new) in plan.entries.iter().zip(&replanned.entries) {
            if old.backend_id == "b-native" {
                assert_eq!(new.backend_id, "b-native", "survivor keeps its entries");
            } else {
                assert_eq!(new.backend_id, "b-native", "lost entries move");
            }
        }
        // Every move is in the record; nothing silent.
        let moved_ids: Vec<&str> = record
            .moved
            .iter()
            .map(|moved| moved.actor_or_bot_id.as_str())
            .collect();
        assert_eq!(moved_ids.len(), 8, "all bot-backend entries moved");
        for entry in &plan.entries {
            if entry.backend_id == "a-bot" {
                assert!(moved_ids.contains(&entry.actor_or_bot_id.as_str()));
            }
        }
        assert!(record.moved.iter().all(|moved| moved.to_backend == "b-native"));
        // Digest changed and verifies; ids stay unique (dedup).
        assert_ne!(record.previous_digest, record.new_digest);
        assert_eq!(record.previous_digest, plan.plan_digest);
        assert_eq!(record.new_digest, replanned.plan_digest);
        assert!(replanned.verify_digest());
        let mut ids: Vec<&str> = replanned
            .entries
            .iter()
            .map(|entry| entry.actor_or_bot_id.as_str())
            .collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), replanned.entries.len(), "no duplicated ids");
    }

    #[test]
    fn replan_without_loss_is_an_honest_no_op() {
        let plan = build_shard_plan("sw-1", 4, &two_backends(), &arrivals(8))
            .expect("plan");
        let (replanned, record) = plan.replan_on_loss(&two_backends()).expect("replan");
        assert_eq!(replanned, plan, "nothing lost, nothing moved");
        assert!(record.moved.is_empty());
        assert_eq!(record.previous_digest, record.new_digest);
        // No survivors at all is refused, not guessed.
        assert_eq!(
            plan.replan_on_loss(&[]),
            Err(PlanError::NoSurvivingBackends)
        );
    }
}
