//! egui desktop interface for Steam Press.

use crate::state::{
    EntryStatus, GameQueue, WorkshopQueue, WorkerEvent,
};
use eframe::egui;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use steam_press_core::compressor::Compressor;
use steam_press_core::config::{Paths, Settings};
use steam_press_core::credentials::{CredentialStore, SavedCredentials};
use steam_press_core::depots::DepotNames;
use steam_press_core::downloader::{Credentials, DownloaderKind};
use steam_press_core::events::{EventSink, PackEvent, Stage};
use steam_press_core::pack::{PackOptions, Packer};
use steam_press_core::platform::Platform;
use steam_press_core::queue::{GameJob, WorkshopJob};
use steam_press_core::tooling;
use steam_press_core::workshop::WorkshopPacker;

type CancelHook = Box<dyn Fn() + Send + Sync>;
type SharedHook = Arc<Mutex<Option<CancelHook>>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Game,
    Workshop,
    Depots,
    Settings,
}

#[derive(Debug, Clone)]
struct GameForm {
    appid: String,
    platform: Platform,
    branch: String,
    branch_password: String,
}

impl Default for GameForm {
    fn default() -> Self {
        Self {
            appid: String::new(),
            platform: Platform::Linux64,
            branch: "public".to_string(),
            branch_password: String::new(),
        }
    }
}

#[derive(Debug, Clone, Default)]
struct WorkshopForm {
    appid: String,
    workshop_id: String,
}

#[derive(Debug, Clone, Default)]
struct LoginForm {
    username: String,
    password: String,
}

enum UiAction {
    StartGames,
    StartWorkshop,
    Cancel,
    SaveSettings,
    SaveLogin,
    DeleteLogin,
    SyncDepots,
    OpenOutput,
    ImportGames,
    ExportGames,
    ImportWorkshop,
    ExportWorkshop,
}

pub struct SteamPressApp {
    paths: Paths,
    settings: Settings,
    downloader_kind: DownloaderKind,
    tab: Tab,
    game_queue: GameQueue,
    workshop_queue: WorkshopQueue,
    game_form: GameForm,
    workshop_form: WorkshopForm,
    login: LoginForm,
    depot_names: DepotNames,
    depot_search: String,
    depot_notice: Option<String>,
    queue_io_path: String,
    workshop_io_path: String,
    output_dir_text: String,
    steamcmd_text: String,
    depotdownloader_text: String,
    sevenzip_text: String,
    rar_text: String,
    template_text: String,
    custom_options_text: String,
    stage: String,
    progress: f32,
    log: Vec<String>,
    show_log: bool,
    status_message: Option<String>,
    error: Option<String>,
    running: bool,
    allow_close: bool,
    close_dialog: bool,
    receiver: Option<Receiver<WorkerEvent>>,
    cancel_hook: Option<SharedHook>,
}

impl SteamPressApp {
    pub fn new(_context: &eframe::CreationContext<'_>) -> Self {
        let paths = Paths::discover();
        let _ = paths.ensure();
        let mut settings = Settings::load(&paths);
        if settings.key.trim().is_empty() {
            settings.ensure_key();
            let _ = settings.save(&paths);
        }

        let depot_names = DepotNames::load(paths.depots_file()).unwrap_or_default();
        let saved = CredentialStore::new(&paths)
            .load(&settings.key)
            .ok()
            .flatten();
        let login = LoginForm {
            username: saved
                .as_ref()
                .map(|saved| saved.username.clone())
                .unwrap_or_default(),
            password: saved
                .as_ref()
                .and_then(|saved| saved.password.clone())
                .unwrap_or_default(),
        };

        let output_dir_text = settings
            .output_dir
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default();
        let steamcmd_text = path_text(settings.steamcmd_path.as_ref());
        let depotdownloader_text = path_text(settings.depotdownloader_path.as_ref());
        let sevenzip_text = path_text(settings.sevenzip_path.as_ref());
        let rar_text = path_text(settings.rar_path.as_ref());
        let template_text = path_text(settings.template.as_ref());
        let custom_options_text = settings
            .custom_compress_options
            .clone()
            .unwrap_or_default();

        Self {
            queue_io_path: paths
                .config_dir
                .join("queue.sspq")
                .to_string_lossy()
                .into_owned(),
            workshop_io_path: paths
                .config_dir
                .join("queue.sswq")
                .to_string_lossy()
                .into_owned(),
            paths,
            settings,
            downloader_kind: DownloaderKind::SteamCmd,
            tab: Tab::Game,
            game_queue: GameQueue::default(),
            workshop_queue: WorkshopQueue::default(),
            game_form: GameForm::default(),
            workshop_form: WorkshopForm::default(),
            login,
            depot_names,
            depot_search: String::new(),
            depot_notice: None,
            output_dir_text,
            steamcmd_text,
            depotdownloader_text,
            sevenzip_text,
            rar_text,
            template_text,
            custom_options_text,
            stage: "Prêt".to_string(),
            progress: 0.0,
            log: Vec::new(),
            show_log: false,
            status_message: None,
            error: None,
            running: false,
            allow_close: false,
            close_dialog: false,
            receiver: None,
            cancel_hook: None,
        }
    }

    fn output_dir(&self) -> PathBuf {
        self.settings.output_dir(&self.paths)
    }

    fn push_log(&mut self, line: String) {
        self.log.push(line);
        if self.log.len() > 2000 {
            let overflow = self.log.len() - 2000;
            self.log.drain(0..overflow);
        }
    }

    fn current_credentials(&self) -> Result<Credentials, String> {
        let username = self.login.username.trim();
        if username.eq_ignore_ascii_case("anonymous") {
            return Ok(Credentials::anonymous());
        }
        if !username.is_empty() {
            return Ok(Credentials {
                username: username.to_string(),
                password: if self.login.password.is_empty() {
                    None
                } else {
                    Some(self.login.password.clone())
                },
            });
        }
        match CredentialStore::new(&self.paths).load(&self.settings.key)? {
            Some(saved) => Ok(Credentials {
                username: saved.username,
                password: saved.password,
            }),
            None => Err(
                "aucun identifiant : renseignez un compte ou utilisez « anonymous »".to_string(),
            ),
        }
    }

    fn pack_options(&self) -> Result<PackOptions, String> {
        Ok(PackOptions {
            settings: self.settings.clone(),
            paths: self.paths.clone(),
            downloader_kind: self.downloader_kind,
            dry_run: false,
            credentials: self.current_credentials()?,
            depot_names: self.depot_names.clone(),
        })
    }

    fn start_games(&mut self, context: &egui::Context) {
        if self.running {
            return;
        }
        let jobs: Vec<GameJob> = self.game_queue.jobs();
        if jobs.is_empty() {
            self.error = Some("la file de jeux est vide".to_string());
            return;
        }
        let options = match self.pack_options() {
            Ok(options) => options,
            Err(error) => {
                self.error = Some(error);
                return;
            }
        };

        let (sender, receiver) = mpsc::channel();
        let sink_sender = sender.clone();
        let sink = EventSink::new(move |event| {
            let _ = sink_sender.send(WorkerEvent::Event(event));
        });
        let hook: SharedHook = Arc::new(Mutex::new(None));
        let thread_hook = hook.clone();

        std::thread::spawn(move || {
            for job in jobs {
                let packer = Arc::new(Packer::new(options.clone()));
                if let Ok(mut slot) = thread_hook.lock() {
                    let packer = packer.clone();
                    *slot = Some(Box::new(move || packer.cancel()));
                }
                let _ = packer.run(&job, sink.clone());
            }
            if let Ok(mut slot) = thread_hook.lock() {
                *slot = None;
            }
            let _ = sender.send(WorkerEvent::Done);
        });

        self.begin_run(receiver, hook);
        context.request_repaint();
    }

    fn start_workshop(&mut self, context: &egui::Context) {
        if self.running {
            return;
        }
        let jobs: Vec<WorkshopJob> = self.workshop_queue.jobs();
        if jobs.is_empty() {
            self.error = Some("la file d'atelier est vide".to_string());
            return;
        }
        let options = match self.pack_options() {
            Ok(options) => options,
            Err(error) => {
                self.error = Some(error);
                return;
            }
        };

        let (sender, receiver) = mpsc::channel();
        let sink_sender = sender.clone();
        let sink = EventSink::new(move |event| {
            let _ = sink_sender.send(WorkerEvent::Event(event));
        });
        let hook: SharedHook = Arc::new(Mutex::new(None));
        let thread_hook = hook.clone();

        std::thread::spawn(move || {
            for job in jobs {
                let packer = Arc::new(WorkshopPacker::new(options.clone()));
                if let Ok(mut slot) = thread_hook.lock() {
                    let packer = packer.clone();
                    *slot = Some(Box::new(move || packer.cancel()));
                }
                let _ = packer.run(&job, sink.clone());
            }
            if let Ok(mut slot) = thread_hook.lock() {
                *slot = None;
            }
            let _ = sender.send(WorkerEvent::Done);
        });

        self.begin_run(receiver, hook);
        context.request_repaint();
    }

    fn begin_run(&mut self, receiver: Receiver<WorkerEvent>, hook: SharedHook) {
        self.error = None;
        self.status_message = None;
        self.progress = 0.0;
        self.stage = "Démarrage".to_string();
        self.log.clear();
        self.show_log = true;
        self.running = true;
        self.game_queue.reset_statuses();
        self.workshop_queue.reset_statuses();
        self.receiver = Some(receiver);
        self.cancel_hook = Some(hook);
    }

    fn cancel(&mut self) {
        if let Some(hook) = &self.cancel_hook {
            if let Ok(slot) = hook.lock() {
                if let Some(cancel) = slot.as_ref() {
                    cancel();
                }
            }
        }
        self.stage = "Annulation demandée…".to_string();
    }

    fn poll(&mut self) {
        let mut messages = Vec::new();
        if let Some(receiver) = &self.receiver {
            while let Ok(message) = receiver.try_recv() {
                messages.push(message);
            }
        }

        let mut done = false;
        for message in messages {
            match message {
                WorkerEvent::Event(event) => self.handle_event(event),
                WorkerEvent::Done => done = true,
            }
        }

        if done {
            self.running = false;
            self.receiver = None;
            self.cancel_hook = None;
            self.stage = "Terminé".to_string();
            self.progress = 1.0;
        }
    }

    fn handle_event(&mut self, event: PackEvent) {
        match event {
            PackEvent::Stage {
                job,
                stage,
                message,
            } => {
                self.stage = format!("{} — {message}", stage.label());
                self.progress = progress_for(stage);
                if matches!(stage, Stage::Preflight | Stage::Download) {
                    self.game_queue
                        .set_status(|entry| entry.encode() == job, EntryStatus::Running);
                }
                self.push_log(format!("[{}] {message}", stage.label()));
            }
            PackEvent::Progress { message, percent, .. } => {
                self.stage = message;
                if let Some(percent) = percent {
                    self.progress = percent;
                }
            }
            PackEvent::Log(line) => self.push_log(line),
            PackEvent::Finished {
                job,
                outcome,
                output,
            } => {
                let status = if outcome.starts_with("Échec") {
                    EntryStatus::Failed(outcome.clone())
                } else if outcome.contains("Ignoré") {
                    EntryStatus::Skipped
                } else if outcome.contains("Annulé") {
                    EntryStatus::Cancelled
                } else {
                    EntryStatus::Done
                };
                self.game_queue
                    .set_status(|entry| entry.encode() == job, status.clone());
                self.workshop_queue
                    .set_status(|entry| workshop_stem(entry) == job, status);
                self.push_log(match output {
                    Some(path) => format!("{outcome} — {}", path.display()),
                    None => outcome,
                });
            }
        }
    }

    fn save_settings(&mut self) {
        self.settings.output_dir = path_or_none(&self.output_dir_text);
        self.settings.steamcmd_path = path_or_none(&self.steamcmd_text);
        self.settings.depotdownloader_path = path_or_none(&self.depotdownloader_text);
        self.settings.sevenzip_path = path_or_none(&self.sevenzip_text);
        self.settings.rar_path = path_or_none(&self.rar_text);
        self.settings.template = path_or_none(&self.template_text);
        self.settings.custom_compress_options =
            if self.custom_options_text.trim().is_empty() {
                None
            } else {
                Some(self.custom_options_text.trim().to_string())
            };
        self.settings.ensure_key();
        match self.settings.save(&self.paths) {
            Ok(()) => self.status_message = Some("Réglages enregistrés.".to_string()),
            Err(error) => self.error = Some(format!("réglages : {error}")),
        }
    }

    fn save_login(&mut self) {
        let username = self.login.username.trim().to_string();
        if username.is_empty() {
            self.error = Some("nom d'utilisateur vide".to_string());
            return;
        }
        self.settings.ensure_key();
        if let Err(error) = self.settings.save(&self.paths) {
            self.error = Some(format!("réglages : {error}"));
            return;
        }
        if username.eq_ignore_ascii_case("anonymous") {
            let _ = CredentialStore::new(&self.paths).clear();
            self.login.password.clear();
            self.status_message = Some("Mode anonyme : identifiants supprimés.".to_string());
            return;
        }
        let credentials = SavedCredentials {
            username,
            password: if self.login.password.is_empty() {
                None
            } else {
                Some(self.login.password.clone())
            },
        };
        match CredentialStore::new(&self.paths).save(&credentials, &self.settings.key) {
            Ok(()) => self.status_message = Some("Identifiants enregistrés.".to_string()),
            Err(error) => self.error = Some(error),
        }
    }

    fn delete_login(&mut self) {
        match CredentialStore::new(&self.paths).clear() {
            Ok(()) => {
                self.login.username.clear();
                self.login.password.clear();
                self.status_message = Some("Identifiants supprimés.".to_string());
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    fn sync_depots(&mut self) {
        match DepotNames::from_official_source() {
            Ok(names) => match names.save(self.paths.depots_file()) {
                Ok(()) => {
                    let count = names.len();
                    self.depot_names = names;
                    self.depot_notice = Some(format!("{count} dépôts synchronisés."));
                }
                Err(error) => self.depot_notice = Some(format!("écriture : {error}")),
            },
            Err(error) => self.depot_notice = Some(error),
        }
    }

    fn import_games(&mut self) {
        match steam_press_core::queue::load_game_queue(&self.queue_io_path) {
            Ok(load) => {
                let mut added = 0;
                for job in load.jobs {
                    if self.game_queue.push(job) {
                        added += 1;
                    }
                }
                self.status_message = Some(format!("{added} job(s) importé(s)."));
                for warning in load.warnings {
                    self.push_log(format!("avertissement : {warning}"));
                }
            }
            Err(error) => self.error = Some(error),
        }
    }

    fn export_games(&mut self) {
        let jobs = self.game_queue.jobs();
        match steam_press_core::queue::save_game_queue(&self.queue_io_path, &jobs) {
            Ok(()) => self.status_message = Some("File exportée.".to_string()),
            Err(error) => self.error = Some(error),
        }
    }

    fn import_workshop(&mut self) {
        match steam_press_core::queue::load_workshop_queue(&self.workshop_io_path) {
            Ok(load) => {
                let mut added = 0;
                for job in load.jobs {
                    if self.workshop_queue.push(job) {
                        added += 1;
                    }
                }
                self.status_message = Some(format!("{added} item(s) importé(s)."));
            }
            Err(error) => self.error = Some(error),
        }
    }

    fn export_workshop(&mut self) {
        let jobs = self.workshop_queue.jobs();
        match steam_press_core::queue::save_workshop_queue(&self.workshop_io_path, &jobs) {
            Ok(()) => self.status_message = Some("File exportée.".to_string()),
            Err(error) => self.error = Some(error),
        }
    }

    fn apply_action(&mut self, action: UiAction, context: &egui::Context) {
        match action {
            UiAction::StartGames => self.start_games(context),
            UiAction::StartWorkshop => self.start_workshop(context),
            UiAction::Cancel => self.cancel(),
            UiAction::SaveSettings => self.save_settings(),
            UiAction::SaveLogin => self.save_login(),
            UiAction::DeleteLogin => self.delete_login(),
            UiAction::SyncDepots => self.sync_depots(),
            UiAction::OpenOutput => {
                let output = self.output_dir();
                let _ = std::fs::create_dir_all(&output);
                let _ = tooling::open_folder(&output);
            }
            UiAction::ImportGames => self.import_games(),
            UiAction::ExportGames => self.export_games(),
            UiAction::ImportWorkshop => self.import_workshop(),
            UiAction::ExportWorkshop => self.export_workshop(),
        }
    }

    fn top_bar(&mut self, context: &egui::Context) {
        egui::TopBottomPanel::top("tabs").show(context, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Steam Press");
                ui.separator();
                ui.selectable_value(&mut self.tab, Tab::Game, "Jeu");
                ui.selectable_value(&mut self.tab, Tab::Workshop, "Atelier");
                ui.selectable_value(&mut self.tab, Tab::Depots, "Dépôts");
                ui.selectable_value(&mut self.tab, Tab::Settings, "Réglages");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.running {
                        if ui.button("Annuler").clicked() {
                            self.cancel();
                        }
                        ui.spinner();
                    }
                    ui.label(if self.running { "En cours" } else { "Inactif" });
                });
            });
            if self.running || self.progress > 0.0 {
                ui.add(
                    egui::ProgressBar::new(self.progress)
                        .text(self.stage.clone())
                        .desired_width(f32::INFINITY),
                );
            }
        });
    }

    fn bottom_bar(&mut self, context: &egui::Context) {
        egui::TopBottomPanel::bottom("status").show(context, |ui| {
            ui.horizontal_wrapped(|ui| {
                if let Some(error) = &self.error {
                    ui.colored_label(egui::Color32::from_rgb(235, 100, 100), error);
                } else if let Some(message) = &self.status_message {
                    ui.colored_label(egui::Color32::from_rgb(120, 200, 120), message);
                } else {
                    ui.label(format!("Sortie : {}", self.output_dir().display()));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Ouvrir le dossier").clicked() {
                        let output = self.output_dir();
                        let _ = std::fs::create_dir_all(&output);
                        let _ = tooling::open_folder(&output);
                    }
                    if ui.button(if self.show_log { "Masquer le log" } else { "Afficher le log" }).clicked() {
                        self.show_log = !self.show_log;
                    }
                });
            });
        });
    }

    fn game_tab(&mut self, ui: &mut egui::Ui) -> Option<UiAction> {
        let mut action = None;
        ui.heading(format!("File de jeux ({})", self.game_queue.len()));
        ui.add_space(4.0);

        ui.horizontal_wrapped(|ui| {
            ui.label("AppID");
            ui.add(
                egui::TextEdit::singleline(&mut self.game_form.appid)
                    .desired_width(100.0)
                    .hint_text("2379780"),
            );
            egui::ComboBox::from_id_salt("platform")
                .selected_text(self.game_form.platform.label())
                .show_ui(ui, |ui| {
                    for platform in Platform::all() {
                        ui.selectable_value(&mut self.game_form.platform, *platform, platform.label());
                    }
                });
            ui.label("Branche");
            ui.add(
                egui::TextEdit::singleline(&mut self.game_form.branch)
                    .desired_width(110.0),
            );
            ui.add(
                egui::TextEdit::singleline(&mut self.game_form.branch_password)
                    .password(true)
                    .desired_width(110.0)
                    .hint_text("mot de passe"),
            );
            if ui.button("Ajouter").clicked() {
                let appid = self.game_form.appid.trim();
                if appid.parse::<u64>().map(|value| value > 0).unwrap_or(false) {
                    let job = GameJob::new(self.game_form.platform, appid)
                        .with_branch(self.game_form.branch.clone(), self.game_form.branch_password.clone());
                    if self.game_queue.push(job) {
                        self.game_form.appid.clear();
                        self.game_form.branch_password.clear();
                        self.error = None;
                    } else {
                        self.error = Some("ce job est déjà dans la file".to_string());
                    }
                } else {
                    self.error = Some("AppID invalide".to_string());
                }
            }
        });

        ui.add_space(6.0);
        ui.separator();

        let mut move_up = None;
        let mut move_down = None;
        let mut remove = None;
        egui::ScrollArea::vertical()
            .max_height(210.0)
            .show(ui, |ui| {
                if self.game_queue.is_empty() {
                    ui.weak("Aucun jeu dans la file.");
                }
                for (index, entry) in self.game_queue.entries().iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.monospace(format!("{:>2}.", index + 1));
                        ui.label(entry.job.platform.label());
                        ui.monospace(entry.job.appid.as_str());
                        ui.label(format!("branche {}", entry.job.branch));
                        ui.colored_label(status_color(&entry.status), entry.status.label());
                        if ui.small_button("↑").clicked() {
                            move_up = Some(index);
                        }
                        if ui.small_button("↓").clicked() {
                            move_down = Some(index);
                        }
                        if ui.small_button("✕").clicked() {
                            remove = Some(index);
                        }
                    });
                }
            });
        if let Some(index) = move_up {
            self.game_queue.move_up(index);
        }
        if let Some(index) = move_down {
            self.game_queue.move_down(index);
        }
        if let Some(index) = remove {
            self.game_queue.remove(index);
        }

        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    !self.running && !self.game_queue.is_empty(),
                    egui::Button::new("Démarrer"),
                )
                .clicked()
            {
                action = Some(UiAction::StartGames);
            }
            if ui
                .add_enabled(self.running, egui::Button::new("Annuler"))
                .clicked()
            {
                action = Some(UiAction::Cancel);
            }
            if ui
                .add_enabled(!self.running, egui::Button::new("Vider"))
                .clicked()
            {
                self.game_queue.clear();
            }
            ui.separator();
            ui.label("Fichier");
            ui.add(egui::TextEdit::singleline(&mut self.queue_io_path).desired_width(260.0));
            if ui.button("Importer").clicked() {
                action = Some(UiAction::ImportGames);
            }
            if ui.button("Exporter").clicked() {
                action = Some(UiAction::ExportGames);
            }
        });

        self.log_panel(ui);
        action
    }

    fn workshop_tab(&mut self, ui: &mut egui::Ui) -> Option<UiAction> {
        let mut action = None;
        ui.heading(format!("File d'atelier ({})", self.workshop_queue.len()));
        ui.add_space(4.0);

        ui.horizontal_wrapped(|ui| {
            ui.label("AppID");
            ui.add(
                egui::TextEdit::singleline(&mut self.workshop_form.appid)
                    .desired_width(100.0)
                    .hint_text("730"),
            );
            ui.label("Workshop ID");
            ui.add(
                egui::TextEdit::singleline(&mut self.workshop_form.workshop_id)
                    .desired_width(140.0),
            );
            if ui.button("Ajouter").clicked() {
                let job = WorkshopJob::new(
                    self.workshop_form.appid.trim(),
                    self.workshop_form.workshop_id.trim(),
                );
                match job.validate() {
                    Ok(()) if self.workshop_queue.push(job) => {
                        self.workshop_form = WorkshopForm::default();
                        self.error = None;
                    }
                    Ok(()) => self.error = Some("cet item est déjà dans la file".to_string()),
                    Err(error) => self.error = Some(error),
                }
            }
        });

        ui.add_space(6.0);
        ui.separator();

        let mut move_up = None;
        let mut move_down = None;
        let mut remove = None;
        egui::ScrollArea::vertical()
            .max_height(210.0)
            .show(ui, |ui| {
                if self.workshop_queue.is_empty() {
                    ui.weak("Aucun item dans la file.");
                }
                for (index, entry) in self.workshop_queue.entries().iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.monospace(format!("{:>2}.", index + 1));
                        ui.monospace(entry.job.appid.as_str());
                        ui.monospace(entry.job.workshop_id.as_str());
                        ui.colored_label(status_color(&entry.status), entry.status.label());
                        if ui.small_button("↑").clicked() {
                            move_up = Some(index);
                        }
                        if ui.small_button("↓").clicked() {
                            move_down = Some(index);
                        }
                        if ui.small_button("✕").clicked() {
                            remove = Some(index);
                        }
                    });
                }
            });
        if let Some(index) = move_up {
            self.workshop_queue.move_up(index);
        }
        if let Some(index) = move_down {
            self.workshop_queue.move_down(index);
        }
        if let Some(index) = remove {
            self.workshop_queue.remove(index);
        }

        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    !self.running && !self.workshop_queue.is_empty(),
                    egui::Button::new("Démarrer"),
                )
                .clicked()
            {
                action = Some(UiAction::StartWorkshop);
            }
            if ui
                .add_enabled(self.running, egui::Button::new("Annuler"))
                .clicked()
            {
                action = Some(UiAction::Cancel);
            }
            ui.separator();
            ui.label("Fichier");
            ui.add(egui::TextEdit::singleline(&mut self.workshop_io_path).desired_width(260.0));
            if ui.button("Importer").clicked() {
                action = Some(UiAction::ImportWorkshop);
            }
            if ui.button("Exporter").clicked() {
                action = Some(UiAction::ExportWorkshop);
            }
        });

        self.log_panel(ui);
        action
    }

    fn depots_tab(&mut self, ui: &mut egui::Ui) -> Option<UiAction> {
        let mut action = None;
        ui.heading("Dépôts Steam");
        ui.label(format!("{} noms connus", self.depot_names.len()));
        ui.horizontal(|ui| {
            if ui.button("Synchroniser SteamDepotNames").clicked() {
                action = Some(UiAction::SyncDepots);
            }
            ui.add(
                egui::TextEdit::singleline(&mut self.depot_search)
                    .hint_text("rechercher un ID ou un nom")
                    .desired_width(260.0),
            );
        });
        if let Some(notice) = &self.depot_notice {
            ui.label(notice);
        }
        ui.separator();

        let needle = self.depot_search.trim().to_lowercase();
        let matches: Vec<(u32, String)> = self
            .depot_names
            .entries()
            .filter(|(id, name)| {
                needle.is_empty()
                    || name.to_lowercase().contains(&needle)
                    || id.to_string().contains(&needle)
            })
            .take(500)
            .map(|(id, name)| (id, name.to_string()))
            .collect();

        egui::ScrollArea::vertical().show(ui, |ui| {
            if matches.is_empty() {
                ui.weak("Aucun dépôt ne correspond.");
            }
            for (id, name) in matches {
                ui.horizontal(|ui| {
                    ui.monospace(id.to_string());
                    ui.label(name);
                });
            }
        });
        action
    }

    fn settings_tab(&mut self, ui: &mut egui::Ui) -> Option<UiAction> {
        let mut action = None;
        ui.heading("Réglages");
        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("settings-grid")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    ui.label("Téléchargeur");
                    egui::ComboBox::from_id_salt("downloader")
                        .selected_text(self.downloader_kind.label())
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.downloader_kind,
                                DownloaderKind::SteamCmd,
                                "SteamCMD",
                            );
                            ui.selectable_value(
                                &mut self.downloader_kind,
                                DownloaderKind::DepotDownloader,
                                "DepotDownloader",
                            );
                        });
                    ui.end_row();

                    ui.label("Compresseur");
                    egui::ComboBox::from_id_salt("compressor")
                        .selected_text(self.settings.compressor.label())
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.settings.compressor,
                                Compressor::SevenZip,
                                "7-Zip",
                            );
                            ui.selectable_value(
                                &mut self.settings.compressor,
                                Compressor::Rar,
                                "RAR",
                            );
                        });
                    ui.end_row();

                    ui.label("Options personnalisées");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.custom_options_text)
                            .hint_text("-mx9 -sdel -mhe=on -v5g")
                            .desired_width(360.0),
                    );
                    ui.end_row();

                    ui.label("Mot de passe d'archive");
                    ui.add(egui::TextEdit::singleline(&mut self.settings.compress_password));
                    ui.end_row();

                    ui.label("Compression désactivée");
                    ui.checkbox(&mut self.settings.skip_compression, "conserver le dossier");
                    ui.end_row();

                    ui.label("Dossier de sortie");
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.output_dir_text)
                                .desired_width(360.0),
                        );
                        if ui.button("Ouvrir").clicked() {
                            action = Some(UiAction::OpenOutput);
                        }
                    });
                    ui.end_row();

                    ui.label("SteamCMD");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.steamcmd_text)
                            .hint_text("steamcmd.sh dans le PATH")
                            .desired_width(360.0),
                    );
                    ui.end_row();

                    ui.label("DepotDownloader");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.depotdownloader_text)
                            .desired_width(360.0),
                    );
                    ui.end_row();

                    ui.label("7-Zip");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.sevenzip_text)
                            .hint_text("7z dans le PATH")
                            .desired_width(360.0),
                    );
                    ui.end_row();

                    ui.label("RAR");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.rar_text)
                            .desired_width(360.0),
                    );
                    ui.end_row();

                    ui.label("Template");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.template_text)
                            .hint_text("chemin vers un template à jetons")
                            .desired_width(360.0),
                    );
                    ui.end_row();

                    ui.label("Uploader");
                    ui.add(egui::TextEdit::singleline(&mut self.settings.uploader));
                    ui.end_row();

                    ui.label("Filehost");
                    ui.add(egui::TextEdit::singleline(&mut self.settings.filehost));
                    ui.end_row();

                    ui.label("Upload Crew");
                    ui.checkbox(&mut self.settings.upload_crew_mode, "générer les infos Upload Crew");
                    ui.end_row();

                    ui.label("Thème sombre");
                    ui.checkbox(&mut self.settings.dark_mode, "interface sombre");
                    ui.end_row();
                });

            ui.add_space(10.0);
            if ui.button("Enregistrer les réglages").clicked() {
                action = Some(UiAction::SaveSettings);
            }

            ui.add_space(14.0);
            ui.separator();
            ui.heading("Identifiants Steam");
            ui.label("Stockés chiffrés (AES-256-GCM) dans le dossier de configuration.");
            egui::Grid::new("login-grid")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    ui.label("Utilisateur");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.login.username)
                            .hint_text("anonymous ou compte Steam")
                            .desired_width(260.0),
                    );
                    ui.end_row();
                    ui.label("Mot de passe");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.login.password)
                            .password(true)
                            .desired_width(260.0),
                    );
                    ui.end_row();
                });
            ui.horizontal(|ui| {
                if ui.button("Enregistrer").clicked() {
                    action = Some(UiAction::SaveLogin);
                }
                if ui.button("Supprimer").clicked() {
                    action = Some(UiAction::DeleteLogin);
                }
            });
        });
        action
    }

    fn log_panel(&mut self, ui: &mut egui::Ui) {
        if !self.show_log {
            return;
        }
        ui.add_space(6.0);
        ui.separator();
        ui.horizontal(|ui| {
            ui.label("Journal");
            if ui.small_button("effacer").clicked() {
                self.log.clear();
            }
        });
        egui::ScrollArea::vertical()
            .stick_to_bottom(true)
            .max_height(220.0)
            .show(ui, |ui| {
                for line in &self.log {
                    ui.monospace(line);
                }
            });
    }
}

impl eframe::App for SteamPressApp {
    fn update(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        if self.settings.dark_mode {
            context.set_visuals(egui::Visuals::dark());
        } else {
            context.set_visuals(egui::Visuals::light());
        }

        if context.input(|input| input.viewport().close_requested())
            && self.running
            && !self.allow_close
        {
            context.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.close_dialog = true;
        }

        self.poll();

        self.top_bar(context);
        self.bottom_bar(context);

        let mut action = None;
        egui::CentralPanel::default().show(context, |ui| {
            action = match self.tab {
                Tab::Game => self.game_tab(ui),
                Tab::Workshop => self.workshop_tab(ui),
                Tab::Depots => self.depots_tab(ui),
                Tab::Settings => self.settings_tab(ui),
            };
        });

        if let Some(action) = action {
            self.apply_action(action, context);
        }

        if self.close_dialog {
            let mut quit = false;
            let mut stay = false;
            egui::Window::new("Quitter Steam Press ?")
                .collapsible(false)
                .resizable(false)
                .show(context, |ui| {
                    ui.label("Un téléchargement est en cours. Il sera annulé.");
                    ui.horizontal(|ui| {
                        if ui.button("Annuler et quitter").clicked() {
                            quit = true;
                        }
                        if ui.button("Continuer").clicked() {
                            stay = true;
                        }
                    });
                });
            if quit {
                self.cancel();
                self.allow_close = true;
                context.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            if stay {
                self.close_dialog = false;
            }
        }

        if self.running {
            context.request_repaint_after(Duration::from_millis(120));
        }
    }
}

fn path_text(path: Option<&PathBuf>) -> String {
    path.map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn path_or_none(text: &str) -> Option<PathBuf> {
    let text = text.trim();
    if text.is_empty() {
        None
    } else {
        Some(PathBuf::from(text))
    }
}

fn workshop_stem(job: &WorkshopJob) -> String {
    format!("Workshop_{}_{}", job.appid.trim(), job.workshop_id.trim())
}

fn status_color(status: &EntryStatus) -> egui::Color32 {
    match status {
        EntryStatus::Ready => egui::Color32::GRAY,
        EntryStatus::Running => egui::Color32::from_rgb(90, 150, 240),
        EntryStatus::Done => egui::Color32::from_rgb(110, 200, 120),
        EntryStatus::Skipped => egui::Color32::from_rgb(200, 180, 90),
        EntryStatus::Failed(_) => egui::Color32::from_rgb(235, 100, 100),
        EntryStatus::Cancelled => egui::Color32::from_rgb(200, 140, 90),
    }
}

fn progress_for(stage: Stage) -> f32 {
    match stage {
        Stage::Preflight => 0.05,
        Stage::Download => 0.2,
        Stage::Normalize => 0.45,
        Stage::Manifest => 0.55,
        Stage::Archive => 0.7,
        Stage::Template => 0.9,
        Stage::Done => 1.0,
    }
}
