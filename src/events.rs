#[derive(Debug, Clone)]
pub enum CatalystEvent {
    SubscriptionCreated(SubscriptionCreated),
    SubscriptionCancelled(SubscriptionCancelled),
    SubscriptionTransfer(SubscriptionTransfer),
    FixedTransfer(FixedTransfer),
    RecurringTransfer(RecurringTransfer),
    SubscriptionResumed(SubscriptionResumed),
}

#[derive(Debug, Clone)]
pub struct SubscriptionCreated {
    pub plan: String,
    pub subscriber: String,
    pub mint: String,
    pub created_ts: i64,
}

#[derive(Debug, Clone)]
pub struct SubscriptionCancelled {
    pub plan: String,
    pub subscriber: String,
    pub expires_at_ts: i64,
}

#[derive(Debug, Clone)]
pub struct SubscriptionTransfer {
    pub subscription: String,
    pub plan: String,
    pub delegator: String,
    pub mint: String,
    pub amount: u64,
    pub period_start_ts: i64,
    pub period_end_ts: i64,
    pub amount_pulled_in_period: u64,
    pub receiver: String,
}

#[derive(Debug, Clone)]
pub struct FixedTransfer {
    pub delegation: String,
    pub delegator: String,
    pub delegatee: String,
    pub mint: String,
    pub amount: u64,
    pub remaining_amount: u64,
    pub receiver: String,
}

#[derive(Debug, Clone)]
pub struct RecurringTransfer {
    pub delegation: String,
    pub delegator: String,
    pub delegatee: String,
    pub mint: String,
    pub amount: u64,
    pub period_start_ts: i64,
    pub period_end_ts: i64,
    pub amount_pulled_in_period: u64,
    pub receiver: String,
}

#[derive(Debug, Clone)]
pub struct SubscriptionResumed {
    pub plan: String,
    pub subscriber: String,
    pub resumed_ts: i64,
}