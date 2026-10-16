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
    ingress::{Connection, ConnectionPhase, NqConnect, Route},
};

#[derive(Clone, Copy)]
enum Action {
    Disconnect,
    Connect {
        protocol: Protocol,
        policy: crate::client_policy::ClientPolicy,
    },
}

pub(super) struct Change {
    seat: SeatId,
    previous: Option<(ClientId, EntityId)>,
    action: Action,
    started: bool,
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

pub(super) fn begin(
    host: &mut FrameHost,
    command: NameId,
    scratch: &ParsedCommand,
    context: ConsoleContext,
    driver: &Driver<'_>,
) -> Option<Change> {
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
            let native = host
                .runtime
                .network
                .get(id, Endpoint::Client)
                .is_some_and(|connection| {
                    connection.phase.is_connected()
                        && connection
                            .commands
                            .as_ref()
                            .is_some_and(|commands| commands.protocol == Protocol::NetQuake15)
                });
            if native {
                if let Some(resources) = host.runtime.client_resources[id.0 as usize].as_mut() {
                    resources.begin_nq_signon();
                }
            } else {
                report(
                    host,
                    format_args!("Reconnect requires a connected NetQuake peer\n"),
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
        let policy = context.client_policy.unwrap_or_else(|| {
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
        });
        Action::Connect { protocol, policy }
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
            self.started = true;
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
                disconnect_wire(host, id, driver);
                if let Some(OperationStep::Call(outcome)) =
                    super::disconnect(host, id, lifetime, &mut self.retiring, access)
                {
                    return Some(match outcome {
                        ModuleDispatch::Completed(result) => {
                            *child = Some(result);
                            FrameStep::Continue
                        }
                        ModuleDispatch::Guest(start) => {
                            FrameStep::Guest(Activation::new(start, false))
                        }
                    });
                }
            }
        }
        if owner.is_some_and(|owner| !super::valid(host, owner, true)) {
            return Some(FrameStep::Finished(Err(())));
        }
        if context.producer.is_some_and(|producer| {
            !qa_console::commands::Host::command_producer_live(&host.runtime, producer)
        }) {
            return None;
        }
        let Action::Connect { protocol, policy } = self.action else {
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
                    ..context
                };
                let (_, buffer, _) = host.console.module_parts();
                match buffer.insert_line_bytes(scratch.raw_bytes(), retry) {
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
            let target = connect_target(scratch).ok_or("invalid server address")?;
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
            let channel = Channel::load(protocol.channel(), Endpoint::Client, 8192, 16)
                .map_err(|error| format!("native channel: {error:?}"))?;
            let connection = Connection {
                phase: ConnectionPhase::Connecting(NqConnect::new()),
                route: Route {
                    socket: lease.socket(),
                    peer: Peer::Socket(peer),
                },
                channel,
                output: None,
                pending_output: None,
                commands: Some(Commands::load(protocol, 8192)),
            };
            let id = host.runtime.connect_client(
                self.seat,
                policy,
                crate::snapshots::NativePlayerBinding::Awaiting,
                connection,
                Some(lease),
            )?;
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
        Protocol::NetQuake15 | Protocol::QuakeWorld28 => protocol.write_disconnect(bytes),
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
    let submitted = crate::transport::send(
        &mut connection.channel,
        host.time,
        &mut false,
        |channel, _| {
            channel
                .prepare_move(&bytes[..length], host.time, None, None)
                .is_ok_and(|packet| packet.is_some())
        },
        &mut admit,
        |channel, included, disposition| {
            *included |= disposition == Unreliable::Included;
            *included && !channel.pending_fragments()
        },
    );
    if !submitted.complete {
        runtime.server.events.reject_publication();
        report(host, format_args!("Disconnect packet was not admitted\n"));
    }
}
