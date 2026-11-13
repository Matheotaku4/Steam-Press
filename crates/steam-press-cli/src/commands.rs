use crate::support;
use crate::{
    Cli, Command, DepotsAction, InfoArgs, LoginArgs, PackArgs, QueueAction, TemplateAction,
    WorkshopAction,
};
use std::path::PathBuf;
use std::process::ExitCode;

use steam_press_core::archive;
use steam_press_core::compressor::Compressor;
use steam_press_core::credentials::CredentialStore;
use steam_press_core::depots::DepotNames;
use steam_press_core::downloader::{self, Downloader};
use steam_press_core::pack::{PackOptions, PackStatus, Packer};
use steam_press_core::queue::{
    load_game_queue, load_workshop_queue, save_game_queue, save_workshop_queue, GameJob,
    WorkshopJob,
};
use steam_press_core::template::{self, TemplateData};
use steam_press_core::workshop::WorkshopPacker;
use steam_press_core::{steam, Platform};

pub fn dispatch(cli: Cli) -> Result<ExitCode, String> {
    match &cli.command {
        Command::Doctor => doctor(&cli),
        Command::Login(args) => login(&cli, args),
        Command::Info(args) => info(&cli, args),
        Command::Search { term } => search(term),
        Command::Queue { action } => queue(&cli, action),
        Command::Pack(args) => pack(&cli, args),
        Command::Workshop { action } => workshop(&cli, action),
        Command::Depots { action } => depots(&cli, action),
        Command::Template { action } => template_command(&cli, action),
    }
}

fn doctor(cli: &Cli) -> Result<ExitCode, String> {
    let (paths, settings) = support::load_settings(cli)?;
    println!("Config  : {}", paths.config_dir.display());
    println!("Work    : {}", paths.work_dir.display());
    println!("Output  : {}", settings.output_dir(&paths).display());
    println!("Archiver: {}", settings.compressor.label());

    let steamcmd = downloader::steamcmd::SteamCmd::from_settings(&settings).check();
    let depotdownloader =
        downloader::depotdownloader::DepotDownloader::from_settings(&settings).check();
    for (label, status) in [("SteamCMD", steamcmd), ("DepotDownloader", depotdownloader)] {
        match status.path {
            Some(path) => println!("{label:<14}: {}", path.display()),
            None => println!("{label:<14}: not found"),
        }
    }

    for compressor in [Compressor::SevenZip, Compressor::Rar] {
        match archive::compressor_binary(compressor, &settings) {
            Ok(path) => println!("{:<14}: {}", compressor.label(), path.display()),
            Err(_) => println!("{:<14}: not found", compressor.label()),
        }
    }

    let names = support::depot_names(&paths);
    println!("Known depots: {}", names.len());

    let output = settings.output_dir(&paths);
    std::fs::create_dir_all(&output).map_err(|error| format!("output: {error}"))?;
    let probe = output.join(".steam-press-write-test");
    std::fs::write(&probe, b"ok").map_err(|error| format!("output is not writable: {error}"))?;
    let _ = std::fs::remove_file(&probe);
    println!("Output  : writable");

    Ok(ExitCode::SUCCESS)
}

fn login(cli: &Cli, args: &LoginArgs) -> Result<ExitCode, String> {
    let (paths, mut settings) = support::load_settings(cli)?;
    settings.ensure_key();
    settings.save(&paths).map_err(|error| error.to_string())?;

    let username = match &args.username {
        Some(username) => username.clone(),
        None => support::prompt("Steam username (or \"anonymous\"): ")?,
    };
    if username.is_empty() {
        return Err("empty username".to_string());
    }

    let store = CredentialStore::new(&paths);
    if username.eq_ignore_ascii_case("anonymous") {
        store.clear().map_err(|error| error.to_string())?;
        println!("Anonymous mode: no credentials stored.");
        return Ok(ExitCode::SUCCESS);
    }

    let password = if args.password_stdin {
        support::prompt("")?
    } else {
        rpassword::prompt_password("Steam password: ")
            .map_err(|error| format!("reading the password: {error}"))?
    };
    if password.is_empty() {
        return Err("empty password".to_string());
    }

    support::save_credentials(&paths, &settings, &username, Some(&password))?;
    println!("Credentials saved for {username}.");
    Ok(ExitCode::SUCCESS)
}

fn info(cli: &Cli, args: &InfoArgs) -> Result<ExitCode, String> {
    let (_paths, _settings) = support::load_settings(cli)?;
    let app = steam::fetch_app_info(&args.appid, &args.branch)?;
    println!("{} ({})", app.name, app.appid);
    println!(
        "Build   : {} - {}",
        app.build_id,
        app.build_time
            .map(template::format_utc_datetime)
            .unwrap_or_else(|| "unknown date".to_string())
    );
    if let Some(install_dir) = &app.install_dir {
        println!("Directory: {install_dir}");
    }
    for depot in &app.depots {
        println!(
            "  {} - {} [Manifest {}]",
            depot.depot_id,
            if depot.name.is_empty() {
                "DepotName"
            } else {
                &depot.name
            },
            depot.manifest_id
        );
    }
    if let Ok(store) = steam::fetch_store_info(&args.appid) {
        if !store.short_description.is_empty() {
            println!("\n{}", store.short_description);
        }
        if let Some(website) = store.website {
            println!("{website}");
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn search(term: &str) -> Result<ExitCode, String> {
    let term = term.trim();
    if term.chars().count() < 2 {
        return Err("search term must be at least two characters".to_string());
    }
    let results = steam::search_store(term)?;
    if results.is_empty() {
        println!("No games found for \"{term}\".");
        return Ok(ExitCode::SUCCESS);
    }
    println!("{} result(s) for \"{term}\":", results.len());
    for item in results {
        println!("{:>10}  {}", item.appid, item.name);
    }
    Ok(ExitCode::SUCCESS)
}

fn game_queue_file(cli: &Cli, file: &Option<PathBuf>) -> PathBuf {
    file.clone()
        .unwrap_or_else(|| support::paths(cli).config_dir.join("queue.sspq"))
}

fn workshop_queue_file(cli: &Cli, file: &Option<PathBuf>) -> PathBuf {
    file.clone()
        .unwrap_or_else(|| support::paths(cli).config_dir.join("queue.sswq"))
}

fn queue(cli: &Cli, action: &QueueAction) -> Result<ExitCode, String> {
    match action {
        QueueAction::List { file } => {
            let path = game_queue_file(cli, file);
            let load = load_game_queue(&path)?;
            if load.jobs.is_empty() {
                println!("Queue is empty ({})", path.display());
            }
            for (index, job) in load.jobs.iter().enumerate() {
                println!(
                    "{:>3}. {:<14} {:>8}  branche {}{}",
                    index + 1,
                    job.platform.label(),
                    job.appid,
                    job.branch,
                    if job.branch_password.is_empty() {
                        String::new()
                    } else {
                        " (protected)".to_string()
                    }
                );
            }
            for warning in load.warnings {
                eprintln!("warning: {warning}");
            }
        }
        QueueAction::Add {
            appid,
            os,
            branch,
            branch_password,
            file,
        } => {
            let path = game_queue_file(cli, file);
            let mut load = load_game_queue(&path)?;
            let job = GameJob::new(Platform::from(*os), appid.clone())
                .with_branch(branch.clone(), branch_password.clone().unwrap_or_default());
            job.validate()?;
            if load.jobs.contains(&job) {
                return Err("this job is already queued".to_string());
            }
            load.jobs.push(job);
            save_game_queue(&path, &load.jobs)?;
            println!("Job added ({}).", path.display());
        }
        QueueAction::Remove { index, file } => {
            let path = game_queue_file(cli, file);
            let mut load = load_game_queue(&path)?;
            if *index == 0 || *index > load.jobs.len() {
                return Err(format!("invalid index: {index}"));
            }
            load.jobs.remove(index - 1);
            save_game_queue(&path, &load.jobs)?;
            println!("Job removed.");
        }
        QueueAction::Move { from, to, file } => {
            let path = game_queue_file(cli, file);
            let mut load = load_game_queue(&path)?;
            if *from == 0 || *from > load.jobs.len() || *to == 0 || *to > load.jobs.len() {
                return Err("invalid index".to_string());
            }
            let job = load.jobs.remove(from - 1);
            load.jobs.insert(to - 1, job);
            save_game_queue(&path, &load.jobs)?;
            println!("Queue reordered.");
        }
        QueueAction::Clear { file } => {
            let path = game_queue_file(cli, file);
            save_game_queue(&path, &[])?;
            println!("Queue cleared.");
        }
        QueueAction::Import { source, file } => {
            let path = game_queue_file(cli, file);
            let incoming = load_game_queue(source)?;
            let mut load = load_game_queue(&path)?;
            let mut added = 0;
            for job in incoming.jobs {
                if !load.jobs.contains(&job) {
                    load.jobs.push(job);
                    added += 1;
                }
            }
            save_game_queue(&path, &load.jobs)?;
            println!("{added} job(s) imported.");
            for warning in incoming.warnings {
                eprintln!("warning: {warning}");
            }
        }
        QueueAction::Export { destination, file } => {
            let path = game_queue_file(cli, file);
            let load = load_game_queue(&path)?;
            save_game_queue(destination, &load.jobs)?;
            println!("Queue exported to {}.", destination.display());
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn resolve_compressor(
    value: &Option<String>,
    no_compress: bool,
) -> Result<(Compressor, bool), String> {
    if no_compress {
        return Ok((Compressor::SevenZip, true));
    }
    match value.as_deref() {
        None => Ok((Compressor::SevenZip, false)),
        Some("none") => Ok((Compressor::SevenZip, true)),
        Some(value) => Compressor::from_setting(value)
            .map(|compressor| (compressor, false))
            .ok_or_else(|| format!("unknown compressor: {value} (7z, rar or none)")),
    }
}

fn pack(cli: &Cli, args: &PackArgs) -> Result<ExitCode, String> {
    let (paths, mut settings) = support::load_settings(cli)?;
    let (compressor, skip_compression) = resolve_compressor(&args.compressor, args.no_compress)?;
    settings.compressor = compressor;
    if skip_compression {
        settings.skip_compression = true;
    }

    let credentials = support::credentials(&paths, &settings, args.anonymous)?;
    let depot_names = support::depot_names(&paths);

    let mut jobs = Vec::new();
    for file in &args.queue_files {
        jobs.extend(load_game_queue(file)?.jobs);
    }
    if let Some(appid) = &args.appid {
        jobs.push(
            GameJob::new(Platform::from(args.os), appid.clone()).with_branch(
                args.branch.clone(),
                args.branch_password.clone().unwrap_or_default(),
            ),
        );
    }
    if jobs.is_empty() {
        return Err("no jobs: provide --appid or --queue".to_string());
    }

    let options = PackOptions {
        settings,
        paths,
        downloader_kind: args.downloader.map(Into::into).unwrap_or_default(),
        dry_run: args.dry_run,
        credentials,
        depot_names,
    };
    let sink = support::event_sink(cli.verbose);
    let mut failed = 0;

    for job in &jobs {
        let packer = Packer::new(options.clone());
        let outcome = packer.run(job, sink.clone());
        match &outcome.status {
            PackStatus::Completed | PackStatus::Skipped => {
                if let Some(output) = &outcome.output {
                    println!("{}", output.display());
                }
                println!(
                    "{} — {:?} en {:.1}s",
                    outcome.stem,
                    outcome.status,
                    outcome.duration.as_secs_f32()
                );
            }
            PackStatus::Failed(message) => {
                failed += 1;
                eprintln!("{} - failed: {message}", outcome.stem);
            }
            PackStatus::Cancelled => {
                failed += 1;
                eprintln!("{} - cancelled", outcome.stem);
            }
        }
    }

    Ok(if failed > 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    })
}

fn workshop(cli: &Cli, action: &WorkshopAction) -> Result<ExitCode, String> {
    match action {
        WorkshopAction::Add {
            appid,
            workshop_id,
            file,
        } => {
            let path = workshop_queue_file(cli, file);
            let mut load = load_workshop_queue(&path)?;
            let job = WorkshopJob::new(appid.clone(), workshop_id.clone());
            job.validate()?;
            if load.jobs.contains(&job) {
                return Err("this item is already queued".to_string());
            }
            load.jobs.push(job);
            save_workshop_queue(&path, &load.jobs)?;
            println!("Item added ({}).", path.display());
        }
        WorkshopAction::List { file } => {
            let path = workshop_queue_file(cli, file);
            let load = load_workshop_queue(&path)?;
            if load.jobs.is_empty() {
                println!("Queue is empty ({})", path.display());
            }
            for (index, job) in load.jobs.iter().enumerate() {
                println!("{:>3}. {:>8} / {}", index + 1, job.appid, job.workshop_id);
            }
            for warning in load.warnings {
                eprintln!("warning: {warning}");
            }
        }
        WorkshopAction::Import { source, file } => {
            let path = workshop_queue_file(cli, file);
            let incoming = load_workshop_queue(source)?;
            let mut load = load_workshop_queue(&path)?;
            let mut added = 0;
            for job in incoming.jobs {
                if !load.jobs.contains(&job) {
                    load.jobs.push(job);
                    added += 1;
                }
            }
            save_workshop_queue(&path, &load.jobs)?;
            println!("{added} item(s) imported.");
        }
        WorkshopAction::Export { destination, file } => {
            let path = workshop_queue_file(cli, file);
            let load = load_workshop_queue(&path)?;
            save_workshop_queue(destination, &load.jobs)?;
            println!("Queue exported to {}.", destination.display());
        }
        WorkshopAction::Run {
            queue_files,
            downloader,
            compressor,
            no_compress,
            anonymous,
        } => {
            let (paths, mut settings) = support::load_settings(cli)?;
            let (compressor, skip) = resolve_compressor(compressor, *no_compress)?;
            settings.compressor = compressor;
            if skip {
                settings.skip_compression = true;
            }
            let credentials = support::credentials(&paths, &settings, *anonymous)?;

            let mut jobs = Vec::new();
            for file in queue_files {
                jobs.extend(load_workshop_queue(file)?.jobs);
            }
            if jobs.is_empty() {
                return Err("no items: provide --queue".to_string());
            }

            let options = PackOptions {
                settings,
                paths,
                downloader_kind: downloader.map(Into::into).unwrap_or_default(),
                dry_run: false,
                credentials,
                depot_names: DepotNames::default(),
            };
            let sink = support::event_sink(cli.verbose);
            let mut failed = 0;
            for job in &jobs {
                let packer = WorkshopPacker::new(options.clone());
                let outcome = packer.run(job, sink.clone());
                println!("{outcome}");
                if matches!(
                    outcome.status,
                    PackStatus::Failed(_) | PackStatus::Cancelled
                ) {
                    failed += 1;
                }
            }
            return Ok(if failed > 0 {
                ExitCode::from(1)
            } else {
                ExitCode::SUCCESS
            });
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn depots(cli: &Cli, action: &DepotsAction) -> Result<ExitCode, String> {
    match action {
        DepotsAction::Sync => {
            let (paths, _) = support::load_settings(cli)?;
            let names = DepotNames::from_official_source()?;
            names
                .save(paths.depots_file())
                .map_err(|error| error.to_string())?;
            println!("{} depot names synchronized.", names.len());
        }
        DepotsAction::Lookup { depot_id } => {
            let (paths, _) = support::load_settings(cli)?;
            let names = support::depot_names(&paths);
            match names.get(depot_id) {
                Some(name) => println!("{depot_id} = {name}"),
                None => {
                    eprintln!("unknown depot {depot_id}");
                    return Ok(ExitCode::from(1));
                }
            }
        }
        DepotsAction::List { search } => {
            let (paths, _) = support::load_settings(cli)?;
            let names = support::depot_names(&paths);
            let needle = search.as_deref().map(str::to_lowercase);
            let mut shown = 0;
            for (depot_id, name) in names.entries() {
                let matches = needle
                    .as_deref()
                    .map(|needle| {
                        name.to_lowercase().contains(needle)
                            || depot_id.to_string().contains(needle)
                    })
                    .unwrap_or(true);
                if matches {
                    println!("{depot_id} = {name}");
                    shown += 1;
                }
            }
            println!("{shown} depot(s).");
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn template_command(cli: &Cli, action: &TemplateAction) -> Result<ExitCode, String> {
    match action {
        TemplateAction::Render { job, template } => {
            let _ = support::load_settings(cli)?;
            let data: TemplateData = serde_json::from_str(
                &std::fs::read_to_string(job).map_err(|error| error.to_string())?,
            )
            .map_err(|error| format!("JSON du job : {error}"))?;
            let template = std::fs::read_to_string(template).map_err(|error| error.to_string())?;
            print!(
                "{}",
                steam_press_core::template::render_custom(&template, &data)
            );
        }
    }
    Ok(ExitCode::SUCCESS)
}
