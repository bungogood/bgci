mod gnubg_cli;
mod pubeval;
mod random;
mod runtime;

pub const BUILTIN_ENGINE_NAMES: [&str; 3] = ["gnubg-cli", "pubeval", "random"];

struct BuiltinEngine {
    name: &'static str,
    run: fn(),
}

const BUILTIN_ENGINES: [BuiltinEngine; 3] = [
    BuiltinEngine {
        name: "gnubg-cli",
        run: gnubg_cli::run,
    },
    BuiltinEngine {
        name: "pubeval",
        run: pubeval::run,
    },
    BuiltinEngine {
        name: "random",
        run: random::run,
    },
];

pub fn builtin_engine_name(alias: &str) -> Option<&'static str> {
    BUILTIN_ENGINES
        .iter()
        .find(|engine| engine.name == alias)
        .map(|engine| engine.name)
}

pub fn run_by_name(kind: &str) -> Result<(), String> {
    let Some(engine) = BUILTIN_ENGINES.iter().find(|engine| engine.name == kind) else {
        return Err(format!("unknown builtin engine kind '{kind}'"));
    };
    (engine.run)();
    Ok(())
}
