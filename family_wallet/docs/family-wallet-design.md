# Family Wallet — Design Documentation

## Overview

The `FamilyWallet` contract is a Soroban-based multisig family wallet with
role-based access control, time-bounded roles, and configurable execution
policies. This document covers two major subsystems: **Role Expiry** and
**Multisig Threshold Bounds Validation**.

---

# Part 1 — Role Expiry Design

## Overview

The `FamilyWallet` contract supports time-bounded roles. Any role except `Owner`
can be given an expiry timestamp. Once the ledger clock reaches or passes that
timestamp, the role is treated as if it does not exist for authorization purposes.

---

## Role Hierarchy

| Role    | Ordinal | Notes                          |
|---------|---------|--------------------------------|
| Owner   | 0       | Never expires; full control    |
| Admin   | 1       | Can manage members and expiries|
| Member  | 2       | Can propose transactions       |
| Viewer  | 3       | Read-only                      |

Lower ordinal = higher privilege. `require_role_at_least(min_role)` passes when
`role_ordinal(caller) <= role_ordinal(min_role)`.

---

## Role Expiry Mechanics

### Storage

Expiries are stored in a `Map<Address, u64>` under the `ROLE_EXP` storage key.
A missing entry means no expiry (the role never expires).

### Setting an Expiry

```rust
// Requires Admin role minimum
pub fn set_role_expiry(env, caller, member, expires_at: Option<u64>) -> bool
```

- `Some(ts)` — sets expiry to ledger timestamp `ts`
- `None` — clears expiry; the role becomes permanent again

### Expiry Check

```rust
fn role_has_expired(env, address) -> bool {
    if let Some(exp) = get_role_expiry(env, address) {
        env.ledger().timestamp() >= exp  // INCLUSIVE boundary
    } else {
        false
    }
}
```

> ⚠️ **Boundary is inclusive (`>=`)**
> A role set to expire at timestamp `T` is already expired when the ledger
> reads exactly `T`. Plan expiry windows accordingly.

### Enforcement

`require_role_at_least()` calls `role_has_expired()` before checking the role
ordinal. An expired role panics with `"Role has expired"` regardless of what
role the member holds.

---

## Lifecycle

```
Owner sets expiry          Role active          Role expires
        │                       │                    │
  t=1000│                 t=1000-1999             t=2000+
        ▼                       ▼                    ▼
  set_role_expiry(         any action            "Role has
  member, Some(2000))       succeeds             expired" panic
```

### Renewal

Only `Owner` or an **active** `Admin` can renew an expired role:

```
set_role_expiry(owner, expired_admin, Some(new_future_ts))
```

An expired admin **cannot renew their own role** — the expiry check fires
before any authorization logic runs.

---

## Security Assumptions

### 1. Expired roles cannot self-renew
`require_role_at_least` is called inside `set_role_expiry`. An expired caller
panics before reaching the storage write, making self-renewal impossible.

### 2. Plain members cannot set expiries
`set_role_expiry` requires `FamilyRole::Admin` minimum. A `Member` or `Viewer`
calling it will panic with `"Insufficient role"`.

### 3. Non-members are fully blocked
Any address not in the `MEMBERS` map panics with `"Not a family member"` before
any role or expiry check runs.

### 4. Owner is immune to expiry side-effects
`role_ordinal(Owner) == 0` satisfies every `require_role_at_least` call.
Even if an expiry is set on the Owner address, the Owner's `require_auth()`
bypass means core admin actions remain available. Setting expiry on Owner
is considered a misconfiguration and should be avoided.

### 5. Ledger timestamp is the source of truth
Tests must use `env.ledger().with_mut(|li| li.timestamp = ts)` to simulate
time. Wall-clock time is irrelevant; only the ledger timestamp matters.

### 6. Past-timestamp expiry takes immediate effect
Setting `expires_at` to a value already less than the current ledger timestamp
immediately invalidates the role. There is no grace period.

---

## Test Coverage Summary

| Test Group                          | Tests | Covers                                      |
|-------------------------------------|-------|---------------------------------------------|
| Role active baseline                | 3     | No expiry, active before expiry, Owner bypass|
| Exact boundary (inclusive >=)       | 3     | At T, T-1, T+1                              |
| Post-expiry rejections              | 4     | add_member, set_expiry, multisig, propose   |
| Unauthorized renewal paths          | 4     | Self-renew, plain member, non-member, expired admin |
| Successful owner renewal            | 4     | Renew, correct storage, permission limits, clear |
| Edge cases                          | 7     | Independent expiries, past timestamp, overflow, audit |
| **Total**                           | **25**| **>95% branch coverage on expiry paths**    |

---

## Running the Tests

```bash
cargo test -p family_wallet
```

Expected output: all 25 tests pass with no warnings on expiry-related code paths.

---

# Part 2 — Multisig Threshold Bounds Validation

## Overview

`configure_multisig` enforces strict bounds on threshold and signer
configuration to prevent invalid execution policy states. The function returns
`Result<bool, Error>` with specific error codes for each validation failure,
enabling callers to programmatically distinguish between error conditions.

## Constants

| Constant       | Value | Purpose                                  |
|----------------|-------|------------------------------------------|
| `MIN_THRESHOLD`| 1     | Minimum required signatures              |
| `MAX_THRESHOLD`| 100   | Maximum allowed threshold                |
| `MAX_SIGNERS`  | 100   | Maximum number of authorized signers     |

## Error Codes

| Error Variant           | Code | Condition                                         |
|-------------------------|------|---------------------------------------------------|
| `Unauthorized`          | 1    | Caller is not Owner or Admin                      |
| `SignersListEmpty`      | 16   | `signers.len() == 0`                              |
| `TooManySigners`        | 19   | `signers.len() > MAX_SIGNERS`                     |
| `ThresholdBelowMinimum` | 14   | `threshold < MIN_THRESHOLD`                       |
| `ThresholdAboveMaximum` | 15   | `threshold > MAX_THRESHOLD`                       |
| `InvalidThreshold`      | 2    | `threshold > signers.len()`                       |
| `SignerNotMember`       | 17   | Any signer is not in the family members map       |
| `DuplicateSigner`       | 18   | Same address appears more than once in signers    |
| `InvalidSpendingLimit`  | 13   | `spending_limit < 0`                              |

## Validation Order

The function validates in this order (short-circuits on first failure):

1. **Caller authorization** — must be Owner or Admin (not expired)
2. **Contract not paused**
3. **Signers list non-empty**
4. **Signer count within MAX_SIGNERS**
5. **Threshold >= MIN_THRESHOLD**
6. **Threshold <= MAX_THRESHOLD**
7. **Threshold <= signer_count**
8. **Each signer is a family member** (single pass)
9. **No duplicate signers** (enforced in same pass via tracking map)
10. **Spending limit non-negative**

## Security Assumptions

### 1. Threshold cannot exceed signer count
A threshold of 5 with only 3 signers would make execution impossible. This
invariant is enforced: `threshold <= signers.len()`.

### 2. Minimum threshold of 1
A threshold of 0 would allow execution without any signatures, defeating the
purpose of multisig. `MIN_THRESHOLD = 1` ensures at least one signature is
always required.

### 3. Maximum signer cap prevents unbounded iteration
`MAX_SIGNERS = 100` bounds the signer verification loop, preventing gas
exhaustion attacks from excessively large signer lists.

### 4. Duplicate signers are rejected
Without duplicate detection, an attacker could add the same address multiple
times to artificially inflate the signer count, allowing a lower effective
threshold.

### 5. All signers must be family members
Only addresses in the `MEMBERS` map can be configured as signers. This prevents
external addresses from being injected into the execution policy.

### 6. Error returns instead of panics
`configure_multisig` returns `Result<bool, Error>` so callers can distinguish
between validation failures programmatically. This is critical for
composability and for frontends that need to display specific error messages.

## Test Coverage Summary

| Test Group                          | Tests | Covers                                      |
|-------------------------------------|-------|---------------------------------------------|
| Threshold minimum valid             | 1     | threshold = 1 succeeds                      |
| Threshold maximum valid             | 1     | threshold = 10 with 10 signers              |
| Threshold above maximum rejected    | 1     | threshold = 101 → `ThresholdAboveMaximum`   |
| Threshold zero rejected             | 1     | threshold = 0 → `ThresholdBelowMinimum`     |
| Threshold exceeds signer count      | 1     | threshold > signers → `InvalidThreshold`    |
| Empty signers list rejected         | 1     | empty vec → `SignersListEmpty`              |
| Signer not family member rejected   | 1     | non-member signer → `SignerNotMember`       |
| Duplicate signer rejected           | 2     | exact duplicate, mid-list duplicate         |
| Too many signers rejected           | 1     | 101 signers → `TooManySigners`              |
| Negative spending limit rejected    | 1     | negative limit → `InvalidSpendingLimit`     |
| Threshold bounds return correct errors | 1  | Verifies all error codes in sequence        |
| Threshold consistency across types  | 1     | Independent thresholds per tx type          |
| Threshold equals signer count       | 1     | Unanimous consent configuration             |
| Threshold one with multiple signers | 1     | Any-single-signer configuration             |
| Paused contract rejection           | 1     | `#[should_panic]` for paused state          |
| Unauthorized caller rejection       | 1     | Non-owner/non-admin → `Unauthorized`        |
| Admin can configure multisig        | 1     | Admin role can configure                    |
| **Total**                           | **17**| **>95% branch coverage on validation paths**|

## Running the Tests

```bash
cargo test -p family_wallet
```