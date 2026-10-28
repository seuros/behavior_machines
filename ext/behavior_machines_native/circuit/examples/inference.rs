//! A reactive guard around fake asynchronous work. No model, device adapter,
//! runtime, network, or wall clock is needed.

use behavior_machines::{Node, Status, Time};
use behavior_machines_circuit::{Action, Condition, ExecutionLeaf, Executor};

#[derive(Default)]
struct Host {
    resources_allowed: bool,
    next_ticket: u64,
    active: Option<u64>,
    completion: Option<u64>,
    cancelled: Vec<u64>,
}

struct ResourcesAllowed;

impl Condition<Host> for ResourcesAllowed {
    fn evaluate(&self, host: &Host, _now: Time) -> Status {
        if host.resources_allowed {
            Status::Success
        } else {
            Status::Failure
        }
    }
}

struct Generate;

impl Action<Host> for Generate {
    fn start(&self, host: &mut Host, _now: Time) -> Status {
        host.next_ticket += 1;
        host.active = Some(host.next_ticket);
        // A real host starts inference here, retaining its cancellation handle
        // and tagging completion with this activation's ticket.
        Status::Running
    }

    fn poll(&self, host: &mut Host, _now: Time) -> Status {
        // Taking the completion also discards stale results, which must not
        // settle the current activation.
        if let Some(ticket) = host.completion.take()
            && host.active == Some(ticket)
        {
            host.active = None;
            return Status::Success;
        }
        Status::Running
    }

    fn halt(&self, host: &mut Host) {
        if let Some(ticket) = host.active.take() {
            // Request cancellation AND invalidate the ticket synchronously.
            host.cancelled.push(ticket);
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut executor = Executor::new(Node::reactive_sequence([
        Node::leaf(ExecutionLeaf::Condition(ResourcesAllowed)),
        Node::leaf(ExecutionLeaf::Action(Generate)),
    ]))?;
    let mut host = Host {
        resources_allowed: true,
        ..Host::default()
    };

    // An explicit request starts one activation.
    assert_eq!(
        executor.tick(&mut host, Time::ZERO)?.root(),
        Some(Status::Running),
    );
    assert_eq!(host.active, Some(1));

    // A hard resource guard preempts the running operation.
    host.resources_allowed = false;
    assert_eq!(
        executor.tick(&mut host, Time::from_nanos(1))?.root(),
        Some(Status::Failure),
    );
    assert_eq!(host.active, None);
    assert_eq!(host.cancelled, [1]);

    // The host explicitly resumes after resources recover. Merely updating
    // the observation does not tick the executor or launch work.
    host.resources_allowed = true;
    executor.tick(&mut host, Time::from_nanos(2))?;
    assert_eq!(host.active, Some(2));

    // A late completion for activation 1 cannot finish activation 2.
    host.completion = Some(1);
    assert_eq!(
        executor.tick(&mut host, Time::from_nanos(3))?.root(),
        Some(Status::Running),
    );
    assert_eq!(host.active, Some(2));

    // The actual operation finishes; a later halt must not cancel it again.
    host.completion = Some(2);
    assert_eq!(
        executor.tick(&mut host, Time::from_nanos(4))?.root(),
        Some(Status::Success),
    );
    executor.halt(&mut host);
    executor.halt(&mut host);
    assert_eq!(host.cancelled, [1]);
    assert!(!executor.is_running());
    println!("Start, preempt, resume, reject stale completion, finish: OK");
    Ok(())
}
