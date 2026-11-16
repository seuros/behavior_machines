use core::time::Duration;

use crate::Status;

/// A task the tree runs: a condition check or an action.
///
/// # Activation
///
/// A leaf is *active* from a tick that returns [`Status::Running`] until a
/// later tick returns [`Status::Success`] or [`Status::Failure`], or the tree
/// halts it. The tree guarantees that:
///
/// - a tick on an inactive leaf starts a new activation;
/// - [`Leaf::halt`] is called only on an active leaf, exactly once per
///   activation, during the same call in which the tree abandons it.
///
/// Leaves are shared, immutable definitions. Keep per-activation state, such
/// as a spawned task handle, in the context `C`.
pub trait Leaf<C: ?Sized> {
    /// Advances the leaf. `now` is the host's monotonic reading, since an
    /// epoch the host picks.
    fn tick(&self, ctx: &mut C, now: Duration) -> Status;

    /// Stops an active leaf that the tree is abandoning: cancel work, release
    /// resources, and clear activation state. The default does nothing, which
    /// suits leaves that never return [`Status::Running`].
    fn halt(&self, _ctx: &mut C) {}
}

impl<C: ?Sized, L: Leaf<C> + ?Sized> Leaf<C> for &L {
    fn tick(&self, ctx: &mut C, now: Duration) -> Status {
        (**self).tick(ctx, now)
    }

    fn halt(&self, ctx: &mut C) {
        (**self).halt(ctx);
    }
}

#[cfg(any(feature = "alloc", test))]
impl<C: ?Sized, L: Leaf<C> + ?Sized> Leaf<C> for alloc::boxed::Box<L> {
    fn tick(&self, ctx: &mut C, now: Duration) -> Status {
        (**self).tick(ctx, now)
    }

    fn halt(&self, ctx: &mut C) {
        (**self).halt(ctx);
    }
}
