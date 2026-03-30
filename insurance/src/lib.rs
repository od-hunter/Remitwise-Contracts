#![no_std]

#[cfg(test)]
mod test;

use remitwise_common::{
    clamp_limit, EventCategory, EventPriority, RemitwiseEvents, CoverageType,
    INSTANCE_BUMP_AMOUNT, INSTANCE_LIFETIME_THRESHOLD, MAX_BATCH_SIZE,
};

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, Address, Env, Map, String,
    Vec, IntoVal,
};

#[derive(Clone, Debug)]
#[contracttype]
pub struct InsurancePolicy {
    pub id: u32,
    pub owner: Address,
    pub name: String,
    pub coverage_type: CoverageType,
    pub monthly_premium: i128,
    pub coverage_amount: i128,
    pub active: bool,
    pub next_payment_due: u64,
    pub last_payment_at: u64,
    pub created_at: u64,
    pub external_ref: Option<String>,
}

#[derive(Clone, Debug)]
#[contracttype]
pub struct InsurancePage {
    pub items: Vec<InsurancePolicy>,
    pub next_cursor: u32,
    pub count: u32,
}

#[derive(Clone, Debug)]
#[contracttype]
pub struct PremiumSchedule {
    pub id: u32,
    pub policy_id: u32,
    pub owner: Address,
    pub next_due: u64,
    pub interval: u64,
    pub active: bool,
    pub missed_count: u32,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum InsuranceError {
    Unauthorized = 1,
    AlreadyInitialized = 2,
    NotInitialized = 3,
    PolicyNotFound = 4,
    PolicyInactive = 5,
    InvalidName = 6,
    InvalidPremiumAmount = 7,
    InvalidCoverageAmount = 8,
    UnsupportedCombination = 9,
    InvalidExternalRef = 10,
    MaxPoliciesReached = 11,
    ScheduleNotFound = 12,
}

#[derive(Clone)]
#[contracttype]
pub enum DataKey {
    Admin,
    PolicyCount,
    Policies,
    ActivePolicies,
    KillswitchId,
    ScheduleCount,
    Schedules,
}

#[contract]
pub struct Insurance;

#[contractimpl]
impl Insurance {
    /// Initializes the insurance contract with an admin and killswitch reference.
    pub fn init(env: Env, admin: Address, killswitch_id: Address) -> Result<(), InsuranceError> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(InsuranceError::AlreadyInitialized);
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::KillswitchId, &killswitch_id);
        env.storage().instance().set(&DataKey::PolicyCount, &0u32);
        env.storage().instance().set(&DataKey::ScheduleCount, &0u32);
        
        let active_policies: Vec<u32> = Vec::new(&env);
        env.storage().instance().set(&DataKey::ActivePolicies, &active_policies);
        
        Ok(())
    }

    /// Creates a new insurance policy for the caller.
    pub fn create_policy(
        env: Env,
        owner: Address,
        name: String,
        coverage_type: CoverageType,
        monthly_premium: i128,
        coverage_amount: i128,
        external_ref: Option<String>,
    ) -> Result<u32, InsuranceError> {
        owner.require_auth();
        Self::check_initialized(&env)?;
        
        // Strict Validation
        if name.len() == 0 { return Err(InsuranceError::InvalidName); }
        if name.len() > 64 { return Err(InsuranceError::InvalidName); }
        if monthly_premium <= 0 { return Err(InsuranceError::InvalidPremiumAmount); }
        if coverage_amount <= 0 { return Err(InsuranceError::InvalidCoverageAmount); }
        
        Self::validate_ranges(&coverage_type, monthly_premium, coverage_amount)?;
        
        // Ratio Guard: coverage_amount <= monthly_premium * 12 * 500
        let max_coverage = monthly_premium.checked_mul(6000).ok_or(InsuranceError::UnsupportedCombination)?;
        if coverage_amount > max_coverage {
             return Err(InsuranceError::UnsupportedCombination);
        }

        if let Some(ref ext) = external_ref {
            if ext.len() == 0 || ext.len() > 128 { return Err(InsuranceError::InvalidExternalRef); }
        }

        let mut active_policies: Vec<u32> = env.storage().instance().get(&DataKey::ActivePolicies).unwrap();
        if active_policies.len() >= 1000 {
            return Err(InsuranceError::MaxPoliciesReached);
        }

        let mut count: u32 = env.storage().instance().get(&DataKey::PolicyCount).unwrap();
        count = count.checked_add(1).ok_or(InsuranceError::MaxPoliciesReached)?;
        
        let policy = InsurancePolicy {
            id: count,
            owner: owner.clone(),
            name: name.clone(),
            coverage_type: coverage_type.clone(),
            monthly_premium,
            coverage_amount,
            active: true,
            next_payment_due: env.ledger().timestamp() + (30 * 86400),
            last_payment_at: 0,
            created_at: env.ledger().timestamp(),
            external_ref,
        };

        let mut policies: Map<u32, InsurancePolicy> = env.storage().instance().get(&DataKey::Policies).unwrap_or_else(|| Map::new(&env));
        policies.set(count, policy);
        env.storage().instance().set(&DataKey::Policies, &policies);
        env.storage().instance().set(&DataKey::PolicyCount, &count);
        
        active_policies.push_back(count);
        env.storage().instance().set(&DataKey::ActivePolicies, &active_policies);

        Self::extend_ttl(&env);
        
        RemitwiseEvents::emit(&env, EventCategory::State, EventPriority::Medium, symbol_short!("created"), (count, owner, (coverage_type as u32), monthly_premium, coverage_amount));

        Ok(count)
    }

    /// Records a premium payment for a policy.
    pub fn pay_premium(env: Env, owner: Address, policy_id: u32) -> Result<bool, InsuranceError> {
        owner.require_auth();
        Self::check_initialized(&env)?;
        Self::check_killswitch(&env)?;

        let mut policies: Map<u32, InsurancePolicy> = env.storage().instance().get(&DataKey::Policies).ok_or(InsuranceError::PolicyNotFound)?;
        let mut policy = policies.get(policy_id).ok_or(InsuranceError::PolicyNotFound)?;

        if policy.owner != owner { return Err(InsuranceError::Unauthorized); }
        if !policy.active { return Err(InsuranceError::PolicyInactive); }

        policy.last_payment_at = env.ledger().timestamp();
        policy.next_payment_due = env.ledger().timestamp() + (30 * 86400);

        policies.set(policy_id, policy.clone());
        env.storage().instance().set(&DataKey::Policies, &policies);
        
        Self::extend_ttl(&env);

        RemitwiseEvents::emit(&env, EventCategory::Transaction, EventPriority::High, symbol_short!("paid"), (policy_id, owner, policy.monthly_premium, policy.next_payment_due));

        Ok(true)
    }

    /// Batch processing of premium payments.
    pub fn batch_pay_premiums(env: Env, owner: Address, ids: Vec<u32>) -> Result<u32, InsuranceError> {
        owner.require_auth();
        if ids.len() > MAX_BATCH_SIZE { return Err(InsuranceError::Unauthorized); } // Or BatchTooLarge if defined
        
        let mut paid_count = 0;
        for id in ids.iter() {
             if let Ok(_) = Self::pay_premium(env.clone(), owner.clone(), id) {
                 paid_count += 1;
             }
        }
        Ok(paid_count)
    }

    /// Deactivates a policy. Restricted to contract admin.
    pub fn deactivate_policy(env: Env, caller: Address, policy_id: u32) -> Result<bool, InsuranceError> {
        caller.require_auth();
        Self::check_initialized(&env)?;
        
        let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        if caller != admin { return Err(InsuranceError::Unauthorized); }

        let mut policies: Map<u32, InsurancePolicy> = env.storage().instance().get(&DataKey::Policies).ok_or(InsuranceError::PolicyNotFound)?;
        let mut policy = policies.get(policy_id).ok_or(InsuranceError::PolicyNotFound)?;

        if !policy.active { return Err(InsuranceError::PolicyInactive); }
        policy.active = false;
        policies.set(policy_id, policy.clone());
        env.storage().instance().set(&DataKey::Policies, &policies);

        let mut active_policies: Vec<u32> = env.storage().instance().get(&DataKey::ActivePolicies).unwrap();
        if let Some(idx) = active_policies.first_index_of(policy_id) {
            active_policies.remove(idx);
            env.storage().instance().set(&DataKey::ActivePolicies, &active_policies);
        }

        Self::extend_ttl(&env);

        RemitwiseEvents::emit(&env, EventCategory::State, EventPriority::Medium, symbol_short!("deactive"), (policy_id, policy.owner, env.ledger().timestamp()));

        Ok(true)
    }

    /// Updates the external reference for a policy. Restricted to contract admin.
    pub fn set_external_ref(env: Env, caller: Address, policy_id: u32, ext_ref: Option<String>) -> Result<bool, InsuranceError> {
        caller.require_auth();
        Self::check_initialized(&env)?;

        let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        if caller != admin { return Err(InsuranceError::Unauthorized); }

        if let Some(ref ext) = ext_ref {
            if ext.len() == 0 || ext.len() > 128 { return Err(InsuranceError::InvalidExternalRef); }
        }

        let mut policies: Map<u32, InsurancePolicy> = env.storage().instance().get(&DataKey::Policies).ok_or(InsuranceError::PolicyNotFound)?;
        let mut policy = policies.get(policy_id).ok_or(InsuranceError::PolicyNotFound)?;

        policy.external_ref = ext_ref;
        policies.set(policy_id, policy);
        env.storage().instance().set(&DataKey::Policies, &policies);

        Self::extend_ttl(&env);
        Ok(true)
    }

    pub fn get_policy(env: Env, policy_id: u32) -> Option<InsurancePolicy> {
        let policies: Map<u32, InsurancePolicy> = env.storage().instance().get(&DataKey::Policies)?;
        policies.get(policy_id)
    }

    pub fn get_active_policies(env: Env, owner: Address, cursor: u32, limit: u32) -> InsurancePage {
        let limit = clamp_limit(limit);
        let active_ids: Vec<u32> = env.storage().instance().get(&DataKey::ActivePolicies).unwrap_or_else(|| Vec::new(&env));
        let policies: Map<u32, InsurancePolicy> = env.storage().instance().get(&DataKey::Policies).unwrap_or_else(|| Map::new(&env));
        
        let mut items = Vec::new(&env);
        let mut next_cursor = 0;
        let mut count = 0;

        for id in active_ids.iter() {
            if id <= cursor { continue; }
            if let Some(p) = policies.get(id) {
                if p.owner == owner {
                    items.push_back(p);
                    count += 1;
                    if count >= limit {
                        next_cursor = id;
                        break;
                    }
                }
            }
        }
        InsurancePage { items, next_cursor, count }
    }

    pub fn get_all_policies_for_owner(env: Env, owner: Address, cursor: u32, limit: u32) -> InsurancePage {
        let limit = clamp_limit(limit);
        let policies: Map<u32, InsurancePolicy> = env.storage().instance().get(&DataKey::Policies).unwrap_or_else(|| Map::new(&env));
        let count_total: u32 = env.storage().instance().get(&DataKey::PolicyCount).unwrap_or(0);
        
        let mut items = Vec::new(&env);
        let mut current_count = 0;
        let mut next_cursor = 0;

        for id in (cursor + 1)..=count_total {
            if let Some(p) = policies.get(id) {
                if p.owner == owner {
                    items.push_back(p);
                    current_count += 1;
                    if current_count >= limit {
                        next_cursor = id;
                        break;
                    }
                }
            }
        }
        InsurancePage { items, next_cursor, count: current_count }
    }

    pub fn get_total_monthly_premium(env: Env, owner: Address) -> i128 {
        let active_ids: Vec<u32> = env.storage().instance().get(&DataKey::ActivePolicies).unwrap_or_else(|| Vec::new(&env));
        let policies: Map<u32, InsurancePolicy> = env.storage().instance().get(&DataKey::Policies).unwrap_or_else(|| Map::new(&env));
        
        let mut total: i128 = 0;
        for id in active_ids.iter() {
            if let Some(p) = policies.get(id) {
                if p.owner == owner {
                    total = total.saturating_add(p.monthly_premium);
                }
            }
        }
        total
    }

    pub fn create_premium_schedule(env: Env, owner: Address, policy_id: u32, next_due: u64, interval: u64) -> Result<u32, InsuranceError> {
        owner.require_auth();
        Self::check_initialized(&env)?;
        
        let policies: Map<u32, InsurancePolicy> = env.storage().instance().get(&DataKey::Policies).ok_or(InsuranceError::PolicyNotFound)?;
        let policy = policies.get(policy_id).ok_or(InsuranceError::PolicyNotFound)?;
        if policy.owner != owner { return Err(InsuranceError::Unauthorized); }
        
        let mut count: u32 = env.storage().instance().get(&DataKey::ScheduleCount).unwrap_or(0);
        count += 1;
        
        let schedule = PremiumSchedule {
            id: count,
            policy_id,
            owner: owner.clone(),
            next_due,
            interval,
            active: true,
            missed_count: 0,
        };

        let mut schedules: Map<u32, PremiumSchedule> = env.storage().instance().get(&DataKey::Schedules).unwrap_or_else(|| Map::new(&env));
        schedules.set(count, schedule);
        env.storage().instance().set(&DataKey::Schedules, &schedules);
        env.storage().instance().set(&DataKey::ScheduleCount, &count);
        
        Ok(count)
    }

    pub fn modify_premium_schedule(env: Env, owner: Address, schedule_id: u32, next_due: u64, interval: u64) -> Result<bool, InsuranceError> {
        owner.require_auth();
        let mut schedules: Map<u32, PremiumSchedule> = env.storage().instance().get(&DataKey::Schedules).ok_or(InsuranceError::ScheduleNotFound)?;
        let mut schedule = schedules.get(schedule_id).ok_or(InsuranceError::ScheduleNotFound)?;
        if schedule.owner != owner { return Err(InsuranceError::Unauthorized); }
        
        schedule.next_due = next_due;
        schedule.interval = interval;
        schedules.set(schedule_id, schedule);
        env.storage().instance().set(&DataKey::Schedules, &schedules);
        Ok(true)
    }

    pub fn cancel_premium_schedule(env: Env, owner: Address, schedule_id: u32) -> Result<bool, InsuranceError> {
        owner.require_auth();
        let mut schedules: Map<u32, PremiumSchedule> = env.storage().instance().get(&DataKey::Schedules).ok_or(InsuranceError::ScheduleNotFound)?;
        let mut schedule = schedules.get(schedule_id).ok_or(InsuranceError::ScheduleNotFound)?;
        if schedule.owner != owner { return Err(InsuranceError::Unauthorized); }
        
        schedule.active = false;
        schedules.set(schedule_id, schedule);
        env.storage().instance().set(&DataKey::Schedules, &schedules);
        Ok(true)
    }

    pub fn execute_due_premium_schedules(env: Env) -> Vec<u32> {
        let now = env.ledger().timestamp();
        let mut schedules: Map<u32, PremiumSchedule> = env.storage().instance().get(&DataKey::Schedules).unwrap_or_else(|| Map::new(&env));
        let mut executed = Vec::new(&env);
        
        let count: u32 = env.storage().instance().get(&DataKey::ScheduleCount).unwrap_or(0);
        for id in 1..=count {
            if let Some(mut s) = schedules.get(id) {
                if s.active && s.next_due <= now {
                    match Self::pay_premium(env.clone(), s.owner.clone(), s.policy_id) {
                        Ok(_) => {
                            executed.push_back(id);
                            if s.interval > 0 {
                                s.next_due += s.interval;
                            } else {
                                s.active = false;
                            }
                        }
                        Err(_) => {
                            s.missed_count += 1;
                            if s.interval > 0 {
                                s.next_due += s.interval;
                            } else {
                                s.active = false;
                            }
                        }
                    }
                    schedules.set(id, s);
                }
            }
        }
        env.storage().instance().set(&DataKey::Schedules, &schedules);
        executed
    }

    pub fn get_premium_schedule(env: Env, id: u32) -> Option<PremiumSchedule> {
        let schedules: Map<u32, PremiumSchedule> = env.storage().instance().get(&DataKey::Schedules)?;
        schedules.get(id)
    }

    // --- Private Helpers ---

    fn check_initialized(env: &Env) -> Result<(), InsuranceError> {
        if !env.storage().instance().has(&DataKey::Admin) {
            return Err(InsuranceError::NotInitialized);
        }
        Ok(())
    }

    fn check_killswitch(env: &Env) -> Result<(), InsuranceError> {
        if let Some(killswitch_id) = env.storage().instance().get::<_, Address>(&DataKey::KillswitchId) {
            let is_paused: bool = env.invoke_contract(&killswitch_id, &symbol_short!("is_paused"), (symbol_short!("ins"),).into_val(env));
            if is_paused {
                 panic!("Contract is currently paused for emergency maintenance.");
            }
        }
        Ok(())
    }

    fn extend_ttl(env: &Env) {
        env.storage().instance().extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
    }

    fn validate_ranges(t: &CoverageType, prem: i128, cov: i128) -> Result<(), InsuranceError> {
        match t {
            CoverageType::Health => {
                if prem < 1_000_000 || prem > 500_000_000 { return Err(InsuranceError::InvalidPremiumAmount); }
                if cov < 10_000_000 || cov > 100_000_000_000 { return Err(InsuranceError::InvalidCoverageAmount); }
            }
            CoverageType::Life => {
                if prem < 500_000 || prem > 1_000_000_000 { return Err(InsuranceError::InvalidPremiumAmount); }
                if cov < 50_000_000 || cov > 500_000_000_000 { return Err(InsuranceError::InvalidCoverageAmount); }
            }
            CoverageType::Property => {
                if prem < 2_000_000 || prem > 2_000_000_000 { return Err(InsuranceError::InvalidPremiumAmount); }
                if cov < 100_000_000 || cov > 1_000_000_000_000 { return Err(InsuranceError::InvalidCoverageAmount); }
            }
            CoverageType::Auto => {
                if prem < 1_500_000 || prem > 750_000_000 { return Err(InsuranceError::InvalidPremiumAmount); }
                if cov < 20_000_000 || cov > 200_000_000_000 { return Err(InsuranceError::InvalidCoverageAmount); }
            }
            CoverageType::Liability => {
                if prem < 800_000 || prem > 400_000_000 { return Err(InsuranceError::InvalidPremiumAmount); }
                if cov < 5_000_000 || cov > 50_000_000_000 { return Err(InsuranceError::InvalidCoverageAmount); }
            }
        }
        Ok(())
    }
}