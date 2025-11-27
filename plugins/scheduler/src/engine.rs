use std::collections::VecDeque;

use indexmap::IndexSet;

use crate::{
    dsl::Scenario,
    error::SchedulerError,
    executor::{ActionComponent, ActionContext, ActionTrace, DefaultActionComponent},
    state_machine::StateMachine,
    wbs::WbsTree,
    workbook::Workbook,
};

#[derive(Debug, Clone)]
pub struct SchedulerPipeline {
    scenario: Scenario,
    workbook: Workbook,
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
        let wbs = WbsTree::build(&scenario)?;
        let state_machine = StateMachine::from_wbs(&wbs);

        Ok(Self {
            scenario,
            workbook,
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

    pub fn wbs(&self) -> &WbsTree {
        &self.wbs
    }

    pub fn state_machine(&self) -> &StateMachine {
        &self.state_machine
    }

    pub fn run_default(&mut self) -> Result<Vec<ActionTrace>, SchedulerError> {
        let mut component = DefaultActionComponent::default();
        self.run(&mut component)
    }

    pub fn run<C>(&mut self, component: &mut C) -> Result<Vec<ActionTrace>, SchedulerError>
    where
        C: ActionComponent,
    {
        component
            .init()
            .map_err(|source| SchedulerError::ActionComponentInit { source })?;

        let run_result = self.execute(component);
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

    fn execute<C>(&mut self, component: &mut C) -> Result<Vec<ActionTrace>, SchedulerError>
    where
        C: ActionComponent,
    {
        let mut traces = Vec::new();
        let mut seen: IndexSet<String> = IndexSet::new();
        let mut queue: VecDeque<String> = self
            .wbs
            .action_task_ids()
            .into_iter()
            .map(|id| {
                seen.insert(id.clone());
                id
            })
            .collect();

        while let Some(task_id) = queue.pop_front() {
            let task_opt = self.wbs.get_task(&task_id).cloned();
            let Some(task) = task_opt else {
                continue;
            };

            let Some(action_id) = task.action_id.clone() else {
                continue;
            };

            let action = self
                .wbs
                .get_action(&action_id)
                .cloned()
                .ok_or_else(|| SchedulerError::ActionNotRegistered(action_id.clone()))?;

            let mut ctx = ActionContext::new(&mut self.wbs, &mut self.state_machine);
            let outcome = component.do_action(&action, &mut ctx).map_err(|source| {
                SchedulerError::ActionExecution {
                    action: action_id.clone(),
                    source,
                }
            })?;

            traces.push(ActionTrace {
                task_id: task.id.clone(),
                action_id: action_id.clone(),
                status: outcome.status,
                detail: outcome.detail,
            });

            for id in self.wbs.action_task_ids() {
                if !seen.contains(&id) {
                    seen.insert(id.clone());
                    queue.push_back(id);
                }
            }
        }

        Ok(traces)
    }

    pub fn summary(&self) -> PipelineSummary {
        PipelineSummary {
            resources: self.workbook.resource_count(),
            metrics: self.workbook.metric_count(),
            tasks: self.wbs.task_count(),
            edges: self.state_machine.transition_count(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PipelineSummary {
    pub resources: usize,
    pub metrics: usize,
    pub tasks: usize,
    pub edges: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::{ActionComponent, ActionContext, ActionOutcome};
    use crate::wbs::{WbsEdge, WbsTask, WbsTaskKind};
    use anyhow::Result;

    const SAMPLE: &str = include_str!("../res/http_scenario.yaml");

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
            action: &crate::dsl::ActionDef,
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
