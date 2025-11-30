use std::collections::VecDeque;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::Duration;

use crate::TemplateContext;
#[cfg(not(target_arch = "wasm32"))]
use ctrlc;
use indexmap::IndexSet;
use scheduler_actions_http::HttpActionComponent;
use scheduler_core::{
    dsl::Scenario, error::SchedulerError, state_machine::StateMachine, wbs::WbsTree,
    workbook::Workbook,
};
use scheduler_executor::{ActionComponent, ActionContext, ActionTrace, SchedulerEvent};

#[derive(Debug, Clone)]
pub struct SchedulerPipeline {
    scenario: Scenario,
    workbook: Workbook,
    template: TemplateContext,
    wbs: WbsTree,
    state_machine: StateMachine,
}

impl SchedulerPipeline {
    pub fn load_from_yaml_str(input: &str) -> Result<Self, SchedulerError> {
        let scenario = Scenario::from_yaml_str(input)?;
        scenario.validate()?;
        Self::from_scenario(scenario)
    }

    pub fn from_scenario(scenario: Scenario) -> Result<Self, SchedulerError> {
        let workbook = Workbook::from_scenario(&scenario);
        let template = TemplateContext::from_workbook(&workbook);
        let wbs = WbsTree::build(&scenario)?;
        let state_machine = StateMachine::from_wbs(&wbs);

        Ok(Self {
            scenario,
            workbook,
            template,
            wbs,
            state_machine,
        })
    }

    pub fn scenario(&self) -> &Scenario {
        &self.scenario
    }

    pub fn workbook(&self) -> &Workbook {
        &self.workbook
    }

    pub fn template_context(&self) -> &TemplateContext {
        &self.template
    }

    pub fn wbs(&self) -> &WbsTree {
        &self.wbs
    }

    pub fn state_machine(&self) -> &StateMachine {
        &self.state_machine
    }

    pub fn summary(&self) -> PipelineSummary {
        PipelineSummary {
            resources: self.workbook.resource_count(),
            metrics: self.workbook.metric_count(),
            tasks: self.wbs.task_count(),
            edges: self.state_machine.transition_count(),
        }
    }

    pub fn run_default(&mut self) -> Result<Vec<ActionTrace>, SchedulerError> {
        let mut component = HttpActionComponent::default();
        self.run(&mut component)
    }

    pub fn run<C>(&mut self, component: &mut C) -> Result<Vec<ActionTrace>, SchedulerError>
    where
        C: ActionComponent,
    {
        component
            .init()
            .map_err(|source| SchedulerError::ActionComponentInit { source })?;

        let shutdown = setup_shutdown_flag()?;
        let run_result = TaskExecutor::new(
            component,
            &mut self.wbs,
            &mut self.state_machine,
            self.template.clone(),
            shutdown,
        )
        .run();

        let release_result = component
            .release()
            .map_err(|source| SchedulerError::ActionComponentRelease { source });
        if let Err(release_err) = release_result {
            if run_result.is_ok() {
                return Err(release_err);
            }
        }

        run_result
    }
}

const PRIORITY_LEVELS: usize = 64;
const ACTION_PRIORITY: u8 = 32;
const EVENT_PRIORITY: u8 = 4;
const IDLE_PRIORITY: u8 = 63;
const IDLE_SPIN_LIMIT: usize = 2;

#[derive(Debug, Clone, Copy)]
pub struct PipelineSummary {
    pub resources: usize,
    pub metrics: usize,
    pub tasks: usize,
    pub edges: usize,
}

#[cfg(not(target_arch = "wasm32"))]
fn setup_shutdown_flag() -> Result<Arc<AtomicBool>, SchedulerError> {
    let flag = Arc::new(AtomicBool::new(false));
    let handler_flag = flag.clone();
    ctrlc::set_handler(move || {
        handler_flag.store(true, Ordering::SeqCst);
    })
    .map_err(|source| SchedulerError::SignalHandler { source })?;
    Ok(flag)
}

#[cfg(target_arch = "wasm32")]
fn setup_shutdown_flag() -> Result<Arc<AtomicBool>, SchedulerError> {
    // Signal handling not available in WASM
    Ok(Arc::new(AtomicBool::new(false)))
}

struct TaskExecutor<'a, C> {
    component: &'a mut C,
    wbs: &'a mut WbsTree,
    state_machine: &'a mut StateMachine,
    template: TemplateContext,
    queues: PriorityQueues,
    seen_tasks: IndexSet<String>,
    traces: Vec<ActionTrace>,
    shutdown: Arc<AtomicBool>,
}

impl<'a, C> TaskExecutor<'a, C>
where
    C: ActionComponent,
{
    fn new(
        component: &'a mut C,
        wbs: &'a mut WbsTree,
        state_machine: &'a mut StateMachine,
        template: TemplateContext,
        shutdown: Arc<AtomicBool>,
    ) -> Self {
        let mut executor = Self {
            component,
            wbs,
            state_machine,
            template,
            queues: PriorityQueues::new(),
            seen_tasks: IndexSet::new(),
            traces: Vec::new(),
            shutdown,
        };
        executor.enqueue_new_action_tasks();
        executor
    }

    fn run(mut self) -> Result<Vec<ActionTrace>, SchedulerError> {
        let mut idle_spins = 0usize;

        while !self.shutdown.load(Ordering::SeqCst) {
            let task = self
                .queues
                .pop()
                .unwrap_or_else(|| ScheduledTask::idle(IDLE_PRIORITY));

            match task.kind {
                TaskKind::Idle => {
                    self.execute_idle();
                    idle_spins += 1;
                    if idle_spins >= IDLE_SPIN_LIMIT && self.queues.is_empty() {
                        break;
                    }
                }
                _ => {
                    idle_spins = 0;
                    self.dispatch(task)?;
                }
            }
        }

        Ok(self.traces)
    }

    fn dispatch(&mut self, task: ScheduledTask) -> Result<(), SchedulerError> {
        match task.kind {
            TaskKind::Action { task_id } => self.execute_action(task_id),
            TaskKind::Event(event) => self.execute_event(event),
            TaskKind::Idle => Ok(()),
        }
    }

    fn execute_action(&mut self, task_id: String) -> Result<(), SchedulerError> {
        let task_opt = self.wbs.get_task(&task_id).cloned();
        let Some(task) = task_opt else {
            return Ok(());
        };

        let Some(action_id) = task.action_id.clone() else {
            return Ok(());
        };

        let action = self
            .wbs
            .get_action(&action_id)
            .cloned()
            .ok_or_else(|| SchedulerError::ActionNotRegistered(action_id.clone()))?;

        let action = self.template.render_action(&action);

        let wbs_view: &WbsTree = &self.wbs;
        let mut ctx = ActionContext::new(wbs_view);
        let outcome = self
            .component
            .do_action(&action, &mut ctx)
            .map_err(|source| SchedulerError::ActionExecution {
                action: action_id.clone(),
                source,
            })?;

        for event in ctx.into_events() {
            self.queues
                .push(ScheduledTask::event(event, EVENT_PRIORITY));
        }

        self.traces.push(ActionTrace {
            task_id: task.id.clone(),
            action_id: action_id.clone(),
            status: outcome.status,
            detail: outcome.detail,
        });

        self.enqueue_new_action_tasks();
        Ok(())
    }

    fn execute_event(&mut self, event: SchedulerEvent) -> Result<(), SchedulerError> {
        event.apply(self.wbs, self.state_machine)?;
        self.enqueue_new_action_tasks();
        Ok(())
    }

    fn execute_idle(&self) {
        thread::sleep(Duration::from_millis(10));
    }

    fn enqueue_new_action_tasks(&mut self) {
        for id in self.wbs.action_task_ids() {
            if self.seen_tasks.insert(id.clone()) {
                self.queues.push(ScheduledTask::action(id, ACTION_PRIORITY));
            }
        }
    }
}

struct PriorityQueues {
    lanes: [VecDeque<ScheduledTask>; PRIORITY_LEVELS],
}

impl PriorityQueues {
    fn new() -> Self {
        Self {
            lanes: std::array::from_fn(|_| VecDeque::new()),
        }
    }

    fn push(&mut self, task: ScheduledTask) {
        let idx = task.priority.min((PRIORITY_LEVELS - 1) as u8) as usize;
        self.lanes[idx].push_back(task);
    }

    fn pop(&mut self) -> Option<ScheduledTask> {
        for lane in self.lanes.iter_mut() {
            if let Some(task) = lane.pop_front() {
                return Some(task);
            }
        }
        None
    }

    fn is_empty(&self) -> bool {
        self.lanes.iter().all(|lane| lane.is_empty())
    }
}

#[derive(Clone)]
struct ScheduledTask {
    priority: u8,
    kind: TaskKind,
}

impl ScheduledTask {
    fn action(task_id: String, priority: u8) -> Self {
        Self {
            priority,
            kind: TaskKind::Action { task_id },
        }
    }

    fn event(event: SchedulerEvent, priority: u8) -> Self {
        Self {
            priority,
            kind: TaskKind::Event(event),
        }
    }

    fn idle(priority: u8) -> Self {
        Self {
            priority,
            kind: TaskKind::Idle,
        }
    }
}

#[derive(Clone)]
enum TaskKind {
    Action { task_id: String },
    Event(SchedulerEvent),
    Idle,
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use scheduler_core::{
        dsl::ActionDef,
        wbs::{WbsEdge, WbsTask, WbsTaskKind},
    };
    use scheduler_executor::{ActionComponent, ActionContext, ActionOutcome};

    const SAMPLE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../res/http_scenario.yaml"
    ));

    #[test]
    fn pipeline_builds_summary() {
        let pipeline = SchedulerPipeline::load_from_yaml_str(SAMPLE).expect("pipeline");
        let summary = pipeline.summary();
        assert!(summary.resources > 0);
        assert!(summary.tasks > 0);
    }

    struct SpawnComponent {
        spawned: bool,
    }

    impl ActionComponent for SpawnComponent {
        fn init(&mut self) -> Result<()> {
            Ok(())
        }

        fn do_action(
            &mut self,
            action: &ActionDef,
            ctx: &mut ActionContext<'_>,
        ) -> Result<ActionOutcome> {
            if action.id == "probe-get" && !self.spawned {
                self.spawned = true;
                ctx.add_task(WbsTask {
                    id: "dynamic-node".into(),
                    action_id: Some("push-post".into()),
                    kind: WbsTaskKind::Action,
                    outgoing: vec![WbsEdge {
                        target: "end".into(),
                        condition: None,
                        label: Some("dynamic".into()),
                    }],
                });
            }

            Ok(ActionOutcome::success().with_detail(format!("executed {}", action.id)))
        }

        fn release(&mut self) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn run_executes_dynamic_tasks() {
        let scenario = Scenario::from_yaml_str(SAMPLE).expect("scenario");
        let mut pipeline = SchedulerPipeline::from_scenario(scenario).expect("pipeline");
        let mut component = SpawnComponent { spawned: false };
        let traces = pipeline.run(&mut component).expect("run pipeline");
        assert!(traces.iter().any(|trace| trace.task_id == "dynamic-node"));
    }
}
