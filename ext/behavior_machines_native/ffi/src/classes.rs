//! The node `Data` classes, resolved once at load and pinned.
use magnus::{Error, RClass, RModule, Ruby};
use matryona_machines::ruby::{Class, Registry};

/// Every node class, in the order of [`ALL_KINDS`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Leaf,
    AlwaysSuccess,
    AlwaysFailure,
    Sleep,
    Sequence,
    ReactiveSequence,
    Fallback,
    ReactiveFallback,
    Parallel,
    IfThenElse,
    WhileDoElse,
    Inverter,
    ForceSuccess,
    ForceFailure,
    Timeout,
    Repeat,
    RetryUntilSuccessful,
    KeepRunningUntilFailure,
}

/// How many node classes there are.
pub const KINDS: usize = 18;

/// Every node class, in the order the node module lists them.
pub const ALL_KINDS: [Kind; KINDS] = [
    Kind::Leaf,
    Kind::AlwaysSuccess,
    Kind::AlwaysFailure,
    Kind::Sleep,
    Kind::Sequence,
    Kind::ReactiveSequence,
    Kind::Fallback,
    Kind::ReactiveFallback,
    Kind::Parallel,
    Kind::IfThenElse,
    Kind::WhileDoElse,
    Kind::Inverter,
    Kind::ForceSuccess,
    Kind::ForceFailure,
    Kind::Timeout,
    Kind::Repeat,
    Kind::RetryUntilSuccessful,
    Kind::KeepRunningUntilFailure,
];

impl Kind {
    /// The class's constant name under `BehaviorMachines::Node`.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Leaf => "Leaf",
            Self::AlwaysSuccess => "AlwaysSuccess",
            Self::AlwaysFailure => "AlwaysFailure",
            Self::Sleep => "Sleep",
            Self::Sequence => "Sequence",
            Self::ReactiveSequence => "ReactiveSequence",
            Self::Fallback => "Fallback",
            Self::ReactiveFallback => "ReactiveFallback",
            Self::Parallel => "Parallel",
            Self::IfThenElse => "IfThenElse",
            Self::WhileDoElse => "WhileDoElse",
            Self::Inverter => "Inverter",
            Self::ForceSuccess => "ForceSuccess",
            Self::ForceFailure => "ForceFailure",
            Self::Timeout => "Timeout",
            Self::Repeat => "Repeat",
            Self::RetryUntilSuccessful => "RetryUntilSuccessful",
            Self::KeepRunningUntilFailure => "KeepRunningUntilFailure",
        }
    }

    /// How many fields the class has.
    pub const fn arity(self) -> usize {
        match self {
            Self::AlwaysSuccess | Self::AlwaysFailure => 0,
            Self::Leaf
            | Self::Sleep
            | Self::Sequence
            | Self::ReactiveSequence
            | Self::Fallback
            | Self::ReactiveFallback
            | Self::Inverter
            | Self::ForceSuccess
            | Self::ForceFailure
            | Self::KeepRunningUntilFailure => 1,
            Self::Timeout | Self::Repeat | Self::RetryUntilSuccessful => 2,
            Self::Parallel | Self::IfThenElse | Self::WhileDoElse => 3,
        }
    }
}

/// Every node class, pinned.
pub struct Classes {
    nodes: [Class; KINDS],
}

static CLASSES: Registry<Classes> = Registry::new("behavior_machines");

/// Resolve the classes `behavior_machines.rb` defined under `Node` before
/// requiring the extension. Called once, from init; never by name again.
pub fn resolve(ruby: &Ruby, scope: RModule) -> Result<(), Error> {
    let mut nodes = Vec::with_capacity(KINDS);
    for kind in ALL_KINDS {
        nodes.push(Class::resolve(ruby, scope, kind.name())?);
    }
    let Ok(nodes) = <[Class; KINDS]>::try_from(nodes) else {
        let message = "every node class resolves once";
        return Err(Error::new(ruby.exception_runtime_error(), message));
    };
    CLASSES.set(ruby, Classes { nodes }).map(drop)
}

/// The node classes, in the order of [`ALL_KINDS`].
pub fn nodes(ruby: &Ruby) -> Result<[RClass; KINDS], Error> {
    Ok(CLASSES.get(ruby)?.nodes.map(|class| class.get(ruby)))
}
