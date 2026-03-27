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
