//! Registered console connection commands use the existing activation stack.
use super::*;
use qa_core::{
    loopback::Endpoint,
    primitives::NameId,
    sys_events::{Peer, SeatId},
};
use qa_network::{
    channel::{Channel, Unreliable},
    commands::{connection::Commands, packet::Protocol},
    ingress::{Connection, ConnectionPhase, NativeConnect, Route},
};

#[derive(Clone, Copy)]
enum Action {
    Disconnect,
    Connect {
        protocol: Protocol,
        policy: crate::client_policy::ClientPolicy,
        peer: Option<std::net::SocketAddr>,
    },
}

pub(super) struct Change {
    seat: SeatId,
    previous: Option<(ClientId, EntityId)>,
    action: Action,
    started: bool,
    renderer_shutdown: Option<crate::modules::ClientRendererShutdown>,
    retiring: Option<(ClientId, EntityId)>,
}

fn report(host: &mut FrameHost, message: std::fmt::Arguments<'_>) {
    let _ = host.runtime.server.events.console_formatted(message);
}

fn connect_target(scratch: &ParsedCommand) -> Option<&str> {
    let args = scratch.arguments();
    let bytes = if args.len() == 2 {
        args.get_bytes(1)
    } else {
        args.tail_bytes(1)?.trim_ascii()
    };
    if bytes.is_empty() || bytes.iter().any(u8::is_ascii_whitespace) {
        return None;
    }
    std::str::from_utf8(bytes).ok()
}

fn client_policy(
    host: &FrameHost,
    context: ConsoleContext,
    previous: Option<(ClientId, EntityId)>,
) -> crate::client_policy::ClientPolicy {
    context.client_policy.unwrap_or_else(|| {
        previous.map_or(
            crate::client_policy::ClientPolicy {
                client: context.source,
                input: context.source,
                movement: context.source,
                trace: context.source,
            },
            |(id, _)| {
                let row = &host.runtime.server.clients[id.0 as usize];
                crate::client_policy::ClientPolicy {
                    client: row.client_rules,
                    input: row.input_rules,
                    movement: host.runtime.prediction[context.seat.index()]
                        .player
                        .movement_rules,
                    trace: host.runtime.prediction[context.seat.index()]
                        .player
                        .trace_rules,
                }
            },
        )
    })
}

pub(super) fn begin(
    host: &mut FrameHost,
    command: NameId,
    scratch: &ParsedCommand,
    context: ConsoleContext,
    driver: &Driver<'_>,
) -> Option<Change> {
    if !context.is_live(&host.runtime) {
        return None;
    }
    let args = scratch.arguments();
    let previous = host.local_clients[context.seat.index()].and_then(|id| {
        let row = host.runtime.server.clients.get(id.0 as usize)?;
        row.connection?;
        Some((id, row.entity))
    });
    if command == host.connection_commands[2] {
        if args.len() != 1 {
            report(host, format_args!("Usage: reconnect\n"));
        } else if let Some((id, _)) = previous {
            let protocol = host
                .runtime
                .network
                .get(id, Endpoint::Client)
                .filter(|connection| connection.phase.is_connected())
                .and_then(|connection| connection.commands.as_ref())
                .map(|commands| commands.protocol)
                .filter(|protocol| protocol.default_connect_port().is_some());
            if let Some(protocol) = protocol {
                if matches!(protocol, Protocol::QuakeWorld28 | Protocol::Quake2_34)
                    && host.runtime.client_resources[id.0 as usize]
                        .as_ref()
                        .is_some_and(|resources| resources.receiving_download())
                {
                    return None;
                }
                if protocol == Protocol::Quake2Repro1038
                    || protocol != Protocol::NetQuake15
                        && host.runtime.client_resources[id.0 as usize]
                            .as_ref()
                            .is_none_or(|resources| !resources.startup.reconnect_in_place())
                {
                    if driver.connector.is_none() {
                        report(
                            host,
                            format_args!(
                                "Network connection setup is unavailable in this context\n"
                            ),
                        );
                        return None;
                    }
                    let peer =
                        host.runtime
                            .network
                            .get(id, Endpoint::Client)
                            .and_then(|connection| match connection.route.peer {
                                Peer::Socket(peer) => Some(peer),
                                _ => None,
                            });
                    let Some(peer) = peer else {
                        report(host, format_args!("Reconnect requires a socket peer\n"));
                        return None;
                    };
                    return Some(Change {
                        seat: context.seat,
                        previous,
                        action: Action::Connect {
                            protocol,
                            policy: client_policy(host, context, previous),
                            peer: Some(peer),
                        },
                        started: false,
                        renderer_shutdown: None,
                        retiring: None,
                    });
                }
                if protocol != Protocol::NetQuake15 {
                    host.begin_client_map_change(id);
                }
                if let Some(resources) = host.runtime.client_resources[id.0 as usize].as_mut() {
                    resources.startup.reconnect(protocol);
                }
            } else {
                report(
                    host,
                    format_args!("Reconnect requires a connected native peer\n"),
                );
            }
        } else {
            report(host, format_args!("No connection to reconnect\n"));
        }
        return None;
    }
    let action = if command == host.connection_commands[1] {
        if args.len() != 1 {
            report(host, format_args!("Usage: disconnect\n"));
            return None;
        }
        Action::Disconnect
    } else if command == host.connection_commands[0] {
        if connect_target(scratch).is_none() {
            report(host, format_args!("Usage: connect <server[:port]>\n"));
            return None;
        }
        let protocol = Protocol::native(context.source);
        if protocol.default_connect_port().is_none() {
            report(
                host,
                format_args!(
                    "Native connection setup for {} is unavailable\n",
                    context.source.name()
                ),
            );
            return None;
        }
        if driver.connector.is_none() {
            report(
                host,
                format_args!("Network connection setup is unavailable in this context\n"),
            );
            return None;
        }
        let policy = client_policy(host, context, previous);
        Action::Connect {
            protocol,
            policy,
            peer: None,
        }
    } else {
        report(
            host,
            format_args!("Console command is unavailable in this context\n"),
        );
        return None;
    };
    Some(Change {
        seat: context.seat,
        previous,
        action,
        started: false,
        renderer_shutdown: None,
        retiring: None,
    })
}

impl Change {
    pub(super) fn advance(
        &mut self,
        host: &mut FrameHost,
        child: &mut Option<ExecutionResult>,
        scratch: &ParsedCommand,
        owner: Option<GuestStart>,
        context: ConsoleContext,
        access: TraceAccess<'_>,
        driver: &mut Driver<'_>,
    ) -> Option<FrameStep> {
        // Retirement completion is mandatory even when the callback traps or
        // invalidates the console producer which initiated the change.
        if let Some((id, lifetime)) = self.retiring.take() {
            child.take();
            host.finish_client_disconnect(id, lifetime, access);
        }
        if !self.started {
            if !context.is_live(&host.runtime) {
                return None;
            }
            if let Some((id, lifetime)) = self.previous {
                if !host
                    .runtime
                    .server
                    .clients
                    .get(id.0 as usize)
                    .is_some_and(|row| row.connection.is_some() && row.entity == lifetime)
                {
                    report(host, format_args!("Connection changed before disconnect\n"));
                    return None;
                }
                match host.advance_client_renderer_shutdown(
                    (id, lifetime),
                    &mut self.renderer_shutdown,
                    child,
                    driver,
                ) {
                    Ok(Some(outcome)) => return Some(call_step(outcome, child)),
                    Ok(None) => {}
                    Err(error) => {
                        report(host, format_args!("CLIENT renderer shutdown: {error:?}\n"));
                        return None;
                    }
                }
                self.started = true;
                disconnect_wire(host, id, driver);
                if let Some(OperationStep::Call(outcome)) =
                    super::disconnect(host, id, lifetime, &mut self.retiring, access)
                {
                    return Some(call_step(outcome, child));
                }
            }
        }
        self.started = true;
        if owner.is_some_and(|owner| !super::valid(host, owner, true)) {
            return Some(FrameStep::Finished(Err(())));
        }
        // The admitted change owns the captured target after disconnecting it.
        let context = ConsoleContext {
            client_lifetime: None,
            ..context
        };
        if !context.is_live(&host.runtime) {
            return None;
        }
        let Action::Connect {
            protocol,
            policy,
            peer,
        } = self.action
        else {
            return None;
        };
        if host.local_clients[self.seat.index()].is_some() {
            report(host, format_args!("Connection changed during disconnect\n"));
            return None;
        }
        if host.runtime.server.available_client_slot().is_none() {
            let awaiting_retirement = self.previous.is_some_and(|(id, lifetime)| {
                host.runtime
                    .server
                    .clients
                    .get(id.0 as usize)
                    .is_some_and(|row| {
                        row.connection.is_none()
                            && row
                                .audio_retirement
                                .is_some_and(|retirement| retirement.lifetime == lifetime)
                            && host.runtime.server.entities.resolve(row.entity).is_some()
                    })
            });
            if awaiting_retirement && context.client_policy.is_none() {
                let retry = ConsoleContext {
                    client_policy: Some(policy),
                    source: protocol.rule_set(),
                    ..context
                };
                let (_, buffer, _) = host.console.module_parts();
                let mut reconnect = qa_core::text::FixedText::<96>::default();
                let text = if let Some(peer) = peer {
                    use std::fmt::Write;
                    if write!(reconnect, "connect {peer}").is_err() {
                        report(host, format_args!("Reconnect address capacity\n"));
                        return None;
                    }
                    reconnect.as_bytes()
                } else {
                    scratch.raw_bytes()
                };
                match buffer.insert_line_bytes(text, retry) {
                    Ok(()) => buffer.wait_next_frame(retry),
                    Err(error) => report(host, format_args!("Connect retry rejected: {error:?}\n")),
                }
            } else {
                report(
                    host,
                    format_args!("Connect failed: no available client slot\n"),
                );
            }
            return None;
        }
        let result = (|| {
            let mut resolved = qa_core::text::FixedText::<64>::default();
            let target = if let Some(peer) = peer {
                use std::fmt::Write;
                write!(resolved, "{peer}").map_err(|_| "reconnect address capacity")?;
                resolved.as_str()
            } else {
                connect_target(scratch).ok_or("invalid server address")?
            };
            let connector = driver
                .connector
                .as_deref_mut()
                .ok_or_else(|| "network connection setup is unavailable".to_owned())?;
            let (lease, peer) = connector
                .open_udp_client(
                    target,
                    protocol
                        .default_connect_port()
                        .ok_or("native connection setup unavailable")?,
                )
                .map_err(|error| format!("server {target}: {error}"))?;
            let qport = if protocol.userinfo_bytes().is_some() {
                let configured =
                    host.console
                        .cvars
                        .numeric(host.qport)
                        .map_err(|_| "invalid native qport")? as i32 as u16;
                let captured = if configured == 0 {
                    lease.local_port()
                } else {
                    configured
                };
                if protocol == Protocol::Quake2Repro1038 {
                    u16::from(captured as u8)
                } else {
                    captured
                }
            } else {
                0
            };
            let maximum_message = protocol.message_bytes();
            let mut channel = Channel::load(
                protocol.client_channel(qport),
                Endpoint::Client,
                maximum_message,
                16,
            )
            .map_err(|error| format!("native channel: {error:?}"))?;
            channel.set_qport(qport);
            let connection = Connection {
                phase: ConnectionPhase::Connecting(NativeConnect::new()),
                route: Route {
                    socket: lease.socket(),
                    peer: Peer::Socket(peer),
                },
                channel,
                output: None,
                pending_output: None,
                commands: Some(Commands::load(protocol, maximum_message)),
            };
            let id = host.runtime.connect_client(
                self.seat,
                policy,
                crate::snapshots::NativePlayerBinding::Awaiting,
                connection,
                Some(lease),
            )?;
            let owner = (id, host.runtime.server.clients[id.0 as usize].entity);
            if let Err(error) = host.console.cvars.load_client_values(owner) {
                host.runtime.disconnect_client(id);
                return Err(format!("CLIENT cvars: {error:?}"));
            }
            host.local_clients[self.seat.index()] = Some(id);
            host.local_worlds[self.seat.index()] = None;
            Ok::<(), String>(())
        })();
        if let Err(error) = result {
            report(host, format_args!("Connect failed: {error}\n"));
        }
        None
    }
}

fn call_step(outcome: ModuleDispatch, child: &mut Option<ExecutionResult>) -> FrameStep {
    match outcome {
        ModuleDispatch::Completed(result) => {
            *child = Some(result);
            FrameStep::Continue
        }
        ModuleDispatch::Guest(start) => FrameStep::Guest(Activation::new(start, false)),
    }
}

fn disconnect_wire(host: &mut FrameHost, id: ClientId, driver: &mut Driver<'_>) {
    let runtime = &mut host.runtime;
    let Some(connection) = runtime
        .network
        .get_mut(id, Endpoint::Client)
        .filter(|connection| connection.phase.is_connected())
    else {
        return;
    };
    let Some(commands) = connection.commands.as_ref() else {
        return;
    };
    let protocol = commands.protocol;
    let route = connection.route;
    let mut admit = |bytes: &[u8]| {
        crate::transport::admit_output(
            &mut runtime.loopback,
            &mut |socket, to, bytes| {
                driver
                    .connector
                    .as_ref()
                    .is_some_and(|pump| pump.send_udp(socket, to, bytes).is_ok())
            },
            Endpoint::Client,
            route,
            bytes,
        )
    };
    if connection.channel.pending_packet().is_some()
        || connection.channel.pending_fragments()
        || connection.channel.pending_controls() != 0
    {
        let submitted = crate::transport::send(
            &mut connection.channel,
            host.time,
            &mut (),
            |channel, ()| {
                channel
                    .prepare(None, host.time)
                    .is_ok_and(|packet| packet.is_some())
            },
            &mut admit,
            |channel, (), _| !channel.pending_fragments() && channel.pending_controls() == 0,
        );
        if !submitted.complete {
            runtime.server.events.reject_publication();
            report(host, format_args!("Disconnect packet was not admitted\n"));
            return;
        }
    }
    let bytes = &mut runtime.output_native_bytes;
    let length = match protocol {
        Protocol::NetQuake15
        | Protocol::QuakeWorld28
        | Protocol::Quake2_34
        | Protocol::Quake2Repro1038 => protocol.write_disconnect(bytes),
        _ => connection
            .channel
            .queue_client_command(protocol, b"disconnect")
            .map_err(|_| qa_network::commands::packet::Error::Context)
            .and_then(|_| commands.encode_control(&connection.channel, bytes)),
    };
    let Ok(length) = length else {
        runtime.server.events.reject_publication();
        report(host, format_args!("Disconnect command was not admitted\n"));
        return;
    };
    let (submissions, repeats) = match protocol {
        Protocol::QuakeWorld28 | Protocol::Quake2_34 | Protocol::Quake3_68 => (3, 1),
        Protocol::Quake2Repro1038 => (1, 3),
        _ => (1, 1),
    };
    let mut disconnect_admit = |bytes: &[u8]| {
        let mut accepted = false;
        for _ in 0..repeats {
            accepted |= admit(bytes);
        }
        accepted
    };
    for _ in 0..submissions {
        let submitted = crate::transport::send(
            &mut connection.channel,
            host.time,
            &mut false,
            |channel, _| {
                channel
                    .prepare_move(&bytes[..length], host.time, None, None)
                    .is_ok_and(|packet| packet.is_some())
            },
            &mut disconnect_admit,
            |channel, included, disposition| {
                *included |= disposition == Unreliable::Included;
                *included && !channel.pending_fragments()
            },
        );
        if !submitted.complete {
            runtime.server.events.reject_publication();
            report(host, format_args!("Disconnect packet was not admitted\n"));
            return;
        }
    }
}
