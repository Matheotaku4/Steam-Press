use crate::Cli;
use steam_press_core::config::{Paths, Settings};
use steam_press_core::credentials::{CredentialStore, SavedCredentials};
use steam_press_core::depots::DepotNames;
use steam_press_core::downloader::Credentials;
use steam_press_core::events::{EventSink, PackEvent};

pub fn paths(cli: &Cli) -> Paths {
    let mut paths = Paths::discover();
    if let Some(config_dir) = &cli.config_dir {
        paths.config_dir = config_dir.clone();
    }
    if let Some(work_dir) = &cli.work_dir {
        paths.work_dir = work_dir.clone();
    }
    if let Some(output) = &cli.output {
        paths.output_dir = output.clone();
    }
    paths
}

pub fn load_settings(cli: &Cli) -> Result<(Paths, Settings), String> {
    let paths = paths(cli);
    paths.ensure().map_err(|error| format!("répertoires : {error}"))?;
    let mut settings = Settings::load(&paths);
    if let Some(output) = &cli.output {
        settings.output_dir = Some(output.clone());
    }
    Ok((paths, settings))
}

pub fn save_credentials(
    paths: &Paths,
    settings: &Settings,
    username: &str,
    password: Option<&str>,
) -> Result<(), String> {
    let store = CredentialStore::new(paths);
    store.save(
        &SavedCredentials {
            username: username.to_string(),
            password: password.map(str::to_string),
        },
        &settings.key,
    )
}

pub fn credentials(
    paths: &Paths,
    settings: &Settings,
    anonymous: bool,
) -> Result<Credentials, String> {
    if anonymous {
        return Ok(Credentials::anonymous());
    }
    match CredentialStore::new(paths).load(&settings.key)? {
        Some(saved) => Ok(Credentials {
            username: saved.username,
            password: saved.password,
        }),
        None => Err("aucun identifiant enregistré : lancez « steam-press login » ou utilisez --anonymous".to_string()),
    }
}

pub fn depot_names(paths: &Paths) -> DepotNames {
    DepotNames::load(paths.depots_file()).unwrap_or_default()
}

pub fn event_sink(verbose: bool) -> EventSink {
    EventSink::new(move |event| match event {
        PackEvent::Stage { stage, message, .. } => {
            eprintln!("[{}] {message}", stage.label());
        }
        PackEvent::Progress {
            percent, message, ..
        } => match percent {
            Some(value) => eprintln!("  {:>5.1}% {message}", value * 100.0),
            None => eprintln!("  {message}"),
        },
        PackEvent::Log(line) => {
            if verbose {
                eprintln!("  {line}");
            }
        }
        PackEvent::Finished { outcome, .. } => eprintln!("{outcome}"),
    })
}

pub fn prompt(label: &str) -> Result<String, String> {
    use std::io::Write;
    print!("{label}");
    std::io::stdout()
        .flush()
        .map_err(|error| error.to_string())?;
    let mut value = String::new();
    std::io::stdin()
        .read_line(&mut value)
        .map_err(|error| error.to_string())?;
    Ok(value.trim().to_string())
}
