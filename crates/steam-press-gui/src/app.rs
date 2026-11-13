//! Modern egui desktop interface for Steam Press.

use crate::state::{EntryStatus, GameQueue, LogEntry, LogLevel, WorkerEvent, WorkshopQueue};
use eframe::egui;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use steam_press_core::compressor::Compressor;
use steam_press_core::config::{Paths, Settings};
use steam_press_core::credentials::{CredentialStore, SavedCredentials};
use steam_press_core::depots::DepotNames;
use steam_press_core::downloader::{Credentials, DownloaderKind};
use steam_press_core::events::{EventSink, FinishStatus, PackEvent, Stage};
use steam_press_core::pack::{PackOptions, Packer};
use steam_press_core::platform::Platform;
use steam_press_core::queue::{GameJob, WorkshopJob};
use steam_press_core::tooling;
use steam_press_core::workshop::WorkshopPacker;

type CancelHook = Box<dyn Fn() + Send + Sync>;
type SharedHook = Arc<Mutex<Option<CancelHook>>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Library,
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

#[derive(Debug, Clone, Copy)]
enum RowAction {
    Up(usize),
    Down(usize),
    Remove(usize),
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

#[derive(Clone, Copy)]
struct Palette {
    dark: bool,
    bg: egui::Color32,
    panel: egui::Color32,
    card: egui::Color32,
    input: egui::Color32,
    hover: egui::Color32,
    border: egui::Color32,
    text: egui::Color32,
    muted: egui::Color32,
    accent: egui::Color32,
    accent_text: egui::Color32,
    success: egui::Color32,
    warning: egui::Color32,
    danger: egui::Color32,
}

impl Palette {
    fn for_mode(dark: bool) -> Self {
        let rgb = egui::Color32::from_rgb;
        if dark {
            Self {
                dark,
                bg: rgb(13, 16, 22),
                panel: rgb(18, 22, 30),
                card: rgb(23, 28, 38),
                input: rgb(29, 35, 46),
                hover: rgb(37, 44, 58),
                border: rgb(47, 56, 73),
                text: rgb(232, 237, 245),
                muted: rgb(145, 156, 176),
                accent: rgb(94, 234, 212),
                accent_text: rgb(4, 40, 35),
                success: rgb(74, 222, 128),
                warning: rgb(250, 204, 21),
                danger: rgb(248, 113, 113),
            }
        } else {
            Self {
                dark,
                bg: rgb(246, 247, 251),
                panel: rgb(255, 255, 255),
                card: rgb(255, 255, 255),
                input: rgb(247, 248, 252),
                hover: rgb(238, 242, 248),
                border: rgb(221, 226, 236),
                text: rgb(23, 29, 41),
                muted: rgb(100, 112, 133),
                accent: rgb(15, 118, 110),
                accent_text: rgb(255, 255, 255),
                success: rgb(21, 128, 61),
                warning: rgb(180, 83, 9),
                danger: rgb(185, 28, 28),
            }
        }
    }

    fn soft(&self, color: egui::Color32, alpha: u8) -> egui::Color32 {
        egui::Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
    }

    fn status_colors(&self, status: &EntryStatus) -> (egui::Color32, egui::Color32) {
        match status {
            EntryStatus::Ready => (self.input, self.muted),
            EntryStatus::Running => (self.soft(self.accent, 32), self.accent),
            EntryStatus::Done => (self.soft(self.success, 30), self.success),
            EntryStatus::Skipped => (self.soft(self.warning, 30), self.warning),
            EntryStatus::Failed(_) => (self.soft(self.danger, 30), self.danger),
            EntryStatus::Cancelled => (self.soft(self.warning, 24), self.warning),
        }
    }

    fn level_color(&self, level: LogLevel) -> egui::Color32 {
        match level {
            LogLevel::Info => self.muted,
            LogLevel::Step => self.accent,
            LogLevel::Success => self.success,
            LogLevel::Warning => self.warning,
            LogLevel::Error => self.danger,
        }
    }
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
    applied_dark: Option<bool>,
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
    log: Vec<LogEntry>,
    activity_open: bool,
    status_message: Option<String>,
    error: Option<String>,
    running: bool,
    cancel_requested: Arc<AtomicBool>,
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
        Self::assemble(paths, settings, depot_names, login)
    }

    fn assemble(
        paths: Paths,
        settings: Settings,
        depot_names: DepotNames,
        login: LoginForm,
    ) -> Self {
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
        let custom_options_text = settings.custom_compress_options.clone().unwrap_or_default();

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
            tab: Tab::Library,
            game_queue: GameQueue::default(),
            workshop_queue: WorkshopQueue::default(),
            game_form: GameForm::default(),
            workshop_form: WorkshopForm::default(),
            login,
            applied_dark: None,
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
            stage: "Idle".to_string(),
            progress: 0.0,
            log: Vec::new(),
            activity_open: true,
            status_message: None,
            error: None,
            running: false,
            cancel_requested: Arc::new(AtomicBool::new(false)),
            allow_close: false,
            close_dialog: false,
            receiver: None,
            cancel_hook: None,
        }
    }

    #[cfg(test)]
    fn for_tests(root: &std::path::Path) -> Self {
        let paths = Paths::under(root);
        let _ = paths.ensure();
        let mut settings = Settings {
            dark_mode: true,
            ..Default::default()
        };
        settings.ensure_key();
        Self::assemble(paths, settings, DepotNames::default(), LoginForm::default())
    }

    fn output_dir(&self) -> PathBuf {
        self.settings.output_dir(&self.paths)
    }

    fn push_log(&mut self, level: LogLevel, text: impl Into<String>) {
        self.log.push(LogEntry::new(level, text));
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
            None => Err("No account configured. Enter one below or use \"anonymous\".".to_string()),
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
        let job_count = jobs.len();
        if jobs.is_empty() {
            self.error = Some("The game queue is empty.".to_string());
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
        let cancel_requested = self.cancel_requested.clone();

        std::thread::spawn(move || {
            for job in jobs {
                if cancel_requested.load(Ordering::SeqCst) {
                    break;
                }
                let packer = Arc::new(Packer::new(options.clone()));
                if let Ok(mut slot) = thread_hook.lock() {
                    let packer = packer.clone();
                    *slot = Some(Box::new(move || packer.cancel()));
                }
                let _ = packer.run(&job, sink.clone());
                if cancel_requested.load(Ordering::SeqCst) {
                    break;
                }
            }
            if let Ok(mut slot) = thread_hook.lock() {
                *slot = None;
            }
            let _ = sender.send(WorkerEvent::Done);
        });

        self.begin_run(receiver, hook, job_count);
        context.request_repaint();
    }

    fn start_workshop(&mut self, context: &egui::Context) {
        if self.running {
            return;
        }
        let jobs: Vec<WorkshopJob> = self.workshop_queue.jobs();
        let job_count = jobs.len();
        if jobs.is_empty() {
            self.error = Some("The workshop queue is empty.".to_string());
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
        let cancel_requested = self.cancel_requested.clone();

        std::thread::spawn(move || {
            for job in jobs {
                if cancel_requested.load(Ordering::SeqCst) {
                    break;
                }
                let packer = Arc::new(WorkshopPacker::new(options.clone()));
                if let Ok(mut slot) = thread_hook.lock() {
                    let packer = packer.clone();
                    *slot = Some(Box::new(move || packer.cancel()));
                }
                let _ = packer.run(&job, sink.clone());
                if cancel_requested.load(Ordering::SeqCst) {
                    break;
                }
            }
            if let Ok(mut slot) = thread_hook.lock() {
                *slot = None;
            }
            let _ = sender.send(WorkerEvent::Done);
        });

        self.begin_run(receiver, hook, job_count);
        context.request_repaint();
    }

    fn begin_run(&mut self, receiver: Receiver<WorkerEvent>, hook: SharedHook, count: usize) {
        self.error = None;
        self.status_message = None;
        self.progress = 0.0;
        self.stage = "Starting".to_string();
        self.log.clear();
        self.activity_open = true;
        self.running = true;
        self.cancel_requested.store(false, Ordering::SeqCst);
        self.game_queue.reset_statuses();
        self.workshop_queue.reset_statuses();
        self.receiver = Some(receiver);
        self.cancel_hook = Some(hook);
        self.push_log(
            LogLevel::Info,
            format!("Starting queue with {count} job(s)."),
        );
    }

    fn cancel(&mut self) {
        self.cancel_requested.store(true, Ordering::SeqCst);
        if let Some(hook) = &self.cancel_hook {
            if let Ok(slot) = hook.lock() {
                if let Some(cancel) = slot.as_ref() {
                    cancel();
                }
            }
        }
        self.stage = "Cancelling".to_string();
        self.push_log(
            LogLevel::Warning,
            "Cancellation requested; stopping the queue.",
        );
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
            if self.cancel_requested.load(Ordering::SeqCst) {
                self.game_queue.cancel_unfinished();
                self.workshop_queue.cancel_unfinished();
                self.stage = "Cancelled".to_string();
                self.push_log(LogLevel::Warning, "Queue cancelled.");
            } else {
                self.stage = "Finished".to_string();
                self.progress = 1.0;
                self.push_log(LogLevel::Success, "Queue finished.");
            }
        }
    }

    fn handle_event(&mut self, event: PackEvent) {
        match event {
            PackEvent::Stage {
                job,
                stage,
                message,
            } => {
                self.stage = message.clone();
                self.progress = progress_for(stage);
                if matches!(stage, Stage::Preflight | Stage::Download) {
                    self.game_queue
                        .set_status(|entry| entry.encode() == job, EntryStatus::Running);
                    self.workshop_queue
                        .set_status(|entry| workshop_stem(entry) == job, EntryStatus::Running);
                }
                self.push_log(LogLevel::Step, format!("{} - {message}", stage.label()));
            }
            PackEvent::Progress {
                message, percent, ..
            } => {
                self.stage = message;
                if let Some(percent) = percent {
                    self.progress = percent;
                }
            }
            PackEvent::Log(line) => self.push_log(LogLevel::Info, line),
            PackEvent::Finished {
                job,
                status,
                outcome,
                output,
            } => {
                let entry_status = match status {
                    FinishStatus::Completed => EntryStatus::Done,
                    FinishStatus::Skipped => EntryStatus::Skipped,
                    FinishStatus::Cancelled => EntryStatus::Cancelled,
                    FinishStatus::Failed => EntryStatus::Failed(outcome.clone()),
                };
                self.game_queue
                    .set_status(|entry| entry.encode() == job, entry_status.clone());
                self.workshop_queue
                    .set_status(|entry| workshop_stem(entry) == job, entry_status);
                let level = match status {
                    FinishStatus::Completed => LogLevel::Success,
                    FinishStatus::Skipped => LogLevel::Warning,
                    FinishStatus::Cancelled => LogLevel::Warning,
                    FinishStatus::Failed => LogLevel::Error,
                };
                let text = match (status, output) {
                    (FinishStatus::Failed, _) => format!("Failed: {outcome}"),
                    (_, Some(path)) => format!("{} - {}", status.label(), path.display()),
                    (_, None) => format!("{} - {outcome}", status.label()),
                };
                self.push_log(level, text);
                if status == FinishStatus::Failed {
                    self.error = Some(outcome);
                }
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
        self.settings.custom_compress_options = if self.custom_options_text.trim().is_empty() {
            None
        } else {
            Some(self.custom_options_text.trim().to_string())
        };
        self.settings.ensure_key();
        match self.settings.save(&self.paths) {
            Ok(()) => self.status_message = Some("Settings saved.".to_string()),
            Err(error) => self.error = Some(format!("Settings: {error}")),
        }
    }

    fn save_login(&mut self) {
        let username = self.login.username.trim().to_string();
        if username.is_empty() {
            self.error = Some("Username is empty.".to_string());
            return;
        }
        self.settings.ensure_key();
        if let Err(error) = self.settings.save(&self.paths) {
            self.error = Some(format!("Settings: {error}"));
            return;
        }
        if username.eq_ignore_ascii_case("anonymous") {
            let _ = CredentialStore::new(&self.paths).clear();
            self.login.password.clear();
            self.status_message = Some("Anonymous mode: credentials removed.".to_string());
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
            Ok(()) => self.status_message = Some("Credentials saved.".to_string()),
            Err(error) => self.error = Some(error),
        }
    }

    fn delete_login(&mut self) {
        match CredentialStore::new(&self.paths).clear() {
            Ok(()) => {
                self.login.username.clear();
                self.login.password.clear();
                self.status_message = Some("Credentials removed.".to_string());
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
                    self.depot_notice = Some(format!("{count} depot names synchronized."));
                }
                Err(error) => self.depot_notice = Some(format!("Write failed: {error}")),
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
                self.status_message = Some(format!("{added} job(s) imported."));
                for warning in load.warnings {
                    self.push_log(LogLevel::Warning, format!("warning: {warning}"));
                }
            }
            Err(error) => self.error = Some(error),
        }
    }

    fn export_games(&mut self) {
        let jobs = self.game_queue.jobs();
        match steam_press_core::queue::save_game_queue(&self.queue_io_path, &jobs) {
            Ok(()) => self.status_message = Some("Queue exported.".to_string()),
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
                self.status_message = Some(format!("{added} item(s) imported."));
            }
            Err(error) => self.error = Some(error),
        }
    }

    fn export_workshop(&mut self) {
        let jobs = self.workshop_queue.jobs();
        match steam_press_core::queue::save_workshop_queue(&self.workshop_io_path, &jobs) {
            Ok(()) => self.status_message = Some("Queue exported.".to_string()),
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

    fn apply_style(&self, context: &egui::Context, palette: &Palette) {
        let mut style = (*context.style()).clone();
        style.spacing.item_spacing = egui::vec2(10.0, 9.0);
        style.spacing.button_padding = egui::vec2(12.0, 7.0);
        style.spacing.interact_size.y = 28.0;
        style.visuals = if palette.dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
        let visuals = &mut style.visuals;
        visuals.panel_fill = palette.bg;
        visuals.window_fill = palette.panel;
        visuals.extreme_bg_color = palette.input;
        visuals.faint_bg_color = palette.card;
        visuals.override_text_color = Some(palette.text);
        visuals.hyperlink_color = palette.accent;
        visuals.window_stroke = egui::Stroke::new(1.0_f32, palette.border);
        visuals.selection.bg_fill = palette.hover;
        visuals.selection.stroke = egui::Stroke::new(1.0_f32, palette.accent);
        visuals.widgets.noninteractive.bg_fill = palette.panel;
        visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0_f32, palette.border);
        visuals.widgets.inactive.bg_fill = palette.input;
        visuals.widgets.inactive.weak_bg_fill = palette.input;
        visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0_f32, palette.border);
        visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0_f32, palette.text);
        visuals.widgets.hovered.bg_fill = palette.hover;
        visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0_f32, palette.accent);
        visuals.widgets.active.bg_fill = palette.card;
        visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0_f32, palette.accent);
        for widget in [
            &mut visuals.widgets.noninteractive,
            &mut visuals.widgets.inactive,
            &mut visuals.widgets.hovered,
            &mut visuals.widgets.active,
            &mut visuals.widgets.open,
        ] {
            widget.corner_radius = egui::CornerRadius::same(8);
        }
        context.set_style(style);
    }

    fn sidebar(&mut self, context: &egui::Context, palette: &Palette) {
        egui::SidePanel::left("sidebar")
            .exact_width(236.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(palette.panel)
                    .inner_margin(egui::Margin::symmetric(16, 20)),
            )
            .show(context, |ui| {
                ui.horizontal(|ui| {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(34.0, 34.0), egui::Sense::hover());
                    ui.painter()
                        .rect_filled(rect, egui::CornerRadius::same(9), palette.accent);
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "SP",
                        egui::FontId::proportional(12.0),
                        palette.accent_text,
                    );
                    ui.add_space(6.0);
                    ui.vertical(|ui| {
                        ui.label(
                            egui::RichText::new("Steam Press")
                                .size(16.0)
                                .strong()
                                .color(palette.text),
                        );
                        ui.label(
                            egui::RichText::new("Release builder")
                                .size(10.5)
                                .color(palette.muted),
                        );
                    });
                });

                ui.add_space(26.0);
                ui.label(
                    egui::RichText::new("WORKSPACE")
                        .size(9.5)
                        .strong()
                        .color(palette.muted),
                );
                ui.add_space(6.0);

                let items = [
                    (Tab::Library, "G", "Library", "Games and branches"),
                    (Tab::Workshop, "W", "Workshop", "Community items"),
                    (Tab::Depots, "D", "Depots", "Steam metadata"),
                    (Tab::Settings, "S", "Settings", "Tools and paths"),
                ];
                for (tab, icon, label, hint) in items {
                    if nav_item(ui, palette, icon, label, hint, self.tab == tab).clicked() {
                        self.tab = tab;
                    }
                    ui.add_space(3.0);
                }

                ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                    ui.label(
                        egui::RichText::new(format!("v{}", env!("CARGO_PKG_VERSION")))
                            .size(10.0)
                            .color(palette.muted),
                    );
                    ui.add_space(8.0);
                    egui::Frame::new()
                        .fill(palette.card)
                        .stroke(egui::Stroke::new(1.0_f32, palette.border))
                        .corner_radius(egui::CornerRadius::same(11))
                        .inner_margin(egui::Margin::same(12))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(
                                egui::RichText::new("ENGINE")
                                    .size(9.0)
                                    .strong()
                                    .color(palette.muted),
                            );
                            ui.label(
                                egui::RichText::new(self.downloader_kind.label())
                                    .size(12.5)
                                    .color(palette.text),
                            );
                            ui.add_space(6.0);
                            ui.horizontal(|ui| {
                                if ui
                                    .small_button(if self.settings.dark_mode {
                                        "Light theme"
                                    } else {
                                        "Dark theme"
                                    })
                                    .clicked()
                                {
                                    self.settings.dark_mode = !self.settings.dark_mode;
                                }
                            });
                        });
                });
            });
    }

    fn feedback(&mut self, ui: &mut egui::Ui, palette: &Palette) {
        let message = self
            .error
            .clone()
            .map(|text| (text, palette.danger))
            .or_else(|| {
                self.status_message
                    .clone()
                    .map(|text| (text, palette.success))
            });
        let Some((text, color)) = message else {
            return;
        };
        let mut dismiss = false;
        egui::Frame::new()
            .fill(palette.soft(color, 24))
            .stroke(egui::Stroke::new(1.0_f32, palette.soft(color, 90)))
            .corner_radius(egui::CornerRadius::same(9))
            .inner_margin(egui::Margin::symmetric(12, 8))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(text).size(12.0).color(color));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("Dismiss").clicked() {
                            dismiss = true;
                        }
                    });
                });
            });
        if dismiss {
            self.error = None;
            self.status_message = None;
        }
        ui.add_space(14.0);
    }

    fn activity_panel(&mut self, context: &egui::Context, palette: &Palette) {
        egui::TopBottomPanel::bottom("activity")
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(palette.panel)
                    .stroke(egui::Stroke::new(1.0_f32, palette.border))
                    .inner_margin(egui::Margin::symmetric(24, 9)),
            )
            .show(context, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("Activity")
                            .size(12.5)
                            .strong()
                            .color(palette.text),
                    );
                    let status_color = if self.running {
                        palette.accent
                    } else {
                        palette.muted
                    };
                    ui.label(
                        egui::RichText::new(if self.running { "● Live" } else { "● Idle" })
                            .size(10.5)
                            .color(status_color),
                    );
                    ui.label(
                        egui::RichText::new(&self.stage)
                            .size(10.5)
                            .color(palette.muted),
                    );
                    ui.with_layout(
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            if ui
                                .small_button(if self.activity_open { "Hide" } else { "Show" })
                                .clicked()
                            {
                                self.activity_open = !self.activity_open;
                            }
                            if self.activity_open && ui.small_button("Clear").clicked() {
                                self.log.clear();
                            }
                            if ui.small_button("Open output").clicked() {
                                let output = self.output_dir();
                                let _ = std::fs::create_dir_all(&output);
                                let _ = tooling::open_folder(&output);
                            }
                            if self.running {
                                if ui.small_button("Cancel").clicked() {
                                    self.cancel();
                                }
                                ui.spinner();
                            }
                        },
                    );
                });

                if self.activity_open {
                    ui.add_space(5.0);
                    ui.add(
                        egui::ProgressBar::new(self.progress)
                            .fill(palette.accent)
                            .desired_width(f32::INFINITY),
                    );
                    ui.add_space(4.0);
                    if self.log.is_empty() {
                        ui.label(
                            egui::RichText::new(
                                "No activity yet. Start a queue to stream download and archive output here.",
                            )
                            .size(11.0)
                            .color(palette.muted),
                        );
                    }
                    egui::ScrollArea::vertical()
                        .stick_to_bottom(true)
                        .max_height(168.0)
                        .show_rows(ui, 18.0, self.log.len(), |ui, range| {
                            for entry in &self.log[range] {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        egui::RichText::new(entry.level.tag())
                                            .monospace()
                                            .size(9.0)
                                            .color(palette.level_color(entry.level)),
                                    );
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(&entry.text)
                                                .monospace()
                                                .size(11.0)
                                                .color(palette.text),
                                        )
                                        .truncate(),
                                    );
                                });
                            }
                        });
                }
            });
    }

    fn library_page(&mut self, ui: &mut egui::Ui, palette: &Palette) -> Option<UiAction> {
        let mut action = None;
        let completed = self
            .game_queue
            .count(|entry| entry.status == EntryStatus::Done);
        let failed = self
            .game_queue
            .count(|entry| matches!(entry.status, EntryStatus::Failed(_)));

        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(page_title(palette, "Library"));
                ui.label(
                    egui::RichText::new("Download, sanitize and pack Steam games.")
                        .size(12.5)
                        .color(palette.muted),
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.running {
                    if primary_button(ui, palette, "Cancel", true).clicked() {
                        action = Some(UiAction::Cancel);
                    }
                } else if primary_button(ui, palette, "Start queue", !self.game_queue.is_empty())
                    .clicked()
                {
                    action = Some(UiAction::StartGames);
                }
            });
        });
        ui.add_space(18.0);

        ui.columns(3, |columns| {
            stat_card(
                &mut columns[0],
                palette,
                "Queued",
                self.game_queue.len(),
                palette.text,
            );
            stat_card(
                &mut columns[1],
                palette,
                "Completed",
                completed,
                palette.success,
            );
            stat_card(&mut columns[2], palette, "Failed", failed, palette.danger);
        });
        ui.add_space(14.0);

        card(ui, palette, |ui| {
            section_title(
                ui,
                palette,
                "Add a game",
                "Steam App ID, branch and target platform.",
            );
            ui.add_space(10.0);
            ui.horizontal_wrapped(|ui| {
                field(ui, palette, "App ID", 96.0, |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.game_form.appid)
                            .hint_text("2379780")
                            .desired_width(96.0),
                    );
                });
                field(ui, palette, "Platform", 126.0, |ui| {
                    egui::ComboBox::from_id_salt("game-platform")
                        .selected_text(self.game_form.platform.label())
                        .width(126.0)
                        .show_ui(ui, |ui| {
                            for platform in Platform::all() {
                                ui.selectable_value(
                                    &mut self.game_form.platform,
                                    *platform,
                                    platform.label(),
                                );
                            }
                        });
                });
                field(ui, palette, "Branch", 116.0, |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.game_form.branch).desired_width(116.0),
                    );
                });
                field(ui, palette, "Branch password", 130.0, |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.game_form.branch_password)
                            .password(true)
                            .hint_text("optional")
                            .desired_width(130.0),
                    );
                });
                ui.add_space(2.0);
                if primary_button(ui, palette, "Add game", true).clicked() {
                    let appid = self.game_form.appid.trim();
                    if appid.parse::<u64>().map(|value| value > 0).unwrap_or(false) {
                        let job = GameJob::new(self.game_form.platform, appid).with_branch(
                            self.game_form.branch.clone(),
                            self.game_form.branch_password.clone(),
                        );
                        if self.game_queue.push(job) {
                            self.game_form.appid.clear();
                            self.game_form.branch_password.clear();
                            self.error = None;
                            self.status_message = Some("Game added to the queue.".to_string());
                        } else {
                            self.error = Some("That job is already queued.".to_string());
                        }
                    } else {
                        self.error = Some("Enter a valid numeric App ID.".to_string());
                    }
                }
            });
        });

        ui.add_space(14.0);
        card(ui, palette, |ui| {
            ui.horizontal(|ui| {
                section_title(ui, palette, "Queue", "Jobs run from top to bottom.");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add_enabled(
                            !self.running && !self.game_queue.is_empty(),
                            egui::Button::new(egui::RichText::new("Clear queue").size(11.0)),
                        )
                        .clicked()
                    {
                        self.game_queue.clear();
                    }
                    ui.label(
                        egui::RichText::new(format!("{} jobs", self.game_queue.len()))
                            .size(10.5)
                            .color(palette.muted),
                    );
                });
            });
            ui.add_space(10.0);
            if self.game_queue.is_empty() {
                empty_state(
                    ui,
                    palette,
                    "No games queued",
                    "Add an App ID above or import a .SSPQ queue file.",
                );
            } else {
                let mut row_action = None;
                egui::ScrollArea::vertical()
                    .max_height(300.0)
                    .show(ui, |ui| {
                        for (index, entry) in self.game_queue.entries().iter().enumerate() {
                            if let Some(action) = game_row(ui, palette, index, entry, self.running)
                            {
                                row_action = Some(action);
                            }
                            ui.add_space(5.0);
                        }
                    });
                if let Some(action) = row_action {
                    match action {
                        RowAction::Up(index) => self.game_queue.move_up(index),
                        RowAction::Down(index) => self.game_queue.move_down(index),
                        RowAction::Remove(index) => {
                            self.game_queue.remove(index);
                        }
                    }
                }
            }
            ui.add_space(10.0);
            ui.separator();
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    egui::RichText::new("Queue file")
                        .size(11.0)
                        .color(palette.muted),
                );
                ui.add(egui::TextEdit::singleline(&mut self.queue_io_path).desired_width(250.0));
                if secondary_button(ui, palette, "Import").clicked() {
                    action = Some(UiAction::ImportGames);
                }
                if secondary_button(ui, palette, "Export").clicked() {
                    action = Some(UiAction::ExportGames);
                }
            });
        });

        action
    }

    fn workshop_page(&mut self, ui: &mut egui::Ui, palette: &Palette) -> Option<UiAction> {
        let mut action = None;
        let completed = self
            .workshop_queue
            .count(|entry| entry.status == EntryStatus::Done);
        let failed = self
            .workshop_queue
            .count(|entry| matches!(entry.status, EntryStatus::Failed(_)));

        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(page_title(palette, "Workshop"));
                ui.label(
                    egui::RichText::new("Archive community items into clean 7-Zip packages.")
                        .size(12.5)
                        .color(palette.muted),
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.running {
                    if primary_button(ui, palette, "Cancel", true).clicked() {
                        action = Some(UiAction::Cancel);
                    }
                } else if primary_button(
                    ui,
                    palette,
                    "Start queue",
                    !self.workshop_queue.is_empty(),
                )
                .clicked()
                {
                    action = Some(UiAction::StartWorkshop);
                }
            });
        });
        ui.add_space(18.0);

        ui.columns(3, |columns| {
            stat_card(
                &mut columns[0],
                palette,
                "Queued",
                self.workshop_queue.len(),
                palette.text,
            );
            stat_card(
                &mut columns[1],
                palette,
                "Completed",
                completed,
                palette.success,
            );
            stat_card(&mut columns[2], palette, "Failed", failed, palette.danger);
        });
        ui.add_space(14.0);

        card(ui, palette, |ui| {
            section_title(
                ui,
                palette,
                "Add a workshop item",
                "The owning App ID and the published file ID.",
            );
            ui.add_space(10.0);
            ui.horizontal_wrapped(|ui| {
                field(ui, palette, "App ID", 110.0, |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.workshop_form.appid)
                            .hint_text("730")
                            .desired_width(110.0),
                    );
                });
                field(ui, palette, "Workshop ID", 150.0, |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.workshop_form.workshop_id)
                            .hint_text("123456789")
                            .desired_width(150.0),
                    );
                });
                ui.add_space(2.0);
                if primary_button(ui, palette, "Add item", true).clicked() {
                    let job = WorkshopJob::new(
                        self.workshop_form.appid.trim(),
                        self.workshop_form.workshop_id.trim(),
                    );
                    match job.validate() {
                        Ok(()) if self.workshop_queue.push(job) => {
                            self.workshop_form = WorkshopForm::default();
                            self.error = None;
                            self.status_message =
                                Some("Workshop item added to the queue.".to_string());
                        }
                        Ok(()) => {
                            self.error = Some("That item is already queued.".to_string());
                        }
                        Err(error) => self.error = Some(error),
                    }
                }
            });
        });

        ui.add_space(14.0);
        card(ui, palette, |ui| {
            ui.horizontal(|ui| {
                section_title(ui, palette, "Queue", "Workshop jobs run sequentially.");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add_enabled(
                            !self.running && !self.workshop_queue.is_empty(),
                            egui::Button::new(egui::RichText::new("Clear queue").size(11.0)),
                        )
                        .clicked()
                    {
                        self.workshop_queue.clear();
                    }
                    ui.label(
                        egui::RichText::new(format!("{} items", self.workshop_queue.len()))
                            .size(10.5)
                            .color(palette.muted),
                    );
                });
            });
            ui.add_space(10.0);
            if self.workshop_queue.is_empty() {
                empty_state(
                    ui,
                    palette,
                    "No workshop items queued",
                    "Add an App ID and workshop ID, or import a .SSWQ file.",
                );
            } else {
                let mut row_action = None;
                egui::ScrollArea::vertical()
                    .max_height(300.0)
                    .show(ui, |ui| {
                        for (index, entry) in self.workshop_queue.entries().iter().enumerate() {
                            if let Some(action) =
                                workshop_row(ui, palette, index, entry, self.running)
                            {
                                row_action = Some(action);
                            }
                            ui.add_space(5.0);
                        }
                    });
                if let Some(action) = row_action {
                    match action {
                        RowAction::Up(index) => self.workshop_queue.move_up(index),
                        RowAction::Down(index) => self.workshop_queue.move_down(index),
                        RowAction::Remove(index) => {
                            self.workshop_queue.remove(index);
                        }
                    }
                }
            }
            ui.add_space(10.0);
            ui.separator();
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    egui::RichText::new("Queue file")
                        .size(11.0)
                        .color(palette.muted),
                );
                ui.add(egui::TextEdit::singleline(&mut self.workshop_io_path).desired_width(250.0));
                if secondary_button(ui, palette, "Import").clicked() {
                    action = Some(UiAction::ImportWorkshop);
                }
                if secondary_button(ui, palette, "Export").clicked() {
                    action = Some(UiAction::ExportWorkshop);
                }
            });
        });

        action
    }

    fn depots_page(&mut self, ui: &mut egui::Ui, palette: &Palette) -> Option<UiAction> {
        let mut action = None;
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(page_title(palette, "Depots"));
                ui.label(
                    egui::RichText::new("Map depot IDs to readable names for release notes.")
                        .size(12.5)
                        .color(palette.muted),
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if primary_button(ui, palette, "Sync names", true).clicked() {
                    action = Some(UiAction::SyncDepots);
                }
            });
        });
        ui.add_space(18.0);

        card(ui, palette, |ui| {
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.depot_search)
                        .hint_text("Search by depot ID or name")
                        .desired_width(300.0),
                );
                ui.label(
                    egui::RichText::new(format!("{} known names", self.depot_names.len()))
                        .size(10.5)
                        .color(palette.muted),
                );
                if let Some(notice) = &self.depot_notice {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(notice)
                                .size(10.5)
                                .color(palette.success),
                        );
                    });
                }
            });
        });

        ui.add_space(14.0);
        card(ui, palette, |ui| {
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

            if matches.is_empty() {
                empty_state(
                    ui,
                    palette,
                    "No matching depots",
                    "Try another ID or synchronize the depot names.",
                );
            } else {
                egui::ScrollArea::vertical().max_height(460.0).show_rows(
                    ui,
                    25.0,
                    matches.len(),
                    |ui, range| {
                        for (id, name) in &matches[range] {
                            ui.horizontal(|ui| {
                                ui.add_sized(
                                    [76.0, 20.0],
                                    egui::Label::new(
                                        egui::RichText::new(id.to_string())
                                            .monospace()
                                            .size(11.0)
                                            .color(palette.accent),
                                    ),
                                );
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(name).size(12.0).color(palette.text),
                                    )
                                    .truncate(),
                                );
                            });
                        }
                    },
                );
            }
        });

        action
    }

    fn settings_page(&mut self, ui: &mut egui::Ui, palette: &Palette) -> Option<UiAction> {
        let mut action = None;
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(page_title(palette, "Settings"));
                ui.label(
                    egui::RichText::new("Download engine, archive tools and release metadata.")
                        .size(12.5)
                        .color(palette.muted),
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if primary_button(ui, palette, "Save settings", true).clicked() {
                    action = Some(UiAction::SaveSettings);
                }
            });
        });
        ui.add_space(18.0);

        ui.columns(2, |columns| {
            card(&mut columns[0], palette, |ui| {
                section_title(
                    ui,
                    palette,
                    "Download and compression",
                    "Choose the engine and archive defaults.",
                );
                ui.add_space(12.0);
                ui.label(field_label(palette, "Download engine"));
                egui::ComboBox::from_id_salt("downloader")
                    .selected_text(self.downloader_kind.label())
                    .width(220.0)
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
                ui.add_space(10.0);
                ui.label(field_label(palette, "Archiver"));
                egui::ComboBox::from_id_salt("compressor")
                    .selected_text(self.settings.compressor.label())
                    .width(220.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.settings.compressor,
                            Compressor::SevenZip,
                            "7-Zip",
                        );
                        ui.selectable_value(&mut self.settings.compressor, Compressor::Rar, "RAR");
                    });
                ui.add_space(10.0);
                ui.label(field_label(palette, "Custom archive options"));
                ui.add(
                    egui::TextEdit::singleline(&mut self.custom_options_text)
                        .hint_text("-mx9 -sdel -mhe=on -v5g")
                        .desired_width(f32::INFINITY),
                );
                ui.add_space(10.0);
                ui.label(field_label(palette, "Archive password"));
                ui.add(
                    egui::TextEdit::singleline(&mut self.settings.compress_password)
                        .desired_width(f32::INFINITY),
                );
                ui.add_space(8.0);
                ui.checkbox(
                    &mut self.settings.skip_compression,
                    "Keep the prepared folder (skip archiving)",
                );
            });

            card(&mut columns[1], palette, |ui| {
                section_title(
                    ui,
                    palette,
                    "Paths",
                    "Leave a field empty to use PATH discovery.",
                );
                ui.add_space(12.0);
                ui.label(field_label(palette, "Output directory"));
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.output_dir_text).desired_width(300.0),
                    );
                    if secondary_button(ui, palette, "Open").clicked() {
                        action = Some(UiAction::OpenOutput);
                    }
                });
                ui.add_space(10.0);
                ui.label(field_label(palette, "SteamCMD"));
                ui.add(
                    egui::TextEdit::singleline(&mut self.steamcmd_text)
                        .hint_text("steamcmd.sh in PATH")
                        .desired_width(f32::INFINITY),
                );
                ui.add_space(10.0);
                ui.label(field_label(palette, "DepotDownloader"));
                ui.add(
                    egui::TextEdit::singleline(&mut self.depotdownloader_text)
                        .desired_width(f32::INFINITY),
                );
                ui.add_space(10.0);
                ui.label(field_label(palette, "7-Zip"));
                ui.add(
                    egui::TextEdit::singleline(&mut self.sevenzip_text)
                        .hint_text("7z in PATH")
                        .desired_width(f32::INFINITY),
                );
                ui.add_space(10.0);
                ui.label(field_label(palette, "RAR"));
                ui.add(egui::TextEdit::singleline(&mut self.rar_text).desired_width(f32::INFINITY));
                ui.add_space(10.0);
                ui.label(field_label(palette, "Release template"));
                ui.add(
                    egui::TextEdit::singleline(&mut self.template_text)
                        .hint_text("Path to a token template")
                        .desired_width(f32::INFINITY),
                );
            });
        });

        ui.add_space(14.0);
        card(ui, palette, |ui| {
            section_title(
                ui,
                palette,
                "Release metadata",
                "Values written into the generated release notes.",
            );
            ui.add_space(12.0);
            ui.columns(2, |columns| {
                field(&mut columns[0], palette, "Uploader", 240.0, |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.settings.uploader)
                            .desired_width(f32::INFINITY),
                    );
                });
                field(&mut columns[1], palette, "File host", 240.0, |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.settings.filehost)
                            .desired_width(f32::INFINITY),
                    );
                });
            });
            ui.add_space(8.0);
            ui.checkbox(
                &mut self.settings.upload_crew_mode,
                "Include Upload Crew metadata in the release notes",
            );
        });

        ui.add_space(14.0);
        card(ui, palette, |ui| {
            section_title(
                ui,
                palette,
                "Steam account",
                "Credentials are encrypted locally with AES-256-GCM.",
            );
            ui.add_space(12.0);
            ui.columns(2, |columns| {
                field(&mut columns[0], palette, "Username", 240.0, |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.login.username)
                            .hint_text("anonymous or a Steam account")
                            .desired_width(f32::INFINITY),
                    );
                });
                field(&mut columns[1], palette, "Password", 240.0, |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.login.password)
                            .password(true)
                            .desired_width(f32::INFINITY),
                    );
                });
            });
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if secondary_button(ui, palette, "Save credentials").clicked() {
                    action = Some(UiAction::SaveLogin);
                }
                if secondary_button(ui, palette, "Remove").clicked() {
                    action = Some(UiAction::DeleteLogin);
                }
            });
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new(
                    "Passwords are never logged and never passed on a command line to DepotDownloader.",
                )
                .size(10.5)
                .color(palette.muted),
            );
        });

        ui.add_space(14.0);
        card(ui, palette, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    section_title(ui, palette, "Appearance", "");
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.checkbox(&mut self.settings.dark_mode, "Dark theme");
                });
            });
        });

        action
    }

    fn draw(&mut self, context: &egui::Context) {
        let palette = Palette::for_mode(self.settings.dark_mode);
        if self.applied_dark != Some(palette.dark) {
            self.apply_style(context, &palette);
            self.applied_dark = Some(palette.dark);
        }

        if context.input(|input| input.viewport().close_requested())
            && self.running
            && !self.allow_close
        {
            context.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.close_dialog = true;
        }

        self.poll();
        self.sidebar(context, &palette);
        self.activity_panel(context, &palette);

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(palette.bg)
                    .inner_margin(egui::Margin::symmetric(28, 24)),
            )
            .show(context, |ui| {
                let mut action = None;
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        self.feedback(ui, &palette);
                        action = match self.tab {
                            Tab::Library => self.library_page(ui, &palette),
                            Tab::Workshop => self.workshop_page(ui, &palette),
                            Tab::Depots => self.depots_page(ui, &palette),
                            Tab::Settings => self.settings_page(ui, &palette),
                        };
                    });
                if let Some(action) = action {
                    self.apply_action(action, context);
                }
            });

        if self.close_dialog {
            let mut quit = false;
            let mut stay = false;
            egui::Window::new("Quit Steam Press?")
                .collapsible(false)
                .resizable(false)
                .show(context, |ui| {
                    ui.label("A download is running. It will be cancelled.");
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if primary_button(ui, &palette, "Cancel and quit", true).clicked() {
                            quit = true;
                        }
                        if secondary_button(ui, &palette, "Keep running").clicked() {
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

impl eframe::App for SteamPressApp {
    fn update(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        self.draw(context);
    }
}

fn nav_item(
    ui: &mut egui::Ui,
    palette: &Palette,
    icon: &str,
    label: &str,
    hint: &str,
    selected: bool,
) -> egui::Response {
    let height = 48.0;
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height),
        egui::Sense::click(),
    );
    if ui.is_rect_visible(rect) {
        let background = if selected {
            palette.card
        } else if response.hovered() {
            palette.hover
        } else {
            egui::Color32::TRANSPARENT
        };
        let painter = ui.painter();
        painter.rect_filled(rect, egui::CornerRadius::same(10), background);
        if selected {
            painter.rect_filled(
                egui::Rect::from_min_size(rect.min + egui::vec2(0.0, 10.0), egui::vec2(3.0, 28.0)),
                egui::CornerRadius::same(2),
                palette.accent,
            );
        }
        let icon_rect =
            egui::Rect::from_min_size(rect.min + egui::vec2(12.0, 12.0), egui::vec2(24.0, 24.0));
        painter.rect_filled(
            icon_rect,
            egui::CornerRadius::same(7),
            if selected {
                palette.accent
            } else {
                palette.input
            },
        );
        painter.text(
            icon_rect.center(),
            egui::Align2::CENTER_CENTER,
            icon,
            egui::FontId::proportional(11.0),
            if selected {
                palette.accent_text
            } else {
                palette.muted
            },
        );
        painter.text(
            rect.min + egui::vec2(46.0, 8.0),
            egui::Align2::LEFT_TOP,
            label,
            egui::FontId::proportional(13.5),
            palette.text,
        );
        painter.text(
            rect.min + egui::vec2(46.0, 26.0),
            egui::Align2::LEFT_TOP,
            hint,
            egui::FontId::proportional(10.0),
            palette.muted,
        );
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn page_title(palette: &Palette, title: &str) -> egui::RichText {
    egui::RichText::new(title)
        .size(29.0)
        .strong()
        .color(palette.text)
}

fn card(ui: &mut egui::Ui, palette: &Palette, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(palette.card)
        .stroke(egui::Stroke::new(1.0_f32, palette.border))
        .corner_radius(egui::CornerRadius::same(14))
        .inner_margin(egui::Margin::same(18))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        });
}

fn section_title(ui: &mut egui::Ui, palette: &Palette, title: &str, caption: &str) {
    ui.label(
        egui::RichText::new(title)
            .size(15.0)
            .strong()
            .color(palette.text),
    );
    if !caption.is_empty() {
        ui.label(egui::RichText::new(caption).size(11.0).color(palette.muted));
    }
}

fn field_label(palette: &Palette, label: &str) -> egui::RichText {
    egui::RichText::new(label).size(10.5).color(palette.muted)
}

fn field(
    ui: &mut egui::Ui,
    palette: &Palette,
    label: &str,
    width: f32,
    add: impl FnOnce(&mut egui::Ui),
) {
    ui.vertical(|ui| {
        ui.label(field_label(palette, label));
        ui.add_space(2.0);
        ui.scope(|ui| {
            ui.set_width(width);
            add(ui);
        });
    });
}

fn stat_card(
    ui: &mut egui::Ui,
    palette: &Palette,
    label: &str,
    value: usize,
    color: egui::Color32,
) {
    egui::Frame::new()
        .fill(palette.card)
        .stroke(egui::Stroke::new(1.0_f32, palette.border))
        .corner_radius(egui::CornerRadius::same(12))
        .inner_margin(egui::Margin::same(15))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                egui::RichText::new(label.to_uppercase())
                    .size(9.5)
                    .strong()
                    .color(palette.muted),
            );
            ui.add_space(2.0);
            ui.label(
                egui::RichText::new(value.to_string())
                    .size(25.0)
                    .strong()
                    .color(color),
            );
        });
}

fn primary_button(
    ui: &mut egui::Ui,
    palette: &Palette,
    label: &str,
    enabled: bool,
) -> egui::Response {
    ui.add_enabled(
        enabled,
        egui::Button::new(
            egui::RichText::new(label)
                .size(12.5)
                .strong()
                .color(palette.accent_text),
        )
        .fill(palette.accent)
        .corner_radius(egui::CornerRadius::same(8))
        .min_size(egui::vec2(0.0, 32.0)),
    )
}

fn secondary_button(ui: &mut egui::Ui, palette: &Palette, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(egui::RichText::new(label).size(12.0).color(palette.text))
            .fill(palette.input)
            .stroke(egui::Stroke::new(1.0_f32, palette.border))
            .corner_radius(egui::CornerRadius::same(8))
            .min_size(egui::vec2(0.0, 30.0)),
    )
}

fn empty_state(ui: &mut egui::Ui, palette: &Palette, title: &str, hint: &str) {
    egui::Frame::new()
        .fill(palette.input)
        .corner_radius(egui::CornerRadius::same(10))
        .inner_margin(egui::Margin::symmetric(16, 18))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                egui::RichText::new(title)
                    .size(13.0)
                    .strong()
                    .color(palette.text),
            );
            ui.label(egui::RichText::new(hint).size(11.0).color(palette.muted));
        });
}

fn badge(ui: &mut egui::Ui, palette: &Palette, text: &str) {
    egui::Frame::new()
        .fill(palette.input)
        .corner_radius(egui::CornerRadius::same(6))
        .inner_margin(egui::Margin::symmetric(7, 3))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(text).size(9.5).color(palette.muted));
        });
}

fn status_pill(ui: &mut egui::Ui, palette: &Palette, status: &EntryStatus) {
    let (background, foreground) = palette.status_colors(status);
    egui::Frame::new()
        .fill(background)
        .corner_radius(egui::CornerRadius::same(20))
        .inner_margin(egui::Margin::symmetric(8, 3))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(status.label())
                    .size(9.5)
                    .strong()
                    .color(foreground),
            );
        });
}

fn game_row(
    ui: &mut egui::Ui,
    palette: &Palette,
    index: usize,
    entry: &crate::state::QueueEntry<GameJob>,
    locked: bool,
) -> Option<RowAction> {
    let mut action = None;
    egui::Frame::new()
        .fill(palette.input)
        .corner_radius(egui::CornerRadius::same(10))
        .inner_margin(egui::Margin::symmetric(12, 9))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!("{:02}", index + 1))
                        .monospace()
                        .size(10.0)
                        .color(palette.muted),
                );
                badge(ui, palette, entry.job.platform.label());
                ui.vertical(|ui| {
                    ui.label(
                        egui::RichText::new(&entry.job.appid)
                            .monospace()
                            .size(12.0)
                            .strong()
                            .color(palette.text),
                    );
                    ui.label(
                        egui::RichText::new(format!("Branch {}", entry.job.branch))
                            .size(10.0)
                            .color(palette.muted),
                    );
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_enabled_ui(!locked, |ui| {
                        if ui.small_button("×").clicked() {
                            action = Some(RowAction::Remove(index));
                        }
                        if ui.small_button("↓").clicked() {
                            action = Some(RowAction::Down(index));
                        }
                        if ui.small_button("↑").clicked() {
                            action = Some(RowAction::Up(index));
                        }
                    });
                    ui.add_space(4.0);
                    status_pill(ui, palette, &entry.status);
                });
            });
        });
    action
}

fn workshop_row(
    ui: &mut egui::Ui,
    palette: &Palette,
    index: usize,
    entry: &crate::state::QueueEntry<WorkshopJob>,
    locked: bool,
) -> Option<RowAction> {
    let mut action = None;
    egui::Frame::new()
        .fill(palette.input)
        .corner_radius(egui::CornerRadius::same(10))
        .inner_margin(egui::Margin::symmetric(12, 9))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!("{:02}", index + 1))
                        .monospace()
                        .size(10.0)
                        .color(palette.muted),
                );
                badge(ui, palette, "WORKSHOP");
                ui.vertical(|ui| {
                    ui.label(
                        egui::RichText::new(&entry.job.workshop_id)
                            .monospace()
                            .size(12.0)
                            .strong()
                            .color(palette.text),
                    );
                    ui.label(
                        egui::RichText::new(format!("App {}", entry.job.appid))
                            .size(10.0)
                            .color(palette.muted),
                    );
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_enabled_ui(!locked, |ui| {
                        if ui.small_button("×").clicked() {
                            action = Some(RowAction::Remove(index));
                        }
                        if ui.small_button("↓").clicked() {
                            action = Some(RowAction::Down(index));
                        }
                        if ui.small_button("↑").clicked() {
                            action = Some(RowAction::Up(index));
                        }
                    });
                    ui.add_space(4.0);
                    status_pill(ui, palette, &entry.status);
                });
            });
        });
    action
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

#[cfg(test)]
mod tests {
    use super::*;
    use steam_press_core::events::PackEvent;

    fn test_app(name: &str) -> SteamPressApp {
        let root = std::env::temp_dir().join(format!(
            "steam-press-gui-test-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        SteamPressApp::for_tests(&root)
    }

    #[test]
    fn cancelled_event_logs_and_marks_the_job() {
        let mut app = test_app("cancel");
        let job = GameJob::new(Platform::Linux64, "10");
        app.game_queue.push(job.clone());
        app.handle_event(PackEvent::Finished {
            job: job.encode(),
            status: FinishStatus::Cancelled,
            outcome: "Cancelled".to_string(),
            output: None,
        });
        assert_eq!(app.game_queue.entries()[0].status, EntryStatus::Cancelled);
        assert!(app.log.iter().any(|entry| entry.text.contains("Cancelled")));
    }

    #[test]
    fn failed_event_sets_error_and_logs() {
        let mut app = test_app("failed");
        let job = GameJob::new(Platform::Linux64, "20");
        app.game_queue.push(job.clone());
        app.handle_event(PackEvent::Finished {
            job: job.encode(),
            status: FinishStatus::Failed,
            outcome: "Steam is rate limiting".to_string(),
            output: None,
        });
        assert!(matches!(
            app.game_queue.entries()[0].status,
            EntryStatus::Failed(_)
        ));
        assert_eq!(app.error.as_deref(), Some("Steam is rate limiting"));
        assert!(app.log.iter().any(|entry| entry.level == LogLevel::Error));
    }

    #[test]
    fn finishing_a_cancelled_queue_marks_remaining_jobs() {
        let mut app = test_app("cancel-queue");
        app.game_queue.push(GameJob::new(Platform::Linux64, "30"));
        app.running = true;
        app.cancel_requested.store(true, Ordering::SeqCst);
        let (sender, receiver) = mpsc::channel();
        sender.send(WorkerEvent::Done).unwrap();
        app.receiver = Some(receiver);
        app.poll();
        assert_eq!(app.game_queue.entries()[0].status, EntryStatus::Cancelled);
        assert_eq!(app.stage, "Cancelled");
        assert!(app
            .log
            .iter()
            .any(|entry| entry.text.contains("Queue cancelled")));
    }

    #[test]
    fn log_is_capped_at_two_thousand_entries() {
        let mut app = test_app("log-cap");
        for index in 0..2100 {
            app.push_log(LogLevel::Info, format!("line {index}"));
        }
        assert_eq!(app.log.len(), 2000);
        assert_eq!(app.log[0].text, "line 100");
        assert_eq!(app.log[1999].text, "line 2099");
    }

    #[test]
    fn a_full_activity_log_renders_headlessly() {
        let mut app = test_app("log-render");
        for index in 0..2000 {
            app.push_log(LogLevel::Info, format!("downloader line {index}"));
        }
        let context = egui::Context::default();
        let _ = context.run(egui::RawInput::default(), |context| {
            app.draw(context);
        });
        assert_eq!(app.log.len(), 2000);
    }

    #[test]
    fn every_tab_renders_headlessly() {
        let mut app = test_app("headless");
        app.game_queue
            .push(GameJob::new(Platform::Linux64, "2379780"));
        app.push_log(LogLevel::Step, "Download - test content");
        let context = egui::Context::default();
        for tab in [Tab::Library, Tab::Workshop, Tab::Depots, Tab::Settings] {
            app.tab = tab;
            let _ = context.run(egui::RawInput::default(), |context| {
                app.draw(context);
            });
        }
    }
}
