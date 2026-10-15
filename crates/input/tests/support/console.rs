#![allow(dead_code)]
use qa_console::{
    command_text::Arguments,
    commands::{CommandError, Console, Host, ScriptError},
    cvars::Cvars,
    input_settings::InputHandles,
    views::Context,
};
use qa_core::{
    primitives::RuleSetId,
    sys_events::{EventTime, KeySource, SeatId},
};
use qa_input::{Input, InputPolicy};

#[derive(Default)]
pub struct Capture {
    input: Input,
    pub presses: u32,
    pub releases: u32,
    pub last_context: Option<Context>,
    pub raw: qa_core::text::FixedText<64>,
    pub raw_calls: u32,
    pub producers: [Option<qa_core::primitives::CommandProducer>; 2],
}
impl Host for Capture {
    fn input(&mut self) -> &mut Input {
        &mut self.input
    }
    fn input_time(&self) -> EventTime {
        EventTime::default()
    }
    fn print(&mut self, _: std::fmt::Arguments<'_>) {}
    fn read_script(&mut self, _: &str, _: &mut [u8]) -> Result<usize, ScriptError> {
        Err(ScriptError::Missing)
    }
    fn quit(&mut self) {}
    fn command_producer_live(&self, producer: qa_core::primitives::CommandProducer) -> bool {
        self.producers.contains(&Some(producer))
    }
}
fn edge(
    _: &mut Console<Capture>,
    host: &mut Capture,
    args: &Arguments<'_>,
    context: Context,
) -> Result<(), CommandError> {
    host.last_context = Some(context);
    if args.get(0).starts_with('+') {
        host.presses += 1;
    } else {
        host.releases += 1;
    }
    Ok(())
}
pub struct Router {
    pub console: Console<Capture>,
    pub host: Capture,
    handles: InputHandles,
}
impl Default for Router {
    fn default() -> Self {
        let mut console = Console::new(Context::default()).unwrap();
        assert!(console.register("+edge", edge));
        assert!(console.register("-edge", edge));
        let handles = InputHandles::load(&console.cvars).unwrap();
        Self {
            console,
            host: Capture::default(),
            handles,
        }
    }
}
impl Router {
    pub fn policy(&self, rules: RuleSetId) -> InputPolicy {
        self.handles.policy(&self.console.cvars, rules)
    }
    pub fn command(
        &mut self,
        seat: SeatId,
        time: EventTime,
        key_source: Option<KeySource>,
        producer: Option<qa_core::primitives::CommandProducer>,
        text: &str,
    ) -> bool {
        self.command_bytes(seat, time, key_source, producer, text.as_bytes())
    }
    pub fn command_bytes(
        &mut self,
        seat: SeatId,
        time: EventTime,
        key_source: Option<KeySource>,
        producer: Option<qa_core::primitives::CommandProducer>,
        text: &[u8],
    ) -> bool {
        self.console
            .module_parts()
            .1
            .append_line_bytes(
                text,
                Context {
                    seat,
                    event_time: Some(time),
                    key_source,
                    producer,
                    ..Context::default()
                },
            )
            .is_ok()
    }
    pub fn execute(&mut self, input: &mut Input) {
        std::mem::swap(input, &mut self.host.input);
        self.console.execute_frame(&mut self.host);
        std::mem::swap(input, &mut self.host.input);
    }
}
pub fn keyboard(control: u16) -> KeySource {
    KeySource {
        device: qa_core::sys_events::DeviceId::Keyboard(0),
        control,
    }
}
pub fn catalog_policies() -> Result<[InputPolicy; 5], String> {
    let vars = Cvars::new().map_err(|error| format!("{error:?}"))?;
    let handles = InputHandles::load(&vars)?;
    Ok(RuleSetId::ALL.map(|rules| handles.policy(&vars, rules)))
}
pub fn catalog_policy(rules: RuleSetId) -> InputPolicy {
    catalog_policies().unwrap()[rules as usize]
}

pub struct RoutedInput {
    input: Input,
    pub router: Router,
}
impl RoutedInput {
    pub fn elapsed_policy(&self, speed: f32) -> InputPolicy {
        let mut policy = self.router.policy(RuleSetId::Quake2);
        policy.speed = [speed; 3];
        policy.back_speed = speed;
        policy.sensitivity = 1.0;
        policy.mouse_side = 0.0;
        policy.mouse_forward = 0.0;
        policy.angle_multiplier = 1.0;
        policy.move_multiplier = 1.0;
        policy
    }
    pub fn load() -> Self {
        Self {
            input: Input::load(),
            router: Router::default(),
        }
    }
    pub fn dispatch(
        &mut self,
        event: qa_core::sys_events::SysEvent<'_>,
        target: &mut impl qa_input::Target,
    ) {
        self.input.dispatch(
            event,
            &mut Routing {
                router: &mut self.router,
                observer: target,
            },
        );
        self.router.execute(&mut self.input);
    }
    pub fn bind(
        &mut self,
        control: u16,
        binding: Option<qa_input::Binding>,
        time: EventTime,
        target: &mut impl qa_input::Target,
    ) -> bool {
        let result = self.input.bind(
            control,
            binding,
            time,
            &mut Routing {
                router: &mut self.router,
                observer: target,
            },
        );
        self.router.execute(&mut self.input);
        result
    }
    pub fn bind_text(
        &mut self,
        control: u16,
        text: &str,
        time: EventTime,
        target: &mut impl qa_input::Target,
    ) -> Result<(), qa_input::BindError> {
        let result = self.input.bind_text(
            control,
            text,
            time,
            &mut Routing {
                router: &mut self.router,
                observer: target,
            },
        );
        self.router.execute(&mut self.input);
        result
    }
}
impl std::ops::Deref for RoutedInput {
    type Target = Input;
    fn deref(&self) -> &Input {
        &self.input
    }
}
impl std::ops::DerefMut for RoutedInput {
    fn deref_mut(&mut self) -> &mut Input {
        &mut self.input
    }
}
struct Routing<'a, T> {
    router: &'a mut Router,
    observer: &'a mut T,
}
impl<T: qa_input::Target> qa_input::Target for Routing<'_, T> {
    fn key(&mut self, seat: SeatId, control: u16, down: bool, repeat: bool) -> bool {
        self.observer.key(seat, control, down, repeat)
    }
    fn character(&mut self, seat: SeatId, value: char) {
        self.observer.character(seat, value);
    }
    fn command(
        &mut self,
        seat: SeatId,
        time: EventTime,
        source: Option<KeySource>,
        producer: Option<qa_core::primitives::CommandProducer>,
        text: &str,
    ) -> bool {
        self.command_bytes(seat, time, source, producer, text.as_bytes())
    }
    fn command_bytes(
        &mut self,
        seat: SeatId,
        time: EventTime,
        source: Option<KeySource>,
        producer: Option<qa_core::primitives::CommandProducer>,
        text: &[u8],
    ) -> bool {
        self.router
            .command_bytes(seat, time, source, producer, text)
            && self
                .observer
                .command_bytes(seat, time, source, producer, text)
    }
    fn cancel_commands(
        &mut self,
        source: KeySource,
        producer: Option<qa_core::primitives::CommandProducer>,
    ) {
        self.router.console.discard_acquisition(source, producer);
        self.observer.cancel_commands(source, producer);
    }
}
