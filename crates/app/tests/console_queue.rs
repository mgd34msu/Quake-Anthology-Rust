//! Behavior tests for buffered console dispatch: the host driver drains
//! `wait` countdowns, multi-frame inserts, and script callbacks across
//! host frames, per seat, with donor ordering.

use qa_app::application::Application;
use qa_app::console::commands::{register_console_commands, ConsoleCommandServices, ConsoleCommands};
use qa_app::console::queue::ConsoleQueue;
use qa_app::options::{parse_application_command, ApplicationCommand};
use qa_app::startup::StartupConfig;
use qa_client::render::NullRenderer;
use qa_core::cmd::Dialect;
use qa_core::cmd_buffer::{CommandContext, CommandOrigin};
use qa_core::cvar::CvarRegistry;
use qa_core::identity::IdentityOwner;

struct Fixture {
    printed: Vec<String>,
    forwarded: Vec<String>,
    toggles: usize,
}

impl ConsoleCommandServices for Fixture {
    fn print(&mut self, text: &str) {
        self.printed.push(text.to_string());
    }

    fn forward_to_server(&mut self, line: &str) {
        self.forwarded.push(line.to_string());
    }

    fn toggle_console(&mut self) {
        self.toggles += 1;
    }

    fn clear_console(&mut self) {}

    fn message_mode(&mut self, _team: bool) {}

    fn can_chat(&self) -> bool {
        true
    }

    fn console_dump(&self) -> String {
        String::new()
    }

    fn write_file(&mut self, _path: &str, _contents: &str) -> Result<(), String> {
        Ok(())
    }

    fn configuration_text(&mut self, _argv: &[String]) -> String {
        String::new()
    }

    fn map_name(&self) -> String {
        String::new()
    }
}

fn harness(dialect: Dialect) -> (IdentityOwner, ConsoleQueue, ConsoleCommands, CvarRegistry, Fixture) {
    let owner = IdentityOwner::create("console-queue").unwrap();
    let context = CommandContext::new(
        owner.session().clone(),
        CommandOrigin::LocalSeat {
            seat: owner.seat(0),
            client: owner.client(0, 0),
        },
    );
    let queue = ConsoleQueue::new(dialect, context).unwrap();
    let mut commands = ConsoleCommands::new();
    register_console_commands(&mut commands);
    let cvars = CvarRegistry::new(dialect);
    (
        owner,
        queue,
        commands,
        cvars,
        Fixture {
            printed: Vec::new(),
            forwarded: Vec::new(),
            toggles: 0,
        },
    )
}

#[test]
fn multi_frame_programs_drain_across_host_frames() {
    let (_owner, mut queue, mut commands, mut cvars, mut services) = harness(Dialect::Q3);
    queue.submit("echo one; wait 2; echo two; set played 1\n").unwrap();
    let revision = queue.buffer().program_revision();
    let mut dispatches = 0;
    queue
        .drive_frame(&mut commands, &mut cvars, &mut services, &mut || dispatches += 1)
        .unwrap();
    assert_eq!(dispatches, 2);
    assert_eq!(services.printed, vec!["one \n".to_string()]);
    assert!(queue.buffer().program_revision() > revision);
    queue
        .drive_frame(&mut commands, &mut cvars, &mut services, &mut || {})
        .unwrap();
    assert_eq!(services.printed, vec!["one \n".to_string()]);
    queue
        .drive_frame(&mut commands, &mut cvars, &mut services, &mut || {})
        .unwrap();
    assert_eq!(services.printed, vec!["one \n".to_string(), "two \n".to_string()]);
    assert_eq!(cvars.variable_string("played"), "1");
    assert!(!queue.buffer().has_pending_commands());
}

#[test]
fn aliases_and_console_commands_share_one_buffered_program() {
    let (_owner, mut queue, mut commands, mut cvars, mut services) = harness(Dialect::Q2Classic);
    queue
        .submit("alias ready \"toggleconsole\"; ready; wait; echo back\n")
        .unwrap();
    queue
        .drive_frame(&mut commands, &mut cvars, &mut services, &mut || {})
        .unwrap();
    assert_eq!(services.toggles, 1);
    assert!(services.printed.is_empty());
    queue
        .drive_frame(&mut commands, &mut cvars, &mut services, &mut || {})
        .unwrap();
    assert_eq!(services.printed, vec!["back \n".to_string()]);
    assert_eq!(queue.buffer().alias_value("ready"), Some("toggleconsole\n"));
}

#[test]
fn synchronous_scripts_run_and_callbacks_fire_once() {
    let (_owner, mut queue, mut commands, mut cvars, mut services) = harness(Dialect::Q3);
    let completed = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let completed_handler = completed.clone();
    queue.bind_script_completion(move |event| {
        completed_handler
            .borrow_mut()
            .push(format!("{}:{:?}", event.name, event.result));
    });
    queue.set_script("late.cfg", Some("echo late; wait; echo resumed\n".to_string()));
    queue.submit("exec late; echo after\n").unwrap();
    queue
        .drive_frame(&mut commands, &mut cvars, &mut services, &mut || {})
        .unwrap();
    assert_eq!(
        services.printed,
        vec!["execing late.cfg\n".to_string(), "late \n".to_string()]
    );
    assert!(completed.borrow().is_empty());
    queue
        .drive_frame(&mut commands, &mut cvars, &mut services, &mut || {})
        .unwrap();
    assert_eq!(
        services.printed,
        vec![
            "execing late.cfg\n".to_string(),
            "late \n".to_string(),
            "resumed \n".to_string(),
            "after \n".to_string(),
        ]
    );
    assert_eq!(*completed.borrow(), vec!["late.cfg:Completed".to_string()]);
}

#[test]
fn failed_scripts_report_and_continue_across_frames() {
    let (_owner, mut queue, mut commands, mut cvars, mut services) = harness(Dialect::Q3);
    queue.fail_script("bad.cfg", "read denied");
    queue.submit("exec bad; echo after\n").unwrap();
    queue
        .drive_until_idle(&mut commands, &mut cvars, &mut services, 8)
        .unwrap();
    assert_eq!(
        services.printed,
        vec![
            "couldn't exec bad.cfg: read denied\n".to_string(),
            "after \n".to_string()
        ]
    );
}

#[test]
fn seats_submit_to_one_program_without_merging_sources() {
    let (owner, mut queue, mut commands, mut cvars, mut services) = harness(Dialect::Q1Quakeworld);
    let second = CommandContext::new(
        owner.session().clone(),
        CommandOrigin::LocalSeat {
            seat: owner.seat(1),
            client: owner.client(1, 0),
        },
    );
    queue.submit("echo one; wait; echo one-tail\n").unwrap();
    queue.submit_as("echo two\n", &second).unwrap();
    queue
        .drive_frame(&mut commands, &mut cvars, &mut services, &mut || {})
        .unwrap();
    assert_eq!(services.printed, vec!["one \n".to_string()]);
    queue
        .drive_frame(&mut commands, &mut cvars, &mut services, &mut || {})
        .unwrap();
    assert_eq!(
        services.printed,
        vec!["one \n".to_string(), "one-tail \n".to_string(), "two \n".to_string()]
    );
}

#[test]
fn q2_overflow_queue_rejoins_between_host_frames() {
    let (_owner, mut queue, mut commands, mut cvars, mut services) = harness(Dialect::Q2Classic);
    queue.submit("echo held\n").unwrap();
    queue.buffer_mut().copy_to_defer().unwrap();
    queue.submit("echo live\n").unwrap();
    queue
        .drive_frame(&mut commands, &mut cvars, &mut services, &mut || {})
        .unwrap();
    assert_eq!(services.printed, vec!["live \n".to_string()]);
    queue.buffer_mut().insert_from_defer().unwrap();
    queue
        .drive_frame(&mut commands, &mut cvars, &mut services, &mut || {})
        .unwrap();
    assert_eq!(services.printed, vec!["live \n".to_string(), "held \n".to_string()]);
}

fn app_config(words: &[&str]) -> StartupConfig {
    let argv: Vec<String> = words.iter().map(|word| (*word).to_string()).collect();
    let options = match parse_application_command(&argv).unwrap() {
        ApplicationCommand::Run { options } => options,
        other => panic!("expected run, got {other:?}"),
    };
    StartupConfig::from_options(&options).unwrap()
}

#[test]
fn host_frames_drain_console_waits_scripts_and_cvars() {
    let mut application =
        Application::open(&app_config(&["--movement", "q3", "--seats", "2"]), NullRenderer::new()).unwrap();
    application.provide_console_script("motd.cfg", Some("echo motd\n".to_string()));
    application
        .submit_console("exec motd; wait 2; set played 1; echo done\n")
        .unwrap();
    application.step_frame().unwrap();
    assert_eq!(
        application.console_log(),
        &["execing motd.cfg\n".to_string(), "motd \n".to_string()]
    );
    assert!(application.console_has_pending());
    application.step_frame().unwrap();
    assert_eq!(application.console_cvar("played"), "");
    application.step_frame().unwrap();
    assert_eq!(application.console_cvar("played"), "1");
    assert!(application.console_log().contains(&"done \n".to_string()));
    assert!(!application.console_has_pending());

    application.submit_console_as_seat(1, "echo seat-one\n").unwrap();
    application.step_frame().unwrap();
    assert!(application.console_log().contains(&"seat-one \n".to_string()));
    assert!(application.submit_console_as_seat(9, "echo nope\n").is_err());
}
