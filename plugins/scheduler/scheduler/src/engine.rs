use std::collections::VecDeque;
use std::env;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::{Duration, Instant};

use crate::core::{
    dsl::{Scenario, UserLifetimeMode},
    error::SchedulerError,
    state_machine::StateMachine,
    wbs::WbsTree,
    workbook::Workbook,
};
use crate::event_bus;
use crate::host_http::WitHttpActionComponent;
use crate::{
    ActionComponent, ActionContext, ActionTrace, IpPoolManager, SchedulerEvent, TemplateContext,
    parse_duration,
};
use indexmap::{IndexMap, IndexSet};

#[derive(Debug, Clone)]
pub struct SchedulerPipeline {
    scenario: Scenario,
    workbook: Workbook,
    template: TemplateContext,
    wbs: WbsTree,
    state_machine: StateMachine,
}

#[derive(Debug, Clone, Copy)]
pub struct LoadRuntimeOptions {
    pub allow_source_ip_binding: bool,
}

impl LoadRuntimeOptions {
    pub fn from_env() -> Self {
        Self {
            allow_source_ip_binding: source_ip_binding_enabled(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct LoadExecutionSummary {
    pub scenario_name: String,
    pub total_users: usize,
    pub traces: Vec<ActionTrace>,
    pub ip_binding: IpBindingSummary,
}

#[derive(Debug, Clone, Default)]
pub struct IpBindingSummary {
    pub requested: bool,
    pub permitted: bool,
    pub pool_stats: Vec<String>,
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

    pub fn run_load_default(&mut self) -> Result<LoadExecutionSummary, SchedulerError> {
        let mut component = WitHttpActionComponent::default();
        self.run_load_with_component(&mut component, LoadRuntimeOptions::from_env())
    }

    pub fn run_load_with_component<C>(
        &mut self,
        component: &mut C,
        options: LoadRuntimeOptions,
    ) -> Result<LoadExecutionSummary, SchedulerError>
    where
        C: ActionComponent,
    {
        let scenario_name = self.scenario.name.clone();
        let Some(load_cfg) = self.scenario.load.clone() else {
            return Err(SchedulerError::InvalidConfiguration(
                "Load configuration is required for runtime execution".into(),
            ));
        };

        let think_time = parse_duration(&load_cfg.user_lifetime.think_time)
            .map_err(|e| SchedulerError::InvalidConfiguration(e.to_string()))?;

        let iterations = match load_cfg.user_lifetime.mode {
            UserLifetimeMode::Once => 1,
            UserLifetimeMode::Loop => {
                if load_cfg.user_lifetime.iterations == 0 {
                    usize::MAX
                } else {
                    load_cfg.user_lifetime.iterations
                }
            }
        };

        let ip_binding_requested = load_cfg.user_resources.ip_binding.enabled;
        let mut available_pool_ids = IndexSet::new();
        for pool in &self.scenario.workbook.ip_pools {
            available_pool_ids.insert(pool.id.clone());
        }

        if ip_binding_requested
            && !available_pool_ids.contains(&load_cfg.user_resources.ip_binding.pool_id)
        {
            return Err(SchedulerError::InvalidConfiguration(format!(
                "IP pool '{}' not found in workbook",
                load_cfg.user_resources.ip_binding.pool_id
            )));
        }

        let mut ip_summary = IpBindingSummary {
            requested: ip_binding_requested,
            permitted: false,
            pool_stats: Vec::new(),
        };

        let mut ip_manager = if ip_binding_requested && options.allow_source_ip_binding {
            let mut manager = IpPoolManager::new();
            manager
                .initialize_from_config(&self.scenario.workbook.ip_pools)
                .map_err(|e| SchedulerError::InvalidConfiguration(e.to_string()))?;
            ip_summary.permitted = true;
            Some(manager)
        } else {
            None
        };

        if ip_binding_requested && !ip_summary.permitted {
            println!(
                "⚠️  已请求 IP 绑定，但当前运行环境不支持自定义源 IP，将自动跳过绑定 (设置 NTX_ENABLE_SOURCE_IP_BINDING=1 可启用)。"
            );
        }

        let mut all_traces = Vec::new();
        let mut total_users = 0usize;

        println!("🚀 Running load test: {}", scenario_name);
        println!("Ramp-up phases: {}", load_cfg.ramp_up.phases.len());
        println!("User lifetime mode: {:?}", load_cfg.user_lifetime.mode);
        println!("Iterations: {}", load_cfg.user_lifetime.iterations);
        println!("Think time: {}", load_cfg.user_lifetime.think_time);

        for phase in &load_cfg.ramp_up.phases {
            println!(
                "\n📊 Phase at {}s: Spawning {} users...",
                phase.at_second, phase.spawn_users
            );

            for _ in 0..phase.spawn_users {
                total_users += 1;
                let user_id = total_users;
                let tenant_id = phase
                    .tenant_id
                    .clone()
                    .unwrap_or_else(|| "default-tenant".to_string());

                let allocated_ip = if ip_summary.permitted {
                    let pool_id = phase
                        .ip_pool_override
                        .as_deref()
                        .unwrap_or(&load_cfg.user_resources.ip_binding.pool_id);
                    if !available_pool_ids.contains(pool_id) {
                        eprintln!(
                            "⚠️  IP pool '{}' not defined, skip allocation for user-{}",
                            pool_id, user_id
                        );
                        None
                    } else {
                        match ip_manager
                            .as_mut()
                            .expect("ip_manager must exist when permitted")
                            .allocate_ip(pool_id, &tenant_id, &format!("user-{}", user_id))
                        {
                            Ok(addr) => Some((addr, pool_id.to_string())),
                            Err(err) => {
                                eprintln!(
                                    "⚠️  Failed to allocate IP for user-{}: {:#}",
                                    user_id, err
                                );
                                None
                            }
                        }
                    }
                } else {
                    None
                };

                let mut base_overrides = IndexMap::new();
                base_overrides.insert("user.id".to_string(), user_id.to_string());
                base_overrides.insert("tenant.id".to_string(), tenant_id.clone());
                if let Some((ip, _)) = allocated_ip {
                    base_overrides.insert("user.allocated_ip".to_string(), ip.to_string());
                }

                let iteration_label = if iterations == usize::MAX {
                    "∞".to_string()
                } else {
                    iterations.to_string()
                };
                println!(
                    "  ↳ user-{} (tenant: {}) iterations={}",
                    user_id, tenant_id, iteration_label
                );

                let infinite_iterations = iterations == usize::MAX;
                let mut iteration_counter = 0usize;

                loop {
                    if iteration_counter > 0 && !think_time.is_zero() {
                        thread::sleep(think_time);
                    }

                    iteration_counter += 1;

                    let mut iteration_overrides = base_overrides.clone();
                    iteration_overrides
                        .insert("user.iteration".to_string(), iteration_counter.to_string());
                    iteration_overrides.insert(
                        "user.iteration_index".to_string(),
                        (iteration_counter - 1).to_string(),
                    );

                    println!("    • user-{} iteration {}", user_id, iteration_counter);

                    let traces = self.run_with_overrides(component, &iteration_overrides)?;
                    all_traces.extend(traces);

                    if !infinite_iterations && iteration_counter >= iterations {
                        break;
                    }
                }

                if let Some((ip, pool_id)) = allocated_ip {
                    if let Some(manager) = ip_manager.as_mut() {
                        if let Err(err) = manager.release_ip(&pool_id, ip) {
                            eprintln!(
                                "⚠️  Failed to release IP {} for user-{}: {:#}",
                                ip, user_id, err
                            );
                        }
                    }
                }
            }
        }

        if let Some(manager) = ip_manager {
            ip_summary.pool_stats = manager.get_all_stats();
        }

        Ok(LoadExecutionSummary {
            scenario_name,
            total_users,
            traces: all_traces,
            ip_binding: ip_summary,
        })
    }

    pub fn run_default(&mut self) -> Result<Vec<ActionTrace>, SchedulerError> {
        let mut component = WitHttpActionComponent::default();
        self.run(&mut component)
    }

    pub fn run<C>(&mut self, component: &mut C) -> Result<Vec<ActionTrace>, SchedulerError>
    where
        C: ActionComponent,
    {
        let overrides = IndexMap::new();
        self.run_with_overrides(component, &overrides)
    }

    pub fn run_with_overrides<C>(
        &mut self,
        component: &mut C,
        overrides: &IndexMap<String, String>,
    ) -> Result<Vec<ActionTrace>, SchedulerError>
    where
        C: ActionComponent,
    {
        component
            .init()
            .map_err(|source| SchedulerError::ActionComponentInit { source })?;

        let shutdown = setup_shutdown_flag()?;
        let merged_template = self.template.merged(overrides);
        let run_result = TaskExecutor::new(
            component,
            &mut self.wbs,
            &mut self.state_machine,
            merged_template,
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

const EVENT_BUS_DRAIN_LIMIT: u32 = 128;
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

fn setup_shutdown_flag() -> Result<Arc<AtomicBool>, SchedulerError> {
    // Signal handling not available in WASM
    Ok(Arc::new(AtomicBool::new(false)))
}

fn source_ip_binding_enabled() -> bool {
    match env::var("NTX_ENABLE_SOURCE_IP_BINDING") {
        Ok(value) => matches!(value.to_ascii_lowercase().as_str(), "1" | "true" | "yes"),
        Err(_) => false,
    }
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

        println!(
            "[Scheduler] → action task={} action={} priority={}",
            task_id, action_id, ACTION_PRIORITY
        );

        let action = self
            .wbs
            .get_action(&action_id)
            .cloned()
            .ok_or_else(|| SchedulerError::ActionNotRegistered(action_id.clone()))?;

        let action = self.template.render_action(&action);

        let start = Instant::now();

        let wbs_view: &WbsTree = &self.wbs;
        let mut ctx = ActionContext::new(wbs_view);
        let outcome = self
            .component
            .do_action(&action, &mut ctx)
            .map_err(|source| SchedulerError::ActionExecution {
                action: action_id.clone(),
                source,
            })?;

        let duration = start.elapsed();

        let mut emitted_events = ctx.into_events();
        if !emitted_events.is_empty() {
            println!(
                "[Scheduler]   ActionContext emitted {} event(s) from {}",
                emitted_events.len(),
                action_id
            );
        }
        let bus_events = event_bus::drain_scheduler_events(EVENT_BUS_DRAIN_LIMIT)?;
        if !bus_events.is_empty() {
            println!(
                "[Scheduler]   Drained {} event(s) from event bus after {}",
                bus_events.len(),
                action_id
            );
        }
        emitted_events.extend(bus_events);

        for event in emitted_events {
            println!(
                "[Scheduler]   enqueue event -> {} at priority {}",
                describe_event(&event),
                EVENT_PRIORITY
            );
            self.queues
                .push(ScheduledTask::event(event, EVENT_PRIORITY));
        }

        self.traces.push(ActionTrace {
            task_id: task.id.clone(),
            action_id: action_id.clone(),
            status: outcome.status,
            detail: outcome.detail,
            duration_ms: duration.as_millis() as u64,
        });

        println!(
            "[Scheduler] ← action {} finished status={:?} duration={}ms",
            action_id,
            outcome.status,
            duration.as_millis()
        );

        self.enqueue_new_action_tasks();
        Ok(())
    }

    fn execute_event(&mut self, event: SchedulerEvent) -> Result<(), SchedulerError> {
        println!("[Scheduler] ↻ applying event {}", describe_event(&event));
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

fn describe_event(event: &SchedulerEvent) -> String {
    match event {
        SchedulerEvent::RegisterAction(action) => {
            format!("register-action id={}", action.id)
        }
        SchedulerEvent::AddTask(task) => format!("add-task id={}", task.id),
        SchedulerEvent::RemoveTask { task_id } => {
            format!("remove-task id={}", task_id)
        }
        SchedulerEvent::UpdateTask(task) => format!("update-task id={}", task.id),
        SchedulerEvent::AddEdge { from_id, edge } => {
            format!("add-edge from={} -> {}", from_id, edge.target)
        }
        SchedulerEvent::RemoveEdge { from_id, target } => {
            format!("remove-edge from={} target={}", from_id, target)
        }
    }
}
