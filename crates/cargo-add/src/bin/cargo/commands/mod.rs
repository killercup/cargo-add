use cargo::CliResult;

pub mod add;

pub fn builtin() -> [clap::Command<'static>; 1] {
    [add::cli()]
}

pub fn builtin_exec(cmd: &str) -> Option<fn(&mut cargo::Config, &clap::ArgMatches) -> CliResult> {
    let f = match cmd {
        "add" => add::exec,
        _ => return None,
    };
    Some(f)
}