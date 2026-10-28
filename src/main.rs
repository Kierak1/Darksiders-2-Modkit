mod cli;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn media_dir() -> PathBuf {
    std::env::var_os("DS2_MEDIA")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("Game")
                .join("Darksiders II Deathinitive Edition")
                .join("media")
        })
}

fn run() -> ds2_modkit::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let media = media_dir();
    let cmd = args.first().map(|s| s.as_str()).unwrap_or("list");

    if cmd == "hash" {
        return cli::obsp::hash(&args);
    }
    if cmd == "save-db" {
        return cli::db::run(&args, &media);
    }
    if cmd.starts_with("save-") {
        return cli::save::run(cmd, &args, &media);
    }
    if cmd == "apply" {
        return cli::mods::apply(&media, &args);
    }
    if let Some(result) = cli::assets::run(cmd, &args, &media) {
        return result;
    }
    if let Some(result) = cli::chars::run(cmd, &args) {
        return result;
    }
    if let Some(result) = cli::map::run(cmd, &args, &media) {
        return result;
    }
    if cmd.starts_with("obsp-") {
        return cli::obsp::run(cmd, &args, &media);
    }
    Err(format!("unknown command: {cmd}").into())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
