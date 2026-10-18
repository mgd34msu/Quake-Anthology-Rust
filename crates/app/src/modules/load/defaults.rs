//! Cold stock selection uses the existing checked module readers and hosts.
use super::*;

fn setting(cvars: &qa_console::cvars::Cvars, context: Context, name: &str) -> Result<i32, String> {
    cvars
        .bind(name, context)
        .and_then(|view| cvars.integer_view(view).ok())
        .ok_or_else(|| format!("missing Q3 module setting {name}"))
}
fn choose_path(
    view: ReadView<'_>,
    candidates: impl IntoIterator<Item = impl AsRef<str>>,
) -> Result<String, String> {
    let mut tried = Vec::new();
    for candidate in candidates {
        let path = candidate.as_ref();
        if view.open(path.as_bytes()).is_some() {
            return Ok(path.to_owned());
        }
        tried.push(path.to_owned());
    }
    Err(format!("stock module not found: {}", tried.join(", ")))
}
fn q3_program(
    view: ReadView<'_>,
    cvars: &qa_console::cvars::Cvars,
    context: Context,
    name: &str,
    mode: &str,
) -> Result<String, String> {
    let mut candidates = Vec::new();
    if setting(cvars, context, mode)? == 0 && setting(cvars, context, "fs_restrict")? == 0 {
        for suffix in [
            "x86_64.so",
            "x64.dll",
            "x86_64.dll",
            "i386.so",
            "x86.so",
            "x86.dll",
        ] {
            candidates.push(format!("{name}{suffix}"));
        }
    }
    candidates.push(format!("vm/{name}.qvm"));
    choose_path(view, candidates)
}

/// An explicit SERVER module takes precedence over stock product defaults.
/// Rules are selected from startup metadata or an explicit SERVER override.
pub fn complete_stock_server(
    view: ReadView<'_>,
    cvars: &qa_console::cvars::Cvars,
    rules: Option<RuleSetId>,
    q3_stats: qa_core::primitives::Q3StatLayout,
    quakec: &mut Vec<QuakeCSpec>,
    q2: &mut Vec<Q2Spec>,
    q3: &mut Vec<Q3Spec>,
) -> Result<(), String> {
    if !quakec.is_empty() || !q2.is_empty() || q3.iter().any(|spec| spec.role == Q3Role::Game) {
        return Ok(());
    }
    let rules = rules.ok_or(
        "unknown SERVER preset; select --server-rules, an explicit module or --manual-modules",
    )?;
    let path = match rules {
        RuleSetId::Quake => choose_path(view, ["progs.dat"]),
        RuleSetId::QuakeWorld => choose_path(view, ["qwprogs.dat", "progs.dat"]),
        RuleSetId::Quake2 => choose_path(
            view,
            [
                "gamex86_64.so",
                "game.so",
                "gamex86.so",
                "gamei386.so",
                "gamex86.dll",
            ],
        ),
        RuleSetId::Quake2Rerelease => {
            choose_path(view, ["game_x64.dll", "game_x86_64.so", "game.so"])
        }
        RuleSetId::Quake3 => q3_program(
            view,
            cvars,
            Context {
                source: rules,
                side: Scope::Server,
                ..Context::default()
            },
            "qagame",
            "vm_game",
        ),
    }?;
    let search = Some(
        view.retain()
            .map_err(|error| format!("SERVER module files: {error}"))?,
    );
    match rules {
        RuleSetId::Quake | RuleSetId::QuakeWorld => quakec.push(QuakeCSpec {
            search,
            rules,
            path,
        }),
        RuleSetId::Quake2 | RuleSetId::Quake2Rerelease => q2.push(Q2Spec {
            search,
            rules,
            path,
        }),
        RuleSetId::Quake3 => q3.push(Q3Spec {
            search,
            role: Q3Role::Game,
            path,
            player_stats: q3_stats,
        }),
    }
    Ok(())
}

/// Complete the local CLIENT roles without changing explicit module choices.
/// The connection's file view and client module rules are independent of its
/// map, movement, presentation and SERVER module selection.
pub fn complete_q3_clients(
    runtime: &Runtime,
    cvars: &qa_console::cvars::Cvars,
    specs: &mut Vec<Q3Spec>,
    clients: &[Option<ClientId>; SeatId::COUNT],
) -> Result<(), String> {
    let mut defaults = Vec::new();
    for (index, id) in clients.iter().copied().enumerate() {
        let Some(id) = id else { continue };
        let client = runtime
            .server
            .clients
            .get(id.0 as usize)
            .ok_or("invalid Q3 module client")?;
        if client.connection != Some(qa_session::clients::Connection::Local)
            || client.client_rules != RuleSetId::Quake3
        {
            continue;
        }
        let seat = SeatId::new(index as u8).ok_or("invalid Q3 module seat")?;
        let context = Context {
            source: RuleSetId::Quake3,
            seat,
            ..Context::default()
        };
        let resources = runtime
            .client_resources
            .get(id.0 as usize)
            .and_then(Option::as_ref)
            .ok_or("missing Q3 client files")?;
        let view = runtime.vfs.read_view(Some(resources.state.file_search()));
        for (role, name, mode) in [
            (Q3Role::Cgame(seat), "cgame", "vm_cgame"),
            (Q3Role::Ui(seat), "ui", "vm_ui"),
        ] {
            if specs.iter().any(|spec| spec.role == role) {
                continue;
            }
            let path = q3_program(view, cvars, context, name, mode)
                .map_err(|error| format!("Q3 seat {index} {name}: {error}"))?;
            defaults.push(Q3Spec {
                search: Some(
                    view.retain()
                        .map_err(|error| format!("Q3 client files: {error}"))?,
                ),
                role,
                path,
                player_stats: qa_core::primitives::Q3StatLayout::BASE,
            });
        }
    }
    specs.extend(defaults);
    Ok(())
}
