#![cfg(test)]

use super::*;
use soroban_sdk::{
    testutils::{Address as AddressTrait, Events},
    Address, Env, String,
};
use testutils::set_ledger_time;

fn setup() -> (Env, InsuranceClient<'static>, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register_contract(None, Insurance);
    let client = InsuranceClient::new(&env, &contract_id);
    let owner = Address::generate(&env);
    (env, client, owner)
}

fn make_policy(env: &Env, client: &InsuranceClient, owner: &Address) -> u32 {
    client.create_policy(
        owner,
        &String::from_str(env, "Test Policy"),
        &CoverageType::Health,
        &1000,
        &10000,
    )
}

// ── create_policy ─────────────────────────────────────────────────────────────

#[test]
fn test_create_policy_succeeds() {
    let (env, client, owner) = setup();
    let id = make_policy(&env, &client, &owner);
    assert_eq!(id, 1);
    let p = client.get_policy(&id).unwrap();
    assert_eq!(p.owner, owner);
    assert_eq!(p.monthly_premium, 1000);
    assert_eq!(p.coverage_amount, 10000);
    assert!(p.active);
    assert_eq!(p.tags.len(), 0);
}

#[test]
#[should_panic(expected = "Monthly premium must be positive")]
fn test_create_policy_invalid_premium() {
    let (env, client, owner) = setup();
    client.create_policy(&owner, &String::from_str(&env, "Bad"), &CoverageType::Health, &0, &10000);
}

#[test]
#[should_panic(expected = "Coverage amount must be positive")]
fn test_create_policy_invalid_coverage() {
    let (env, client, owner) = setup();
    client.create_policy(&owner, &String::from_str(&env, "Bad"), &CoverageType::Health, &100, &0);
}

// ── pay_premium ───────────────────────────────────────────────────────────────

#[test]
fn test_pay_premium_updates_date() {
    let (env, client, owner) = setup();
    let id = make_policy(&env, &client, &owner);
    let before = client.get_policy(&id).unwrap().next_payment_date;
    set_ledger_time(&env, 1, env.ledger().timestamp() + 1000);
    client.pay_premium(&owner, &id);
    let after = client.get_policy(&id).unwrap().next_payment_date;
    assert!(after > before);
}

#[test]
fn test_pay_premium_unauthorized() {
    let (env, client, owner) = setup();
    let id = make_policy(&env, &client, &owner);
    let other = Address::generate(&env);
    let result = client.try_pay_premium(&other, &id);
    assert_eq!(result, Err(Ok(InsuranceError::Unauthorized)));
}

#[test]
fn test_pay_premium_inactive_policy() {
    let (env, client, owner) = setup();
    let id = make_policy(&env, &client, &owner);
    client.deactivate_policy(&owner, &id);
    let result = client.try_pay_premium(&owner, &id);
    assert_eq!(result, Err(Ok(InsuranceError::PolicyInactive)));
}

// ── deactivate_policy ─────────────────────────────────────────────────────────

#[test]
fn test_deactivate_policy() {
    let (env, client, owner) = setup();
    let id = make_policy(&env, &client, &owner);
    assert!(client.deactivate_policy(&owner, &id));
    assert!(!client.get_policy(&id).unwrap().active);
}

#[test]
fn test_deactivate_policy_unauthorized() {
    let (env, client, owner) = setup();
    let id = make_policy(&env, &client, &owner);
    let other = Address::generate(&env);
    let result = client.try_deactivate_policy(&other, &id);
    assert_eq!(result, Err(Ok(InsuranceError::Unauthorized)));
}

// ── get_active_policies / get_total_monthly_premium ───────────────────────────

#[test]
fn test_get_active_policies_filters_inactive() {
    let (env, client, owner) = setup();
    let p1 = make_policy(&env, &client, &owner);
    make_policy(&env, &client, &owner);
    client.deactivate_policy(&owner, &p1);
    let active = client.get_active_policies(&owner);
    assert_eq!(active.len(), 1);
    assert!(active.get(0).unwrap().active);
}

#[test]
fn test_get_total_monthly_premium() {
    let (env, client, owner) = setup();
    client.create_policy(&owner, &String::from_str(&env, "P1"), &CoverageType::Health, &100, &1000);
    client.create_policy(&owner, &String::from_str(&env, "P2"), &CoverageType::Health, &200, &2000);
    assert_eq!(client.get_total_monthly_premium(&owner), 300);
}

#[test]
fn test_get_total_monthly_premium_excludes_inactive() {
    let (env, client, owner) = setup();
    let p1 = client.create_policy(&owner, &String::from_str(&env, "P1"), &CoverageType::Health, &100, &1000);
    client.create_policy(&owner, &String::from_str(&env, "P2"), &CoverageType::Health, &200, &2000);
    client.deactivate_policy(&owner, &p1);
    assert_eq!(client.get_total_monthly_premium(&owner), 200);
}

// ── add_tag: authorization ────────────────────────────────────────────────────

/// Policy owner can add a tag.
#[test]
fn test_add_tag_by_owner_succeeds() {
    let (env, client, owner) = setup();
    let id = make_policy(&env, &client, &owner);
    client.add_tag(&owner, &id, &String::from_str(&env, "vip"));
    let tags = client.get_policy(&id).unwrap().tags;
    assert_eq!(tags.len(), 1);
    assert_eq!(tags.get(0).unwrap(), String::from_str(&env, "vip"));
}

/// Admin can add a tag to any policy.
#[test]
fn test_add_tag_by_admin_succeeds() {
    let (env, client, owner) = setup();
    let admin = Address::generate(&env);
    client.set_admin(&admin, &admin);
    let id = make_policy(&env, &client, &owner);
    client.add_tag(&admin, &id, &String::from_str(&env, "admin-tag"));
    let tags = client.get_policy(&id).unwrap().tags;
    assert_eq!(tags.len(), 1);
}

/// A third party that is neither owner nor admin must be rejected.
#[test]
#[should_panic(expected = "unauthorized")]
fn test_add_tag_by_stranger_panics() {
    let (env, client, owner) = setup();
    let id = make_policy(&env, &client, &owner);
    let stranger = Address::generate(&env);
    client.add_tag(&stranger, &id, &String::from_str(&env, "hack"));
}

/// Missing auth must fail.
#[test]
fn test_add_tag_requires_auth() {
    let env = Env::default(); // no mock_all_auths
    let contract_id = env.register_contract(None, Insurance);
    let client = InsuranceClient::new(&env, &contract_id);
    let owner = Address::generate(&env);
    // try_add_tag without any auth mock — must fail
    let result = client.try_add_tag(
        &owner,
        &1u32,
        &String::from_str(&env, "tag"),
    );
    assert!(result.is_err());
}

// ── add_tag: deduplication ────────────────────────────────────────────────────

/// Adding the same tag twice must result in exactly one entry.
#[test]
fn test_add_tag_deduplication() {
    let (env, client, owner) = setup();
    let id = make_policy(&env, &client, &owner);
    let tag = String::from_str(&env, "priority");
    client.add_tag(&owner, &id, &tag);
    client.add_tag(&owner, &id, &tag); // duplicate
    assert_eq!(client.get_policy(&id).unwrap().tags.len(), 1);
}

/// Multiple distinct tags all get stored.
#[test]
fn test_add_multiple_distinct_tags() {
    let (env, client, owner) = setup();
    let id = make_policy(&env, &client, &owner);
    client.add_tag(&owner, &id, &String::from_str(&env, "a"));
    client.add_tag(&owner, &id, &String::from_str(&env, "b"));
    client.add_tag(&owner, &id, &String::from_str(&env, "c"));
    assert_eq!(client.get_policy(&id).unwrap().tags.len(), 3);
}

/// Tags on one policy must not appear on another.
#[test]
fn test_tags_isolated_per_policy() {
    let (env, client, owner) = setup();
    let p1 = make_policy(&env, &client, &owner);
    let p2 = make_policy(&env, &client, &owner);
    client.add_tag(&owner, &p1, &String::from_str(&env, "exclusive"));
    assert_eq!(client.get_policy(&p2).unwrap().tags.len(), 0);
}

// ── add_tag: events ───────────────────────────────────────────────────────────

/// add_tag must emit a tag_added event.
#[test]
fn test_add_tag_emits_event() {
    let (env, client, owner) = setup();
    let id = make_policy(&env, &client, &owner);
    let before = env.events().all().len();
    client.add_tag(&owner, &id, &String::from_str(&env, "vip"));
    assert!(env.events().all().len() > before);
}

/// Duplicate add must NOT emit a tag_added event (nothing changed).
#[test]
fn test_add_tag_duplicate_no_event() {
    let (env, client, owner) = setup();
    let id = make_policy(&env, &client, &owner);
    let tag = String::from_str(&env, "vip");
    client.add_tag(&owner, &id, &tag);
    let before = env.events().all().len();
    client.add_tag(&owner, &id, &tag); // duplicate — should be silent
    assert_eq!(env.events().all().len(), before);
}

// ── remove_tag: happy path ────────────────────────────────────────────────────

/// Removing an existing tag works and leaves the rest intact.
#[test]
fn test_remove_tag_removes_correct_tag() {
    let (env, client, owner) = setup();
    let id = make_policy(&env, &client, &owner);
    client.add_tag(&owner, &id, &String::from_str(&env, "keep"));
    client.add_tag(&owner, &id, &String::from_str(&env, "remove_me"));
    client.remove_tag(&owner, &id, &String::from_str(&env, "remove_me"));
    let tags = client.get_policy(&id).unwrap().tags;
    assert_eq!(tags.len(), 1);
    assert_eq!(tags.get(0).unwrap(), String::from_str(&env, "keep"));
}

/// Removing all tags results in an empty list.
#[test]
fn test_remove_all_tags() {
    let (env, client, owner) = setup();
    let id = make_policy(&env, &client, &owner);
    client.add_tag(&owner, &id, &String::from_str(&env, "a"));
    client.add_tag(&owner, &id, &String::from_str(&env, "b"));
    client.remove_tag(&owner, &id, &String::from_str(&env, "a"));
    client.remove_tag(&owner, &id, &String::from_str(&env, "b"));
    assert_eq!(client.get_policy(&id).unwrap().tags.len(), 0);
}

// ── remove_tag: graceful on missing ──────────────────────────────────────────

/// Removing a tag that was never added must NOT panic.
#[test]
fn test_remove_nonexistent_tag_is_noop() {
    let (env, client, owner) = setup();
    let id = make_policy(&env, &client, &owner);
    // no tags added — should return gracefully
    client.remove_tag(&owner, &id, &String::from_str(&env, "ghost"));
    assert_eq!(client.get_policy(&id).unwrap().tags.len(), 0);
}

/// Removing a missing tag emits a "tag_no_tag" (Tag Not Found) event.
#[test]
fn test_remove_nonexistent_tag_emits_not_found_event() {
    let (env, client, owner) = setup();
    let id = make_policy(&env, &client, &owner);
    let before = env.events().all().len();
    client.remove_tag(&owner, &id, &String::from_str(&env, "ghost"));
    // must have emitted exactly one new event (tag_no_tag)
    assert_eq!(env.events().all().len(), before + 1);
}

// ── remove_tag: authorization ─────────────────────────────────────────────────

/// A stranger cannot remove tags.
#[test]
#[should_panic(expected = "unauthorized")]
fn test_remove_tag_by_stranger_panics() {
    let (env, client, owner) = setup();
    let id = make_policy(&env, &client, &owner);
    client.add_tag(&owner, &id, &String::from_str(&env, "vip"));
    let stranger = Address::generate(&env);
    client.remove_tag(&stranger, &id, &String::from_str(&env, "vip"));
}

/// Admin can remove tags from any policy.
#[test]
fn test_remove_tag_by_admin_succeeds() {
    let (env, client, owner) = setup();
    let admin = Address::generate(&env);
    client.set_admin(&admin, &admin);
    let id = make_policy(&env, &client, &owner);
    client.add_tag(&owner, &id, &String::from_str(&env, "vip"));
    client.remove_tag(&admin, &id, &String::from_str(&env, "vip"));
    assert_eq!(client.get_policy(&id).unwrap().tags.len(), 0);
}

// ── remove_tag: events ────────────────────────────────────────────────────────

/// Successful remove emits tag_removed event.
#[test]
fn test_remove_tag_emits_event() {
    let (env, client, owner) = setup();
    let id = make_policy(&env, &client, &owner);
    client.add_tag(&owner, &id, &String::from_str(&env, "vip"));
    let before = env.events().all().len();
    client.remove_tag(&owner, &id, &String::from_str(&env, "vip"));
    assert!(env.events().all().len() > before);
}

// ═══════════════════════════════════════════════════════════════════════════════
// QA: Exhaustive tagging tests — unauthorized access, double-tag, ghost remove,
//     and full event verification.
// ═══════════════════════════════════════════════════════════════════════════════

// ── 1. Unauthorized Access ────────────────────────────────────────────────────

/// A random address that is neither the policy owner nor the admin must cause
/// add_tag to panic with "unauthorized". State must be unchanged.
#[test]
#[should_panic(expected = "unauthorized")]
fn test_qa_unauthorized_stranger_cannot_add_tag() {
    let (env, client, owner) = setup();
    let id = make_policy(&env, &client, &owner);
    let random = Address::generate(&env);
    // random is not owner, no admin set — must panic
    client.add_tag(&random, &id, &String::from_str(&env, "ACTIVE"));
}

/// A random address must also be blocked from remove_tag.
#[test]
#[should_panic(expected = "unauthorized")]
fn test_qa_unauthorized_stranger_cannot_remove_tag() {
    let (env, client, owner) = setup();
    let id = make_policy(&env, &client, &owner);
    client.add_tag(&owner, &id, &String::from_str(&env, "ACTIVE"));
    let random = Address::generate(&env);
    client.remove_tag(&random, &id, &String::from_str(&env, "ACTIVE"));
}

/// After a failed unauthorized add_tag, the policy tags must remain empty —
/// no partial state mutation.
#[test]
fn test_qa_unauthorized_add_leaves_state_unchanged() {
    let (env, client, owner) = setup();
    let id = make_policy(&env, &client, &owner);
    let random = Address::generate(&env);

    // attempt unauthorized add — ignore the panic via try_
    let _ = client.try_add_tag(&random, &id, &String::from_str(&env, "ACTIVE"));

    // state must be untouched
    assert_eq!(
        client.get_policy(&id).unwrap().tags.len(),
        0,
        "unauthorized call must not mutate policy tags"
    );
}

// ── 2. The Double-Tag ─────────────────────────────────────────────────────────

/// Adding "ACTIVE" twice must leave exactly one "ACTIVE" tag in storage.
#[test]
fn test_qa_double_tag_active_stored_once() {
    let (env, client, owner) = setup();
    let id = make_policy(&env, &client, &owner);
    let active = String::from_str(&env, "ACTIVE");

    client.add_tag(&owner, &id, &active);
    client.add_tag(&owner, &id, &active); // duplicate

    let tags = client.get_policy(&id).unwrap().tags;
    assert_eq!(tags.len(), 1, "duplicate tag must not be stored twice");
    assert_eq!(
        tags.get(0).unwrap(),
        String::from_str(&env, "ACTIVE"),
        "the stored tag must be ACTIVE"
    );
}

/// The second (duplicate) add_tag call must emit NO new event — the contract
/// returns early before publishing.
#[test]
fn test_qa_double_tag_second_call_emits_no_event() {
    let (env, client, owner) = setup();
    let id = make_policy(&env, &client, &owner);
    let active = String::from_str(&env, "ACTIVE");

    // first add — emits tag_added
    client.add_tag(&owner, &id, &active);
    let event_count_after_first = env.events().all().len();

    // second add (duplicate) — must be silent
    client.add_tag(&owner, &id, &active);
    assert_eq!(
        env.events().all().len(),
        event_count_after_first,
        "duplicate add_tag must not emit any event"
    );
}

/// Adding "ACTIVE" then a different tag then "ACTIVE" again must still result
/// in exactly two unique tags.
#[test]
fn test_qa_double_tag_interleaved_stays_deduplicated() {
    let (env, client, owner) = setup();
    let id = make_policy(&env, &client, &owner);

    client.add_tag(&owner, &id, &String::from_str(&env, "ACTIVE"));
    client.add_tag(&owner, &id, &String::from_str(&env, "VIP"));
    client.add_tag(&owner, &id, &String::from_str(&env, "ACTIVE")); // dup

    let tags = client.get_policy(&id).unwrap().tags;
    assert_eq!(tags.len(), 2, "only two unique tags should be stored");
}

// ── 3. The Ghost Remove ───────────────────────────────────────────────────────

/// Removing a tag that was never added must not crash.
#[test]
fn test_qa_ghost_remove_does_not_panic() {
    let (env, client, owner) = setup();
    let id = make_policy(&env, &client, &owner);
    // no tags — removing "GHOST" must be graceful
    client.remove_tag(&owner, &id, &String::from_str(&env, "GHOST"));
}

/// After a ghost remove the tag list must still be empty.
#[test]
fn test_qa_ghost_remove_state_unchanged() {
    let (env, client, owner) = setup();
    let id = make_policy(&env, &client, &owner);
    client.remove_tag(&owner, &id, &String::from_str(&env, "GHOST"));
    assert_eq!(
        client.get_policy(&id).unwrap().tags.len(),
        0,
        "ghost remove must not alter the tag list"
    );
}

/// Ghost remove on a policy that already has other tags must not disturb them.
#[test]
fn test_qa_ghost_remove_preserves_existing_tags() {
    let (env, client, owner) = setup();
    let id = make_policy(&env, &client, &owner);
    client.add_tag(&owner, &id, &String::from_str(&env, "KEEP"));
    client.remove_tag(&owner, &id, &String::from_str(&env, "GHOST")); // not present
    let tags = client.get_policy(&id).unwrap().tags;
    assert_eq!(tags.len(), 1, "existing tags must be preserved after ghost remove");
    assert_eq!(tags.get(0).unwrap(), String::from_str(&env, "KEEP"));
}

// ── 4. Event Verification ─────────────────────────────────────────────────────

/// add_tag must publish exactly one event with topic ("insure", "tag_added")
/// and data (policy_id, tag).
#[test]
fn test_qa_add_tag_event_topics_and_data() {
    use soroban_sdk::{symbol_short, IntoVal};

    let (env, client, owner) = setup();
    let id = make_policy(&env, &client, &owner);
    let tag = String::from_str(&env, "ACTIVE");

    let events_before = env.events().all().len();
    client.add_tag(&owner, &id, &tag);

    let all = env.events().all();
    assert_eq!(
        all.len(),
        events_before + 1,
        "add_tag must emit exactly one event"
    );

    let (contract_id, topics, data) = all.last().unwrap();
    let _ = contract_id; // emitted by our contract

    // Verify topics: ("insure", "tag_added")
    let expected_topics = soroban_sdk::vec![
        &env,
        symbol_short!("insure").into_val(&env),
        symbol_short!("tag_added").into_val(&env),
    ];
    assert_eq!(topics, expected_topics, "tag_added event topics mismatch");

    // Verify data: (policy_id, tag)
    let (emitted_id, emitted_tag): (u32, String) =
        soroban_sdk::FromVal::from_val(&env, &data);
    assert_eq!(emitted_id, id, "tag_added event must carry the correct policy_id");
    assert_eq!(emitted_tag, tag, "tag_added event must carry the correct tag");
}

/// remove_tag on an existing tag must publish exactly one event with topic
/// ("insure", "tag_rmvd") and data (policy_id, tag).
#[test]
fn test_qa_remove_tag_event_topics_and_data() {
    use soroban_sdk::{symbol_short, IntoVal};

    let (env, client, owner) = setup();
    let id = make_policy(&env, &client, &owner);
    let tag = String::from_str(&env, "ACTIVE");
    client.add_tag(&owner, &id, &tag);

    let events_before = env.events().all().len();
    client.remove_tag(&owner, &id, &tag);

    let all = env.events().all();
    assert_eq!(
        all.len(),
        events_before + 1,
        "remove_tag must emit exactly one event"
    );

    let (_, topics, data) = all.last().unwrap();

    let expected_topics = soroban_sdk::vec![
        &env,
        symbol_short!("insure").into_val(&env),
        symbol_short!("tag_rmvd").into_val(&env),
    ];
    assert_eq!(topics, expected_topics, "tag_rmvd event topics mismatch");

    let (emitted_id, emitted_tag): (u32, String) =
        soroban_sdk::FromVal::from_val(&env, &data);
    assert_eq!(emitted_id, id);
    assert_eq!(emitted_tag, tag);
}

/// Ghost remove must publish exactly one event with topic ("insure", "tag_miss")
/// and data (policy_id, tag) — the "Tag Not Found" signal.
#[test]
fn test_qa_ghost_remove_event_topics_and_data() {
    use soroban_sdk::{symbol_short, IntoVal};

    let (env, client, owner) = setup();
    let id = make_policy(&env, &client, &owner);
    let tag = String::from_str(&env, "GHOST");

    let events_before = env.events().all().len();
    client.remove_tag(&owner, &id, &tag);

    let all = env.events().all();
    assert_eq!(
        all.len(),
        events_before + 1,
        "ghost remove must emit exactly one tag_miss event"
    );

    let (_, topics, data) = all.last().unwrap();

    let expected_topics = soroban_sdk::vec![
        &env,
        symbol_short!("insure").into_val(&env),
        symbol_short!("tag_miss").into_val(&env),
    ];
    assert_eq!(topics, expected_topics, "tag_miss event topics mismatch");

    let (emitted_id, emitted_tag): (u32, String) =
        soroban_sdk::FromVal::from_val(&env, &data);
    assert_eq!(emitted_id, id, "tag_miss event must carry the correct policy_id");
    assert_eq!(emitted_tag, tag, "tag_miss event must carry the correct tag");
}

/// Full lifecycle: add "ACTIVE", add "ACTIVE" again (dup), remove "ACTIVE",
/// remove "ACTIVE" again (ghost). Verify the exact event sequence.
#[test]
fn test_qa_full_lifecycle_event_sequence() {
    use soroban_sdk::{symbol_short, IntoVal};

    let (env, client, owner) = setup();
    let id = make_policy(&env, &client, &owner);
    let tag = String::from_str(&env, "ACTIVE");

    let baseline = env.events().all().len(); // events from create_policy

    // 1. add "ACTIVE" → emits tag_added
    client.add_tag(&owner, &id, &tag);
    assert_eq!(env.events().all().len(), baseline + 1);

    // 2. add "ACTIVE" again (dup) → no event
    client.add_tag(&owner, &id, &tag);
    assert_eq!(env.events().all().len(), baseline + 1, "dup add must be silent");

    // 3. remove "ACTIVE" → emits tag_rmvd
    client.remove_tag(&owner, &id, &tag);
    assert_eq!(env.events().all().len(), baseline + 2);

    // 4. remove "ACTIVE" again (ghost) → emits tag_miss
    client.remove_tag(&owner, &id, &tag);
    assert_eq!(env.events().all().len(), baseline + 3);

    // Verify the three event topics in order
    let all = env.events().all();
    let e1 = all.get(baseline as u32).unwrap();
    let e2 = all.get((baseline + 1) as u32).unwrap();
    let e3 = all.get((baseline + 2) as u32).unwrap();

    let topic_added = soroban_sdk::vec![
        &env,
        symbol_short!("insure").into_val(&env),
        symbol_short!("tag_added").into_val(&env),
    ];
    let topic_rmvd = soroban_sdk::vec![
        &env,
        symbol_short!("insure").into_val(&env),
        symbol_short!("tag_rmvd").into_val(&env),
    ];
    let topic_miss = soroban_sdk::vec![
        &env,
        symbol_short!("insure").into_val(&env),
        symbol_short!("tag_miss").into_val(&env),
    ];

    assert_eq!(e1.1, topic_added, "first event must be tag_added");
    assert_eq!(e2.1, topic_rmvd,  "second event must be tag_rmvd");
    assert_eq!(e3.1, topic_miss,  "third event must be tag_miss");
}
