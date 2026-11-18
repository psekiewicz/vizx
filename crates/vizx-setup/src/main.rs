//! The VizX setup program. With VizX inside it (see `payload.rs`) it
//! installs VizX; as `uninstall.exe` in an install folder, or started with
//! `--uninstall`, it removes the VizX installed there.
//!
//! When administrator rights are needed (installing for everyone, or a
//! folder the user cannot write to), it starts itself again with them and
//! `--elevated`; that copy begins the work at once and this one closes.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Arc, Mutex};

use eframe::egui;
use eframe::egui_wgpu::{WgpuConfiguration, WgpuSetup};
use vizx_setup::install::{self, Options, Progress, Scope};
use vizx_setup::payload;

const WINDOW_SIZE: [f32; 2] = [600.0, 440.0];
/// Room left on the drive beyond VizX itself, for its pre-rendered frames.
const SPARE_BYTES: u64 = 500 * 1024 * 1024;
const ERROR_COLOR: egui::Color32 = egui::Color32::from_rgb(240, 110, 100);

/// Without a window, for scripts and testing:
///
/// `vizx-setup.exe --silent [--all-users] [--folder <folder>]
/// [--no-shortcuts]` installs, `uninstall.exe --silent` uninstalls. No
/// administrator rights are asked for: where they are needed, run it from
/// an administrator prompt. The exit code says whether it worked (0) or
/// not (1).
fn run_silently(arguments: &[String], this_program: &Path, is_uninstaller: bool) -> ExitCode {
    let result = if is_uninstaller {
        let folder = this_program.parent().unwrap_or(Path::new("."));
        install::uninstall(folder)
    } else {
        install::install(this_program, &Options::from_arguments(arguments), |_| {})
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(problem) => {
            eprintln!("{problem}");
            ExitCode::FAILURE
        }
    }
}

fn main() -> ExitCode {
    let this_program = std::env::current_exe().unwrap_or_default();
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let is_uninstaller = arguments.iter().any(|argument| argument == "--uninstall")
        || this_program.file_name().is_some_and(|name| {
            name.to_string_lossy()
                .eq_ignore_ascii_case(install::UNINSTALLER)
        });
    if arguments.iter().any(|argument| argument == "--silent") {
        return run_silently(&arguments, &this_program, is_uninstaller);
    }
    match run_window(this_program, &arguments, is_uninstaller) {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}

fn run_window(this_program: PathBuf, arguments: &[String], is_uninstaller: bool) -> eframe::Result {
    let starts_at_once = arguments.iter().any(|argument| argument == "--elevated");
    let (title, app) = if is_uninstaller {
        let folder = this_program
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        ("Uninstall VizX", SetupApp::uninstalling(folder))
    } else {
        let mut app = SetupApp::installing(this_program);
        if starts_at_once {
            app.options = Options::from_arguments(arguments);
        }
        ("VizX Setup", app)
    };
    let mut app = app;
    app.starts_at_once = starts_at_once;

    let mut wgpu_options = WgpuConfiguration::default();
    if let WgpuSetup::CreateNew(setup) = &mut wgpu_options.wgpu_setup {
        // As in VizX itself: DirectX 12, or OpenGL on old hardware.
        setup.instance_descriptor.backends = eframe::wgpu::Backends::from_env()
            .unwrap_or(eframe::wgpu::Backends::DX12 | eframe::wgpu::Backends::GL);
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(title)
            .with_inner_size(WINDOW_SIZE)
            .with_resizable(false)
            .with_maximize_button(false),
        wgpu_options,
        ..Default::default()
    };
    eframe::run_native(title, options, Box::new(|_| Ok(Box::new(app))))
}

/// Where the work stands, shared with the thread doing it.
#[derive(Debug, Clone)]
enum Stage {
    /// Waiting for the user to start.
    Ready,
    Working(Progress),
    Finished,
    Failed(String),
}

struct SetupApp {
    /// The setup program itself, which carries VizX (installing only).
    installer: PathBuf,
    is_uninstaller: bool,
    /// For the uninstaller, `folder` is the installation to remove.
    options: Options,
    /// Started again with administrator rights: the work begins without
    /// another click.
    starts_at_once: bool,
    stage: Arc<Mutex<Stage>>,
    /// The size of VizX once installed, if this program carries it.
    install_size: Option<u64>,
}

impl SetupApp {
    fn installing(installer: PathBuf) -> Self {
        let install_size = std::fs::File::open(&installer)
            .ok()
            .and_then(|mut file| payload::read_trailer(&mut file).ok().flatten())
            .map(|trailer| trailer.total_size);
        Self {
            installer,
            is_uninstaller: false,
            options: Options {
                folder: install::default_folder(Scope::CurrentUser),
                scope: Scope::CurrentUser,
                desktop_shortcut: true,
                start_menu_shortcut: true,
            },
            starts_at_once: false,
            stage: Arc::new(Mutex::new(Stage::Ready)),
            install_size,
        }
    }

    fn uninstalling(folder: PathBuf) -> Self {
        Self {
            installer: PathBuf::new(),
            is_uninstaller: true,
            options: Options {
                folder,
                scope: Scope::CurrentUser,
                desktop_shortcut: false,
                start_menu_shortcut: false,
            },
            starts_at_once: false,
            stage: Arc::new(Mutex::new(Stage::Ready)),
            install_size: None,
        }
    }

    fn stage(&self) -> Stage {
        self.stage
            .lock()
            .map(|stage| stage.clone())
            .unwrap_or(Stage::Ready)
    }

    /// Starts installing: on a thread of its own, or, when administrator
    /// rights are needed, in a copy of this program that has them (this
    /// window then closes).
    fn start_install(&self, ctx: &egui::Context) {
        if install::install_needs_elevation(&self.options) {
            match install::run_elevated(&self.options.elevated_arguments()) {
                Ok(()) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                Err(problem) => set(&self.stage, Stage::Failed(problem)),
            }
            return;
        }
        let options = self.options.clone();
        let installer = self.installer.clone();
        let stage = Arc::clone(&self.stage);
        let ctx = ctx.clone();
        set(&stage, Stage::Working(Progress::default()));
        std::thread::spawn(move || {
            let result = install::install(&installer, &options, |progress| {
                set(&stage, Stage::Working(progress));
                ctx.request_repaint();
            });
            set(
                &stage,
                match result {
                    Ok(()) => Stage::Finished,
                    Err(problem) => Stage::Failed(problem),
                },
            );
            ctx.request_repaint();
        });
    }

    fn start_uninstall(&self, ctx: &egui::Context) {
        let folder = self.options.folder.clone();
        if install::uninstall_needs_elevation(&folder) {
            match install::run_elevated("--uninstall --elevated") {
                Ok(()) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                Err(problem) => set(&self.stage, Stage::Failed(problem)),
            }
            return;
        }
        let stage = Arc::clone(&self.stage);
        let ctx = ctx.clone();
        set(&stage, Stage::Working(Progress::default()));
        std::thread::spawn(move || {
            let result = install::uninstall(&folder);
            set(
                &stage,
                match result {
                    Ok(()) => Stage::Finished,
                    Err(problem) => Stage::Failed(problem),
                },
            );
            ctx.request_repaint();
        });
    }

    /// A big button for the main action, with a note when Windows will ask
    /// for administrator rights first.
    fn action_button(ui: &mut egui::Ui, label: &str, enabled: bool, needs_admin: bool) -> bool {
        let button = egui::Button::new(egui::RichText::new(label).size(16.0))
            .min_size(egui::vec2(130.0, 32.0));
        let clicked = ui.add_enabled(enabled, button).clicked();
        if needs_admin && enabled {
            ui.label(
                egui::RichText::new("Windows will ask for administrator rights.")
                    .small()
                    .weak(),
            );
        }
        clicked
    }

    fn install_page(&mut self, ui: &mut egui::Ui) {
        let Some(size) = self.install_size else {
            ui.label("This setup program does not contain VizX (it may be damaged).");
            return;
        };
        ui.label(format!(
            "VizX {} takes {} once installed.",
            env!("CARGO_PKG_VERSION"),
            megabytes(size)
        ));
        ui.add_space(10.0);

        ui.label("Install for");
        let previous_scope = self.options.scope;
        for scope in Scope::ALL {
            ui.radio_value(&mut self.options.scope, scope, scope.label());
        }
        // Switching who it is for moves the folder along, unless the user
        // chose one of their own.
        if self.options.scope != previous_scope
            && self.options.folder == install::default_folder(previous_scope)
        {
            self.options.folder = install::default_folder(self.options.scope);
        }

        ui.add_space(8.0);
        ui.label("Install folder");
        ui.horizontal(|ui| {
            let mut text = self.options.folder.to_string_lossy().into_owned();
            let field = egui::TextEdit::singleline(&mut text).desired_width(440.0);
            if ui.add(field).changed() {
                self.options.folder = PathBuf::from(text);
            }
            if ui.button("Browse...").clicked()
                && let Some(chosen) = rfd::FileDialog::new()
                    .set_title("Where should VizX be installed?")
                    .pick_folder()
            {
                self.options.folder = install::folder_for_choice(&chosen);
            }
        });

        let folder = &self.options.folder;
        let folder_problem = install::check_folder(folder).err();
        let free = install::free_space(folder);
        let space_problem = free.filter(|free| *free < size + SPARE_BYTES).map(|free| {
            format!(
                "Not enough space on this drive: {} free, {} needed. Choose another drive.",
                megabytes(free),
                megabytes(size + SPARE_BYTES)
            )
        });
        match (&folder_problem, &space_problem) {
            (Some(problem), _) | (None, Some(problem)) => {
                ui.colored_label(ERROR_COLOR, problem);
            }
            (None, None) => {
                let note = match (install::is_installation(folder), free) {
                    (true, _) => "VizX is already installed here: it will be updated.".to_owned(),
                    (false, Some(free)) => format!("{} free on this drive.", megabytes(free)),
                    (false, None) => String::new(),
                };
                ui.label(egui::RichText::new(note).weak());
            }
        }

        ui.add_space(8.0);
        let (start_menu, desktop) = match self.options.scope {
            Scope::CurrentUser => ("Start menu shortcut", "Desktop shortcut"),
            Scope::AllUsers => (
                "Start menu shortcut (for everyone)",
                "Desktop shortcut (for everyone)",
            ),
        };
        ui.checkbox(&mut self.options.start_menu_shortcut, start_menu);
        ui.checkbox(&mut self.options.desktop_shortcut, desktop);

        let can_install = folder_problem.is_none() && space_problem.is_none();
        let needs_admin = install::install_needs_elevation(&self.options);
        ui.with_layout(egui::Layout::bottom_up(egui::Align::RIGHT), |ui| {
            if Self::action_button(ui, "Install", can_install, needs_admin) {
                self.start_install(ui.ctx());
            }
        });
    }

    fn uninstall_page(&mut self, ui: &mut egui::Ui) {
        let folder = &self.options.folder;
        if !install::is_installation(folder) {
            ui.label(format!(
                "There is no VizX installation in {}.",
                folder.display()
            ));
            return;
        }
        let for_whom = match install::installed_scope(folder) {
            Scope::CurrentUser => "",
            Scope::AllUsers => " for everyone on this computer",
        };
        ui.label(format!(
            "VizX{for_whom} will be removed from {}, with its shortcuts.",
            folder.display()
        ));
        ui.add_space(6.0);
        ui.label(
            "Its pre-rendered frames and the stems it made (the data folder) are removed \
             too. Your projects, songs and pictures stay where they are.",
        );
        let needs_admin = install::uninstall_needs_elevation(folder);
        ui.with_layout(egui::Layout::bottom_up(egui::Align::RIGHT), |ui| {
            if Self::action_button(ui, "Uninstall", true, needs_admin) {
                self.start_uninstall(ui.ctx());
            }
        });
    }

    /// Starts VizX for the user. From a setup program running with
    /// administrator rights, VizX is started through Explorer, so it does
    /// not inherit those rights.
    fn start_vizx(&self) {
        let program = self.options.folder.join(install::APP_EXECUTABLE);
        let _ = if install::is_elevated() {
            std::process::Command::new("explorer.exe")
                .arg(&program)
                .spawn()
        } else {
            std::process::Command::new(&program)
                .current_dir(&self.options.folder)
                .spawn()
        };
    }
}

fn set(stage: &Mutex<Stage>, value: Stage) {
    if let Ok(mut stage) = stage.lock() {
        *stage = value;
    }
}

fn megabytes(bytes: u64) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.1} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    } else {
        format!("{:.0} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

impl eframe::App for SetupApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if std::mem::take(&mut self.starts_at_once) {
            if self.is_uninstaller {
                self.start_uninstall(ui.ctx());
            } else {
                self.start_install(ui.ctx());
            }
        }
        egui::CentralPanel::default().show(ui, |ui| {
            ui.add_space(6.0);
            ui.heading(if self.is_uninstaller {
                "Uninstall VizX"
            } else {
                "Install VizX"
            });
            ui.label(egui::RichText::new("Music visualizer editor").weak());
            ui.separator();
            ui.add_space(6.0);

            match self.stage() {
                Stage::Ready if self.is_uninstaller => self.uninstall_page(ui),
                Stage::Ready => self.install_page(ui),
                Stage::Working(progress) => {
                    let fraction = if progress.total_bytes > 0 {
                        progress.done_bytes as f32 / progress.total_bytes as f32
                    } else {
                        0.0
                    };
                    ui.label(if self.is_uninstaller {
                        "Removing VizX..."
                    } else {
                        "Installing VizX..."
                    });
                    ui.add_space(8.0);
                    if !self.is_uninstaller {
                        ui.add(egui::ProgressBar::new(fraction).show_percentage());
                        ui.label(egui::RichText::new(progress.file).small().weak());
                    } else {
                        ui.spinner();
                    }
                }
                Stage::Finished if self.is_uninstaller => {
                    ui.label("VizX has been removed.");
                    ui.with_layout(egui::Layout::bottom_up(egui::Align::RIGHT), |ui| {
                        if ui.button("Close").clicked() {
                            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    });
                }
                Stage::Finished => {
                    ui.label(format!(
                        "VizX is installed in {}.",
                        self.options.folder.display()
                    ));
                    ui.with_layout(egui::Layout::bottom_up(egui::Align::RIGHT), |ui| {
                        ui.horizontal(|ui| {
                            if ui.button("Close").clicked() {
                                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                            }
                            let start =
                                egui::Button::new(egui::RichText::new("Start VizX").size(16.0))
                                    .min_size(egui::vec2(130.0, 32.0));
                            if ui.add(start).clicked() {
                                self.start_vizx();
                                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                            }
                        });
                    });
                }
                Stage::Failed(problem) => {
                    ui.colored_label(
                        ERROR_COLOR,
                        if self.is_uninstaller {
                            "VizX could not be removed:"
                        } else {
                            "VizX could not be installed:"
                        },
                    );
                    ui.label(problem);
                    ui.with_layout(egui::Layout::bottom_up(egui::Align::RIGHT), |ui| {
                        ui.horizontal(|ui| {
                            if ui.button("Back").clicked() {
                                set(&self.stage, Stage::Ready);
                            }
                            // The last resort for a folder that turned out
                            // to need administrator rights.
                            if !install::is_elevated()
                                && ui.button("Try as administrator").clicked()
                            {
                                let arguments = if self.is_uninstaller {
                                    "--uninstall --elevated".to_owned()
                                } else {
                                    self.options.elevated_arguments()
                                };
                                match install::run_elevated(&arguments) {
                                    Ok(()) => {
                                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close)
                                    }
                                    Err(problem) => set(&self.stage, Stage::Failed(problem)),
                                }
                            }
                        });
                    });
                }
            }
        });
    }
}
