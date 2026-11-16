//! Symbol names for the core's enums. Every table is exhaustive: a new
//! variant fails to compile here until it has a name.
use behavior_machines::{Halt, Refusal, Status};

matryona_machines::symbols! {
    /// A node's status: what a leaf returns and a tick answers.
    pub Statuses for Status as "a leaf status" {
        Success => "success",
        Failure => "failure",
        Running => "running",
    }
}

matryona_machines::symbols! {
    /// What a halt did.
    pub Halts for Halt as "a halt" {
        Halted => "halted",
        AlreadyIdle => "already_idle",
    }
}

matryona_machines::symbols! {
    /// The core's refusals as Symbols: the snake case of each variant.
    pub Refusals for Refusal as "a refusal" => names_only {
        Full => "full",
        EmptyComposite(..) => "empty_composite",
        ParallelThreshold { .. } => "parallel_threshold",
        Incomplete => "incomplete",
        Trailing => "trailing",
        TimeRegression => "time_regression",
    }
}
