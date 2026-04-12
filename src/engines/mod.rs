mod gnubg_cli;
mod hureval;
mod pipcount;
mod pubeval;
mod random;
mod runtime;

pub const BUILTIN_ENGINE_NAMES: [&str; 5] =
    ["gnubg-cli", "hureval", "pipcount", "pubeval", "random"];

struct BuiltinEngine {
    name: &'static str,
    run: fn(&[String]) -> Result<(), String>,
}

const BUILTIN_ENGINES: [BuiltinEngine; 5] = [
    BuiltinEngine {
        name: "gnubg-cli",
        run: gnubg_cli::run,
    },
    BuiltinEngine {
        name: "hureval",
        run: hureval::run,
    },
    BuiltinEngine {
        name: "pipcount",
        run: pipcount::run,
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
    run_by_name_with_args(kind, &[])
}

pub fn run_by_name_with_args(kind: &str, args: &[String]) -> Result<(), String> {
    let Some(engine) = BUILTIN_ENGINES.iter().find(|engine| engine.name == kind) else {
        return Err(format!("unknown builtin engine kind '{kind}'"));
    };
    (engine.run)(args)
}
