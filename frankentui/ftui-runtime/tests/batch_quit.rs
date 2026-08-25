use ftui_core::event::Event;
use ftui_render::frame::Frame;
use ftui_runtime::program::{Cmd, Model, TaskSpec};
use ftui_runtime::simulator::ProgramSimulator;
use std::time::Duration;

struct TestModel {
    executed_after_quit: bool,
}

#[derive(Debug)]
enum TestMsg {
    QuitInBatch,
    SetExecuted,
}

impl From<Event> for TestMsg {
    fn from(_: Event) -> Self {
        TestMsg::QuitInBatch
    }
}

impl Model for TestModel {
    type Message = TestMsg;

    fn update(&mut self, msg: Self::Message) -> Cmd<Self::Message> {
        match msg {
            TestMsg::QuitInBatch => Cmd::Batch(vec![
                Cmd::Quit,
                Cmd::Msg(TestMsg::SetExecuted), // Should NOT be executed
            ]),
            TestMsg::SetExecuted => {
                self.executed_after_quit = true;
                Cmd::None
            }
        }
    }

    fn view(&self, _frame: &mut Frame) {}
}

#[test]
fn batch_stops_after_quit() {
    let mut sim = ProgramSimulator::new(TestModel {
        executed_after_quit: false,
    });
    sim.init();

    sim.send(TestMsg::QuitInBatch);

    // Check if the model state changed after quit
    assert!(
        !sim.model().executed_after_quit,
        "Commands after Quit in Batch should not be executed"
    );
    assert!(!sim.is_running(), "Simulator should have stopped");
}

// -------------------------------------------------------------------------
// Cmd::Task + effect ordering + tick scheduling (bd-1av4o.2.2)
// -------------------------------------------------------------------------

struct TaskOrderModel {
    trace: Vec<&'static str>,
}

#[derive(Debug)]
enum TaskOrderMsg {
    Start,
    TaskDone,
    Followup,
    AfterTask,
}

impl From<Event> for TaskOrderMsg {
    fn from(_: Event) -> Self {
        TaskOrderMsg::Start
    }
}

impl Model for TaskOrderModel {
    type Message = TaskOrderMsg;

    fn update(&mut self, msg: Self::Message) -> Cmd<Self::Message> {
        match msg {
            TaskOrderMsg::Start => Cmd::Batch(vec![
                Cmd::task_named("task-order", || TaskOrderMsg::TaskDone),
                Cmd::Msg(TaskOrderMsg::AfterTask),
            ]),
            TaskOrderMsg::TaskDone => {
                self.trace.push("task");
                Cmd::Msg(TaskOrderMsg::Followup)
            }
            TaskOrderMsg::Followup => {
                self.trace.push("followup");
                Cmd::None
            }
            TaskOrderMsg::AfterTask => {
                self.trace.push("after");
                Cmd::None
            }
        }
    }

    fn view(&self, _frame: &mut Frame) {}
}

#[test]
fn task_executes_before_following_batch_command() {
    let mut sim = ProgramSimulator::new(TaskOrderModel { trace: Vec::new() });
    sim.init();

    sim.send(TaskOrderMsg::Start);

    assert_eq!(sim.model().trace, vec!["task", "followup", "after"]);
}

struct TickTaskModel {
    ticks: usize,
    trace: Vec<&'static str>,
}

#[derive(Debug)]
enum TickTaskMsg {
    Start,
    TaskDone,
    Tick,
}

impl From<Event> for TickTaskMsg {
    fn from(event: Event) -> Self {
        match event {
            Event::Tick => TickTaskMsg::Tick,
            _ => TickTaskMsg::Start,
        }
    }
}

impl Model for TickTaskModel {
    type Message = TickTaskMsg;

    fn update(&mut self, msg: Self::Message) -> Cmd<Self::Message> {
        match msg {
            TickTaskMsg::Start => Cmd::Batch(vec![
                Cmd::Tick(Duration::from_millis(50)),
                Cmd::task_named("tick-task", || TickTaskMsg::TaskDone),
            ]),
            TickTaskMsg::TaskDone => {
                self.trace.push("task");
                Cmd::Tick(Duration::from_millis(10))
            }
            TickTaskMsg::Tick => {
                self.ticks += 1;
                self.trace.push("tick");
                Cmd::None
            }
        }
    }

    fn view(&self, _frame: &mut Frame) {}
}

#[test]
fn tick_scheduling_is_deterministic_and_task_driven() {
    let mut sim = ProgramSimulator::new(TickTaskModel {
        ticks: 0,
        trace: Vec::new(),
    });
    sim.init();

    sim.send(TickTaskMsg::Start);

    // The task-triggered tick should override the initial tick rate.
    assert_eq!(sim.tick_rate(), Some(Duration::from_millis(10)));

    // Ticks are only delivered when injected (no wall-clock dependency).
    sim.inject_event(Event::Tick);
    sim.inject_event(Event::Tick);

    assert_eq!(sim.model().ticks, 2);
    assert_eq!(sim.tick_rate(), Some(Duration::from_millis(10)));
    assert_eq!(sim.model().trace, vec!["task", "tick", "tick"]);
}

// -------------------------------------------------------------------------
// Cmd + TaskSpec coverage (bd-1av4o.2)
// -------------------------------------------------------------------------

#[test]
fn task_spec_builder_sets_fields() {
    let spec = TaskSpec::new(2.5, 12.0).with_name("demo-task");
    assert_eq!(spec.weight, 2.5);
    assert_eq!(spec.estimate_ms, 12.0);
    assert_eq!(spec.name.as_deref(), Some("demo-task"));
}

#[test]
fn cmd_task_variants_capture_spec() {
    let cmd_named: Cmd<TestMsg> = Cmd::task_named("named", || TestMsg::SetExecuted);
    match cmd_named {
        Cmd::Task(spec, _) => {
            assert_eq!(spec.name.as_deref(), Some("named"));
            assert_eq!(spec.weight, 1.0);
            assert_eq!(spec.estimate_ms, 10.0);
        }
        _ => panic!("expected Cmd::Task for task_named"),
    }

    let cmd_weighted: Cmd<TestMsg> = Cmd::task_weighted(3.0, 7.5, || TestMsg::SetExecuted);
    match cmd_weighted {
        Cmd::Task(spec, _) => {
            assert_eq!(spec.name.as_deref(), None);
            assert_eq!(spec.weight, 3.0);
            assert_eq!(spec.estimate_ms, 7.5);
        }
        _ => panic!("expected Cmd::Task for task_weighted"),
    }

    let spec = TaskSpec::new(4.0, 9.0).with_name("spec-task");
    let cmd_spec: Cmd<TestMsg> = Cmd::task_with_spec(spec, || TestMsg::SetExecuted);
    match cmd_spec {
        Cmd::Task(spec, _) => {
            assert_eq!(spec.name.as_deref(), Some("spec-task"));
            assert_eq!(spec.weight, 4.0);
            assert_eq!(spec.estimate_ms, 9.0);
        }
        _ => panic!("expected Cmd::Task for task_with_spec"),
    }
}

#[test]
fn cmd_count_matches_nested_structure() {
    let cmd: Cmd<TestMsg> = Cmd::Batch(vec![
        Cmd::None,
        Cmd::Msg(TestMsg::SetExecuted),
        Cmd::Sequence(vec![
            Cmd::None,
            Cmd::Msg(TestMsg::QuitInBatch),
            Cmd::Batch(vec![
                Cmd::Msg(TestMsg::SetExecuted),
                Cmd::Msg(TestMsg::SetExecuted),
            ]),
        ]),
    ]);

    // None counts as 0; each Msg counts as 1; Batch/Sequence sum their children.
    assert_eq!(cmd.count(), 4);
}
