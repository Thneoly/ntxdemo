//! Typed event topics/kinds used by the scheduler component.
//!
//! Goal: avoid stringly-typed topic usage across the codebase.
//! We still lower to `&'static str` when talking to the eventbus.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum EventKind {
    // Packet
    PacketRx,
    PacketTxRequest,

    // Scheduling
    SendScheduleRequest,
    SendScheduled,

    // Scheduler lifecycle/state
    SchedulerStateChanged,
    SchedulerActionResult,
    SchedulerTaskStateChanged,

    // Control
    SchedulerControlStop,
    SchedulerControlPause,
    SchedulerControlResume,

    // Timers
    SchedulerTimerTimeout,
    SchedulerTimerRetry,
    SchedulerTimerThink,

    // Users
    SchedulerUserStart,
    SchedulerUserExit,

    // Resources
    SchedulerResourceBound,
    SchedulerResourceReleased,

    // Topology
    TopologyChanged,
    SchedulerTopologyApplied,
    SchedulerTopologyRejected,
}

impl EventKind {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            EventKind::PacketRx => "packet.rx",
            EventKind::PacketTxRequest => "packet.tx-request",
            EventKind::SendScheduleRequest => "send.schedule-request",
            EventKind::SendScheduled => "send.scheduled",
            EventKind::SchedulerStateChanged => "scheduler.state-changed",
            EventKind::SchedulerActionResult => "scheduler.action-result",
            EventKind::SchedulerTaskStateChanged => "scheduler.task.state-changed",
            EventKind::SchedulerControlStop => "scheduler.control.stop",
            EventKind::SchedulerControlPause => "scheduler.control.pause",
            EventKind::SchedulerControlResume => "scheduler.control.resume",
            EventKind::SchedulerTimerTimeout => "scheduler.timer.timeout",
            EventKind::SchedulerTimerRetry => "scheduler.timer.retry",
            EventKind::SchedulerTimerThink => "scheduler.timer.think",
            EventKind::SchedulerUserStart => "scheduler.user.start",
            EventKind::SchedulerUserExit => "scheduler.user.exit",
            EventKind::SchedulerResourceBound => "scheduler.resource-bound",
            EventKind::SchedulerResourceReleased => "scheduler.resource-released",
            EventKind::TopologyChanged => "topology.changed",
            EventKind::SchedulerTopologyApplied => "scheduler.topology.applied",
            EventKind::SchedulerTopologyRejected => "scheduler.topology.rejected",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum TopicFilter {
    Exact(EventKind),

    // Wildcards used by the best-effort eventbus.
    SchedulerControlAll,
    SchedulerTimerAll,
    SchedulerUserAll,
}

impl TopicFilter {
    pub(crate) const fn as_filter_str(self) -> &'static str {
        match self {
            TopicFilter::Exact(k) => k.as_str(),
            TopicFilter::SchedulerControlAll => "scheduler.control.*",
            TopicFilter::SchedulerTimerAll => "scheduler.timer.*",
            TopicFilter::SchedulerUserAll => "scheduler.user.*",
        }
    }
}
