#![no_std]
use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short,
    Address, Env, Map, String, Vec,
};

// ── Coverage type (defined locally to avoid cross-crate DLL issues) ───────────

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum CoverageType {
    Health   = 1,
    Life     = 2,
    Property = 3,
    Auto     = 4,
    Liability = 5,
}

// ── Errors ────────────────────────────────────────────────────────────────────

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum InsuranceError {
    Unauthorized   = 1,
    InvalidAmount  = 2,
    PolicyNotFound = 3,
    PolicyInactive = 4,
    InvalidPremium = 5,
    InvalidCoverage = 6,
}

// ── Storage types ─────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone)]
pub struct InsurancePolicy {
    pub id: u32,
    pub owner: Address,
    pub name: String,
    pub coverage_type: CoverageType,
    pub monthly_premium: i128,
    pub coverage_amount: i128,
    pub active: bool,
    pub next_payment_date: u64,
    pub external_ref: Option<String>,
    /// Policy tags — deduplicated, max 32 chars each
    pub tags: Vec<String>,
}

// ── Storage keys ──────────────────────────────────────────────────────────────

const KEY_POLICIES: soroban_sdk::Symbol = symbol_short!("POLICIES");
const KEY_NEXT_ID:  soroban_sdk::Symbol = symbol_short!("NEXT_ID");
const KEY_ADMIN:    soroban_sdk::Symbol = symbol_short!("ADMIN");

// ── Contract ──────────────────────────────────────────────────────────────────

#[contract]
pub struct Insurance;

#[contractimpl]
impl Insurance {

    // ── Admin ─────────────────────────────────────────────────────────────────

    /// Set the contract admin. Can only be called once (bootstrap) or by the
    /// existing admin.
    pub fn set_admin(env: Env, caller: Address, new_admin: Address) {
        caller.require_auth();
        let current: Option<Address> = env.storage().instance().get(&KEY_ADMIN);
        if let Some(ref admin) = current {
            if *admin != caller {
                panic!("unauthorized");
            }
        }
        env.storage().instance().set(&KEY_ADMIN, &new_admin);
    }

    fn get_admin(env: &Env) -> Option<Address> {
        env.storage().instance().get(&KEY_ADMIN)
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn load_policies(env: &Env) -> Map<u32, InsurancePolicy> {
        env.storage()
            .instance()
            .get(&KEY_POLICIES)
            .unwrap_or_else(|| Map::new(env))
    }

    fn save_policies(env: &Env, m: &Map<u32, InsurancePolicy>) {
        env.storage().instance().set(&KEY_POLICIES, m);
    }

    fn bump(env: &Env) {
        env.storage()
            .instance()
            .extend_ttl(17280, 518400);
    }

    // ── Policy CRUD ───────────────────────────────────────────────────────────

    /// Create a new insurance policy. Caller must authorise.
    pub fn create_policy(
        env: Env,
        owner: Address,
        name: String,
        coverage_type: CoverageType,
        monthly_premium: i128,
        coverage_amount: i128,
    ) -> u32 {
        owner.require_auth();
        if monthly_premium <= 0 {
            panic!("Monthly premium must be positive");
        }
        if coverage_amount <= 0 {
            panic!("Coverage amount must be positive");
        }
        Self::bump(&env);

        let mut policies = Self::load_policies(&env);
        let next_id: u32 = env
            .storage()
            .instance()
            .get(&KEY_NEXT_ID)
            .unwrap_or(0u32)
            + 1;

        let policy = InsurancePolicy {
            id: next_id,
            owner: owner.clone(),
            name,
            coverage_type,
            monthly_premium,
            coverage_amount,
            active: true,
            next_payment_date: env.ledger().timestamp() + 30 * 86400,
            external_ref: None,
            tags: Vec::new(&env),
        };

        policies.set(next_id, policy);
        Self::save_policies(&env, &policies);
        env.storage().instance().set(&KEY_NEXT_ID, &next_id);

        env.events().publish(
            (symbol_short!("insure"), symbol_short!("created")),
            (next_id, owner),
        );

        next_id
    }

    /// Pay a premium. Caller must be the policy owner.
    pub fn pay_premium(env: Env, caller: Address, policy_id: u32) -> Result<(), InsuranceError> {
        caller.require_auth();
        Self::bump(&env);

        let mut policies = Self::load_policies(&env);
        let mut policy = policies
            .get(policy_id)
            .ok_or(InsuranceError::PolicyNotFound)?;

        if policy.owner != caller {
            return Err(InsuranceError::Unauthorized);
        }
        if !policy.active {
            return Err(InsuranceError::PolicyInactive);
        }

        policy.next_payment_date = env.ledger().timestamp() + 30 * 86400;
        policies.set(policy_id, policy);
        Self::save_policies(&env, &policies);

        env.events().publish(
            (symbol_short!("insure"), symbol_short!("paid")),
            (policy_id, caller),
        );
        Ok(())
    }

    /// Deactivate a policy. Caller must be the policy owner.
    pub fn deactivate_policy(env: Env, caller: Address, policy_id: u32) -> Result<bool, InsuranceError> {
        caller.require_auth();
        Self::bump(&env);

        let mut policies = Self::load_policies(&env);
        let mut policy = policies
            .get(policy_id)
            .ok_or(InsuranceError::PolicyNotFound)?;

        if policy.owner != caller {
            return Err(InsuranceError::Unauthorized);
        }

        policy.active = false;
        policies.set(policy_id, policy);
        Self::save_policies(&env, &policies);

        env.events().publish(
            (symbol_short!("insure"), symbol_short!("deactive")),
            (policy_id, caller),
        );
        Ok(true)
    }

    /// Get a policy by ID.
    pub fn get_policy(env: Env, policy_id: u32) -> Option<InsurancePolicy> {
        Self::load_policies(&env).get(policy_id)
    }

    /// Get all active policies for an owner.
    pub fn get_active_policies(env: Env, owner: Address) -> Vec<InsurancePolicy> {
        let policies = Self::load_policies(&env);
        let mut result = Vec::new(&env);
        for (_, p) in policies.iter() {
            if p.active && p.owner == owner {
                result.push_back(p);
            }
        }
        result
    }

    /// Get total monthly premium for all active policies of an owner.
    pub fn get_total_monthly_premium(env: Env, owner: Address) -> i128 {
        let policies = Self::load_policies(&env);
        let mut total = 0i128;
        for (_, p) in policies.iter() {
            if p.active && p.owner == owner {
                total = total.saturating_add(p.monthly_premium);
            }
        }
        total
    }

    // ── Tag management ────────────────────────────────────────────────────────

    /// Add a tag to a policy.
    ///
    /// # Authorization
    /// Only the **policy owner** or the contract **admin** may add tags.
    ///
    /// # Deduplication
    /// If the tag already exists on the policy it is silently skipped — no
    /// duplicate is stored and no error is returned.
    ///
    /// # Events
    /// Emits `("insure", "tag_added")` with `(policy_id, tag)` as data.
    ///
    /// # Panics
    /// - If `policy_id` does not exist.
    /// - If `caller` is neither the policy owner nor the admin.
    /// - If `tag` is empty or longer than 32 characters.
    pub fn add_tag(env: Env, caller: Address, policy_id: u32, tag: String) {
        caller.require_auth();

        if tag.len() == 0 || tag.len() > 32 {
            panic!("tag must be 1–32 characters");
        }

        Self::bump(&env);
        let mut policies = Self::load_policies(&env);
        let mut policy = policies
            .get(policy_id)
            .unwrap_or_else(|| panic!("policy not found"));

        // Authorization: policy owner OR admin
        let is_owner = policy.owner == caller;
        let is_admin = Self::get_admin(&env).map(|a| a == caller).unwrap_or(false);
        if !is_owner && !is_admin {
            panic!("unauthorized");
        }

        // Deduplication: skip if tag already present
        for existing in policy.tags.iter() {
            if existing == tag {
                return; // already exists, nothing to do
            }
        }

        policy.tags.push_back(tag.clone());
        policies.set(policy_id, policy);
        Self::save_policies(&env, &policies);

        // Emit TagAdded event
        env.events().publish(
            (symbol_short!("insure"), symbol_short!("tag_added")),
            (policy_id, tag),
        );
    }

    /// Remove a tag from a policy.
    ///
    /// # Authorization
    /// Only the **policy owner** or the contract **admin** may remove tags.
    ///
    /// # Graceful removal
    /// If the tag does not exist on the policy, the function returns without
    /// panicking and emits a `"tag_no_tag"` ("Tag Not Found") event instead.
    ///
    /// # Events
    /// - `("insure", "tag_removed")` with `(policy_id, tag)` when removed.
    /// - `("insure", "tag_no_tag")` with `(policy_id, tag)` when not found.
    ///
    /// # Panics
    /// - If `policy_id` does not exist.
    /// - If `caller` is neither the policy owner nor the admin.
    pub fn remove_tag(env: Env, caller: Address, policy_id: u32, tag: String) {
        caller.require_auth();
        Self::bump(&env);

        let mut policies = Self::load_policies(&env);
        let mut policy = policies
            .get(policy_id)
            .unwrap_or_else(|| panic!("policy not found"));

        // Authorization: policy owner OR admin
        let is_owner = policy.owner == caller;
        let is_admin = Self::get_admin(&env).map(|a| a == caller).unwrap_or(false);
        if !is_owner && !is_admin {
            panic!("unauthorized");
        }

        // Find the tag
        let mut found = false;
        let mut new_tags = Vec::new(&env);
        for existing in policy.tags.iter() {
            if existing == tag {
                found = true; // skip it (remove)
            } else {
                new_tags.push_back(existing);
            }
        }

        if !found {
            // Graceful: emit "Tag Not Found" event, do not panic
            env.events().publish(
                (symbol_short!("insure"), symbol_short!("tag_miss")),
                (policy_id, tag),
            );
            return;
        }

        policy.tags = new_tags;
        policies.set(policy_id, policy);
        Self::save_policies(&env, &policies);

        // Emit TagRemoved event
        env.events().publish(
            (symbol_short!("insure"), symbol_short!("tag_rmvd")),
            (policy_id, tag),
        );
    }
}

#[cfg(test)]
mod test;
