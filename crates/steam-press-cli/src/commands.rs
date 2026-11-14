use crate::support;
use crate::{
    Cli, Command, DepotsAction, DiffArgs, ImportArgs, InfoArgs, LoginArgs, PackArgs, QueueAction,
    SearchArgs, TemplateAction, WorkshopAction,
};
use std::path::PathBuf;
use std::process::ExitCode;

use steam_press_core::archive;
use steam_press_core::compressor::Compressor;
use steam_press_core::config;
use steam_press_core::credentials::CredentialStore;
use steam_press_core::depots::DepotNames;
use steam_press_core::diff;
use steam_press_core::downloader::{self, Downloader};
use steam_press_core::import::{self, ImportOptions};
use steam_press_core::pack::{PackOptions, PackStatus, Packer};
use steam_press_core::queue::{
    load_game_queue, load_workshop_queue, save_game_queue, save_workshop_queue, GameJob,
    WorkshopJob,
};
use steam_press_core::template::{self, TemplateData};
use steam_press_core::tooling;
use steam_press_core::workshop::WorkshopPacker;
use steam_press_core::{steam, Platform};

pub const JSON_SCHEMA_VERSION: u32 = 1;

fn json_document(mut value: serde_json::Value) -> serde_json::Value {
    if let Some(object) = value.as_object_mut() {
        object.insert(
            "schemaVersion".to_string(),
            serde_json::json!(JSON_SCHEMA_VERSION),
        );
    }
    value
}

fn print_json(value: serde_json::Value) -> Result<(), String> {
    let value = json_document(value);
    println!(
        "{}",
        serde_json::to_string_pretty(&value).map_err(|error| format!("JSON: {error}"))?
    );
    Ok(())
}

fn outcome_json(outcome: &steam_press_core::PackOutcome) -> serde_json::Value {
    let (status, error) = match &outcome.status {
        PackStatus::Completed => ("completed", None),
        PackStatus::Skipped => ("skipped", None),
        PackStatus::Failed(message) => ("failed", Some(message.clone())),
        PackStatus::Cancelled => ("cancelled", None),
    };
    serde_json::json!({
        "job": outcome.stem,
        "status": status,
        "error": error,
        "output": outcome.output.as_ref().map(|path| path.display().to_string()),
        "infoFiles": outcome.info_files.iter().map(|path| path.display().to_string()).collect::<Vec<_>>(),
        "buildId": outcome.build_id,
        "durationMs": outcome.duration.as_millis(),
        "depots": outcome.depots.iter().map(|depot| serde_json::json!({
            "depotId": depot.depot_id,
            "manifestId": depot.manifest_id,
            "size": depot.size_bytes,
            "sharedOwner": depot.shared_owner,
        })).collect::<Vec<_>>(),
    })
}

pub fn dispatch(cli: Cli) -> Result<ExitCode, String> {
    match &cli.command {
        Command::Doctor => doctor(&cli),
        Command::Login(args) => login(&cli, args),
        Command::Info(args) => info(&cli, args),
        Command::Search(args) => search(&cli, args),
        Command::Queue { action } => queue(&cli, action),
        Command::Pack(args) => pack(&cli, args),
        Command::Import(args) => import(&cli, args),
        Command::Diff(args) => diff(&cli, args),
        Command::Workshop { action } => workshop(&cli, action),
        Command::Depots { action } => depots(&cli, action),
        Command::Template { action } => template_command(&cli, action),
    }
}

fn version_hint(found: &str, minimum: &str) -> String {
    if tooling::version_at_least(found, minimum) {
        format!("(minimum {minimum})")
    } else {
        format!("is older than {minimum}; update the tool")
    }
}

fn doctor(cli: &Cli) -> Result<ExitCode, String> {
    let (paths, settings) = support::load_settings(cli)?;
    let output = settings.output_dir(&paths);
    std::fs::create_dir_all(&output).map_err(|error| format!("output: {error}"))?;
    let probe = output.join(".steam-press-write-test");
    std::fs::write(&probe, b"ok").map_err(|error| format!("output is not writable: {error}"))?;
    let _ = std::fs::remove_file(&probe);

    let steamcmd = downloader::steamcmd::SteamCmd::from_settings(&settings).check();
    let depotdownloader =
        downloader::depotdownloader::DepotDownloader::from_settings(&settings).check();
    let sevenzip = archive::compressor_binary(Compressor::SevenZip, &settings).ok();
    let rar = archive::compressor_binary(Compressor::Rar, &settings).ok();
    let names = support::depot_names(&paths);
    let sevenzip_version = sevenzip
        .as_deref()
        .and_then(|path| tooling::probe_version(path, &["i"]));
    let rar_version = rar
        .as_deref()
        .and_then(|path| tooling::probe_version(path, &["-?"]));
    let depotdownloader_version = depotdownloader
        .path
        .as_deref()
        .and_then(|path| tooling::probe_version(path, &["--version"]));
    let version_info = |found: Option<&str>, minimum: &str| {
        let ok = found.map(|version| tooling::version_at_least(version, minimum));
        serde_json::json!({
            "found": found,
            "minimum": minimum,
            "ok": ok,
        })
    };
    let disk_free = config::available_space(&output).ok();
    let store = CredentialStore::new(&paths);
    let credentials_present = store.exists();
    let credentials_readable =
        credentials_present && store.load(&settings.key).ok().flatten().is_some();
    let network = steam::check_network();

    if cli.json {
        let tool = |status: downloader::ToolStatus| {
            serde_json::json!({
                "available": status.available,
                "path": status.path.map(|path| path.display().to_string()),
                "detail": status.detail,
            })
        };
        print_json(serde_json::json!({
            "ok": true,
            "configDir": paths.config_dir.display().to_string(),
            "workDir": paths.work_dir.display().to_string(),
            "outputDir": output.display().to_string(),
            "outputWritable": true,
            "archiver": settings.compressor.label(),
            "steamcmd": tool(steamcmd.clone()),
            "depotdownloader": tool(depotdownloader.clone()),
            "sevenzip": sevenzip.map(|path| path.display().to_string()),
            "rar": rar.map(|path| path.display().to_string()),
            "knownDepots": names.len(),
            "toolVersions": {
                "sevenzip": version_info(sevenzip_version.as_deref(), "21.0"),
                "rar": version_info(rar_version.as_deref(), "6.0"),
                "depotdownloader": version_info(
                    depotdownloader_version.as_deref(),
                    "3.0.0"
                ),
                "steamcmd": serde_json::json!({
                    "found": null,
                    "minimum": null,
                    "ok": null
                }),
            },
            "diskFreeBytes": disk_free,
            "credentials": {
                "present": credentials_present,
                "readable": credentials_readable,
            },
            "network": {
                "reachable": network.is_ok(),
                "detail": network.err(),
            },
        }))?;
        return Ok(ExitCode::SUCCESS);
    }

    println!("Config  : {}", paths.config_dir.display());
    println!("Work    : {}", paths.work_dir.display());
    println!("Output  : {}", output.display());
    println!("Archiver: {}", settings.compressor.label());
    for (label, status, key) in [
        ("SteamCMD", steamcmd, "steamcmdpath"),
        ("DepotDownloader", depotdownloader, "depotdownloaderpath"),
    ] {
        match status.path {
            Some(path) => println!("{label:<14}: {}", path.display()),
            None => println!(
                "{label:<14}: {} (install it or set {key} in settings)",
                status.detail
            ),
        }
    }
    for (label, path, key) in [("7-Zip", sevenzip, "sevenzippath"), ("RAR", rar, "rarpath")] {
        match path {
            Some(path) => println!("{label:<14}: {}", path.display()),
            None => println!("{label:<14}: not found (install it or set {key} in settings)"),
        }
    }
    if let Some(version) = &sevenzip_version {
        println!(
            "  7-Zip version : {version} {}",
            version_hint(version, "21.0")
        );
    }
    if let Some(version) = &rar_version {
        println!(
            "  RAR version   : {version} {}",
            version_hint(version, "6.0")
        );
    }
    if let Some(version) = &depotdownloader_version {
        println!(
            "  DepotDownloader version: {version} {}",
            version_hint(version, "3.0.0")
        );
    }
    println!("Known depots: {}", names.len());
    println!("Output  : writable");
    if let Some(bytes) = disk_free {
        println!(
            "Free    : {:.1} GiB",
            bytes as f64 / 1024.0 / 1024.0 / 1024.0
        );
    } else {
        println!("Free    : unknown (check the output path and its permissions)");
    }
    println!(
        "Login   : {}",
        if credentials_readable {
            "credentials readable"
        } else if credentials_present {
            "credentials present but unreadable (run `steam-press login` again)"
        } else {
            "no saved credentials (run `steam-press login` or use --anonymous)"
        }
    );
    match network {
        Ok(()) => println!("Network : Steam API reachable"),
        Err(error) => println!(
            "Network : unavailable ({error}; check proxy/firewall access to api.steampowered.com)"
        ),
    }
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
        if cli.json {
            print_json(serde_json::json!({"ok": true, "anonymous": true}))?;
        } else {
            println!("Anonymous mode: no credentials stored.");
        }
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
    if cli.json {
        print_json(serde_json::json!({"ok": true, "username": username}))?;
    } else {
        println!("Credentials saved for {username}.");
    }
    Ok(ExitCode::SUCCESS)
}

fn info(cli: &Cli, args: &InfoArgs) -> Result<ExitCode, String> {
    let (_paths, _settings) = support::load_settings(cli)?;
    let app = steam::fetch_app_info(&args.appid, &args.branch)?;
    if cli.json {
        let store = steam::fetch_store_info(&args.appid).ok();
        let payload = serde_json::json!({
            "appid": app.appid,
            "name": app.name,
            "installDir": app.install_dir,
            "buildId": app.build_id,
            "buildTime": app.build_time.map(template::format_utc_datetime),
            "depots": app.depots.iter().map(|depot| serde_json::json!({
                "depotId": depot.depot_id,
                "name": depot.name,
                "manifestId": depot.manifest_id,
                "dlcAppid": depot.dlc_appid,
                "size": depot.size_bytes,
                "sharedOwner": depot.shared_owner,
            })).collect::<Vec<_>>(),
            "store": store.map(|store| serde_json::json!({
                "name": store.name,
                "shortDescription": store.short_description,
                "website": store.website,
            })),
        });
        print_json(payload)?;
        return Ok(ExitCode::SUCCESS);
    }
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

fn search(cli: &Cli, args: &SearchArgs) -> Result<ExitCode, String> {
    let term = args.term.trim();
    if term.chars().count() < 2 {
        return Err("search term must be at least two characters".to_string());
    }
    let results = steam::search_store(term)?;
    if cli.json {
        let payload: Vec<_> = results
            .iter()
            .map(|item| serde_json::json!({"appid": item.appid, "name": item.name}))
            .collect();
        print_json(serde_json::json!({"ok": true, "results": payload}))?;
        return Ok(ExitCode::SUCCESS);
    }
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
            if cli.json {
                let jobs: Vec<_> = load
                    .jobs
                    .iter()
                    .map(|job| {
                        serde_json::json!({
                            "platform": job.platform.key(),
                            "appid": job.appid,
                            "branch": job.branch,
                            "protected": !job.branch_password.is_empty(),
                        })
                    })
                    .collect();
                print_json(serde_json::json!({
                    "ok": true, "file": path.display().to_string(), "jobs": jobs, "warnings": load.warnings
                }))?;
                return Ok(ExitCode::SUCCESS);
            }
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
            if cli.json {
                print_json(
                    serde_json::json!({"ok": true, "file": path.display().to_string(), "count": load.jobs.len()}),
                )?;
            } else {
                println!("Job added ({}).", path.display());
            }
        }
        QueueAction::Remove { index, file } => {
            let path = game_queue_file(cli, file);
            let mut load = load_game_queue(&path)?;
            if *index == 0 || *index > load.jobs.len() {
                return Err(format!("invalid index: {index}"));
            }
            load.jobs.remove(index - 1);
            save_game_queue(&path, &load.jobs)?;
            if cli.json {
                print_json(
                    serde_json::json!({"ok": true, "file": path.display().to_string(), "count": load.jobs.len()}),
                )?;
            } else {
                println!("Job removed.");
            }
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
            if cli.json {
                print_json(
                    serde_json::json!({"ok": true, "file": path.display().to_string(), "count": load.jobs.len()}),
                )?;
            } else {
                println!("Queue reordered.");
            }
        }
        QueueAction::Clear { file } => {
            let path = game_queue_file(cli, file);
            save_game_queue(&path, &[])?;
            if cli.json {
                print_json(
                    serde_json::json!({"ok": true, "file": path.display().to_string(), "count": 0}),
                )?;
            } else {
                println!("Queue cleared.");
            }
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
            if cli.json {
                print_json(
                    serde_json::json!({"ok": true, "file": path.display().to_string(), "added": added, "count": load.jobs.len(), "warnings": incoming.warnings}),
                )?;
                return Ok(ExitCode::SUCCESS);
            }
            println!("{added} job(s) imported.");
            for warning in incoming.warnings {
                eprintln!("warning: {warning}");
            }
        }
        QueueAction::Export { destination, file } => {
            let path = game_queue_file(cli, file);
            let load = load_game_queue(&path)?;
            save_game_queue(destination, &load.jobs)?;
            if cli.json {
                print_json(
                    serde_json::json!({"ok": true, "source": path.display().to_string(), "destination": destination.display().to_string(), "count": load.jobs.len()}),
                )?;
            } else {
                println!("Queue exported to {}.", destination.display());
            }
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
        fresh: args.fresh,
        credentials,
        depot_names,
    };
    let sink = if cli.json {
        steam_press_core::events::EventSink::noop()
    } else {
        support::event_sink(cli.verbose)
    };
    let mut failed = 0;
    let mut results = Vec::new();

    for job in &jobs {
        let packer = Packer::new(options.clone());
        let outcome = packer.run(job, sink.clone());
        results.push(outcome_json(&outcome));
        match &outcome.status {
            PackStatus::Completed | PackStatus::Skipped => {
                if !cli.json {
                    if let Some(output) = &outcome.output {
                        println!("{}", output.display());
                        let checksum = output.with_file_name(format!("{}.sha256", outcome.stem));
                        if checksum.is_file() {
                            println!("Checksum: {}", checksum.display());
                        }
                    }
                    println!(
                        "{} — {:?} en {:.1}s",
                        outcome.stem,
                        outcome.status,
                        outcome.duration.as_secs_f32()
                    );
                }
            }
            PackStatus::Failed(message) => {
                failed += 1;
                if !cli.json {
                    eprintln!("{} - failed: {message}", outcome.stem);
                }
            }
            PackStatus::Cancelled => {
                failed += 1;
                if !cli.json {
                    eprintln!("{} - cancelled", outcome.stem);
                }
            }
        }
    }

    if cli.json {
        print_json(serde_json::json!({"ok": failed == 0, "jobs": results}))?;
    }

    Ok(if failed > 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    })
}

fn import(cli: &Cli, args: &ImportArgs) -> Result<ExitCode, String> {
    let (paths, mut settings) = support::load_settings(cli)?;
    let (compressor, skip) = resolve_compressor(&args.compressor, args.no_compress)?;
    settings.compressor = compressor;
    if skip {
        settings.skip_compression = true;
    }
    let sink = if cli.json {
        steam_press_core::events::EventSink::noop()
    } else {
        support::event_sink(cli.verbose)
    };
    let options = ImportOptions {
        settings,
        paths,
        source: args.path.clone(),
        appid: args.appid.clone(),
        branch: args.branch.clone(),
        platform: Platform::from(args.os),
        name: args.name.clone(),
        install_dir: args.install_dir.clone(),
        fresh: args.fresh,
        depot_names: support::depot_names(&support::paths(cli)),
    };
    let outcome = import::import_local(&options, sink).map_err(|error| error.to_string())?;
    if cli.json {
        print_json(serde_json::json!({
            "ok": true,
            "job": outcome.stem,
            "status": match outcome.status { PackStatus::Skipped => "skipped", _ => "completed" },
            "output": outcome.output.as_ref().map(|path| path.display().to_string()),
            "manifest": outcome.manifest.as_ref().map(|path| path.display().to_string()),
            "copiedBytes": outcome.copied_bytes,
            "durationMs": outcome.duration.as_millis(),
        }))?;
    } else {
        println!(
            "{} — {:?} en {:.1}s",
            outcome.stem,
            outcome.status,
            outcome.duration.as_secs_f32()
        );
        if let Some(output) = &outcome.output {
            println!("{}", output.display());
        }
        if let Some(manifest) = &outcome.manifest {
            println!("Manifest: {}", manifest.display());
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn diff(cli: &Cli, args: &DiffArgs) -> Result<ExitCode, String> {
    let (paths, mut settings) = support::load_settings(cli)?;
    let (compressor, skip) = resolve_compressor(&args.compressor, args.no_compress)?;
    settings.compressor = compressor;
    if skip {
        settings.skip_compression = true;
    }
    let stem = args.stem.clone().unwrap_or_else(|| {
        let old = args
            .old
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "old".to_string());
        let new = args
            .new
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "new".to_string());
        format!(
            "{}-to-{}",
            steam_press_core::naming::sanitize_game_name(&old),
            steam_press_core::naming::sanitize_game_name(&new)
        )
    });
    let (report, report_path, output) = if args.archive {
        let archive = diff::create_update_archive(
            &settings,
            &paths,
            &args.old,
            &args.new,
            &stem,
            &support::event_sink(cli.verbose),
        )
        .map_err(|error| error.to_string())?;
        (archive.report, Some(archive.report_path), archive.output)
    } else {
        (
            diff::diff_directories(&args.old, &args.new)
                .map_err(|error| format!("diff: {error}"))?,
            None,
            None,
        )
    };
    if cli.json {
        print_json(serde_json::json!({
            "ok": true,
            "stem": stem,
            "report": report,
            "reportPath": report_path.as_ref().map(|path| path.display().to_string()),
            "output": output.as_ref().map(|path| path.display().to_string()),
        }))?;
    } else {
        println!(
            "{} added, {} modified, {} removed, {} unchanged",
            report.added.len(),
            report.modified.len(),
            report.removed.len(),
            report.unchanged
        );
        for change in report.changes() {
            println!("{:?}\t{}", change.kind, change.path);
        }
        if let Some(path) = report_path {
            println!("Report: {}", path.display());
        }
        if let Some(path) = output {
            println!("Archive: {}", path.display());
        }
    }
    Ok(ExitCode::SUCCESS)
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
            if cli.json {
                print_json(
                    serde_json::json!({"ok": true, "file": path.display().to_string(), "count": load.jobs.len()}),
                )?;
            } else {
                println!("Item added ({}).", path.display());
            }
        }
        WorkshopAction::List { file } => {
            let path = workshop_queue_file(cli, file);
            let load = load_workshop_queue(&path)?;
            if cli.json {
                print_json(serde_json::json!({
                    "ok": true, "file": path.display().to_string(),
                    "jobs": load.jobs.iter().map(|job| serde_json::json!({"appid": job.appid, "workshopId": job.workshop_id})).collect::<Vec<_>>(),
                    "warnings": load.warnings
                }))?;
                return Ok(ExitCode::SUCCESS);
            }
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
            if cli.json {
                print_json(
                    serde_json::json!({"ok": true, "file": path.display().to_string(), "added": added, "count": load.jobs.len()}),
                )?;
            } else {
                println!("{added} item(s) imported.");
            }
        }
        WorkshopAction::Export { destination, file } => {
            let path = workshop_queue_file(cli, file);
            let load = load_workshop_queue(&path)?;
            save_workshop_queue(destination, &load.jobs)?;
            if cli.json {
                print_json(
                    serde_json::json!({"ok": true, "source": path.display().to_string(), "destination": destination.display().to_string(), "count": load.jobs.len()}),
                )?;
            } else {
                println!("Queue exported to {}.", destination.display());
            }
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
                fresh: false,
                credentials,
                depot_names: DepotNames::default(),
            };
            let sink = if cli.json {
                steam_press_core::events::EventSink::noop()
            } else {
                support::event_sink(cli.verbose)
            };
            let mut failed = 0;
            let mut results = Vec::new();
            for job in &jobs {
                let packer = WorkshopPacker::new(options.clone());
                let outcome = packer.run(job, sink.clone());
                results.push(serde_json::json!({
                    "job": outcome.stem,
                    "status": match &outcome.status {
                        PackStatus::Completed => "completed",
                        PackStatus::Skipped => "skipped",
                        PackStatus::Failed(_) => "failed",
                        PackStatus::Cancelled => "cancelled",
                    },
                    "output": outcome.output.as_ref().map(|path| path.display().to_string()),
                    "durationMs": outcome.duration.as_millis(),
                }));
                if !cli.json {
                    println!("{outcome}");
                    if let Some(output) = &outcome.output {
                        let checksum = output.with_file_name(format!("{}.sha256", outcome.stem));
                        if checksum.is_file() {
                            println!("Checksum: {}", checksum.display());
                        }
                    }
                }
                if matches!(
                    outcome.status,
                    PackStatus::Failed(_) | PackStatus::Cancelled
                ) {
                    failed += 1;
                }
            }
            if cli.json {
                print_json(serde_json::json!({"ok": failed == 0, "jobs": results}))?;
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
            if cli.json {
                print_json(serde_json::json!({"ok": true, "count": names.len()}))?;
            } else {
                println!("{} depot names synchronized.", names.len());
            }
        }
        DepotsAction::Lookup { depot_id } => {
            let (paths, _) = support::load_settings(cli)?;
            let names = support::depot_names(&paths);
            match names.get(depot_id) {
                Some(name) if cli.json => {
                    print_json(serde_json::json!({"ok": true, "depotId": depot_id, "name": name}))?;
                }
                Some(name) => println!("{depot_id} = {name}"),
                None => {
                    if cli.json {
                        print_json(
                            serde_json::json!({"ok": false, "error": format!("unknown depot {depot_id}")}),
                        )?;
                    } else {
                        eprintln!("unknown depot {depot_id}");
                    }
                    return Ok(ExitCode::from(1));
                }
            }
        }
        DepotsAction::List { search } => {
            let (paths, _) = support::load_settings(cli)?;
            let names = support::depot_names(&paths);
            let needle = search.as_deref().map(str::to_lowercase);
            let mut shown = 0;
            let mut rows = Vec::new();
            for (depot_id, name) in names.entries() {
                let matches = needle
                    .as_deref()
                    .map(|needle| {
                        name.to_lowercase().contains(needle)
                            || depot_id.to_string().contains(needle)
                    })
                    .unwrap_or(true);
                if matches {
                    if !cli.json {
                        println!("{depot_id} = {name}");
                    }
                    rows.push(serde_json::json!({"depotId": depot_id, "name": name}));
                    shown += 1;
                }
            }
            if cli.json {
                print_json(serde_json::json!({"ok": true, "count": shown, "depots": rows}))?;
            } else {
                println!("{shown} depot(s).");
            }
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
            let rendered = steam_press_core::template::render_custom(&template, &data);
            if cli.json {
                print_json(serde_json::json!({"ok": true, "rendered": rendered}))?;
            } else {
                print!("{rendered}");
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_object_document_has_the_schema_version() {
        let document = json_document(serde_json::json!({"ok": true, "count": 2}));
        assert_eq!(document["schemaVersion"], 1);
        assert_eq!(document["count"], 2);
    }

    #[test]
    fn version_hints_flag_outdated_tools() {
        assert_eq!(version_hint("21.0", "21.0"), "(minimum 21.0)");
        assert_eq!(version_hint("23.01", "21.0"), "(minimum 21.0)");
        assert!(version_hint("16.02", "21.0").contains("older than 21.0"));
    }
}
