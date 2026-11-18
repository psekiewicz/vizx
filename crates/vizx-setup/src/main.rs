//! The VizX setup program. With VizX inside it (see `payload.rs`) it
//! installs VizX; as `uninstall.exe` in an install folder, or started with
//! `--uninstall`, it removes the VizX installed there.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Arc, Mutex};

use eframe::egui;
use eframe::egui_wgpu::{WgpuConfiguration, WgpuSetup};
use vizx_setup::install::{self, Options, Progress};
use vizx_setup::payload;

const WINDOW_SIZE: [f32; 2] = [560.0, 380.0];
/// Room left on the drive beyond VizX itself, for its pre-rendered frames.
const SPARE_BYTES: u64 = 500 * 1024 * 1024;

/// Without a window, for scripts and testing:
///
/// `vizx-setup.exe --silent [--folder <folder>] [--no-shortcuts]` installs,
/// `uninstall.exe --silent` uninstalls. The exit code says whether it
/// worked (0) or not (1).
fn run_silently(arguments: &[String], this_program: &Path, is_uninstaller: bool) -> ExitCode {
    let result = if is_uninstaller {
        let folder = this_program.parent().unwrap_or(Path::new("."));
        install::uninstall(folder)
    } else {
        let folder = arguments
            .iter()
            .position(|argument| argument == "--folder")
            .and_then(|index| arguments.get(index + 1))
            .map_or_else(install::default_folder, PathBuf::from);
        let with_shortcuts = !arguments
            .iter()
            .any(|argument| argument == "--no-shortcuts");
        let options = Options {
            folder,
            desktop_shortcut: with_shortcuts,
            start_menu_shortcut: with_shortcuts,
        };
        install::install(this_program, &options, |_| {})
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
    match run_window(this_program, is_uninstaller) {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}

fn run_window(this_program: PathBuf, is_uninstaller: bool) -> eframe::Result {
    let (title, app) = if is_uninstaller {
        let folder = this_program
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        ("Uninstall VizX", SetupApp::uninstalling(folder))
    } else {
        ("VizX Setup", SetupApp::installing(this_program))
    };

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
    folder: PathBuf,
    desktop_shortcut: bool,
    start_menu_shortcut: bool,
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
            folder: install::default_folder(),
            desktop_shortcut: true,
            start_menu_shortcut: true,
            stage: Arc::new(Mutex::new(Stage::Ready)),
            install_size,
        }
    }

    fn uninstalling(folder: PathBuf) -> Self {
        Self {
            installer: PathBuf::new(),
            is_uninstaller: true,
            folder,
            desktop_shortcut: false,
            start_menu_shortcut: false,
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

    /// Starts the installation on a thread of its own.
    fn start_install(&self, ctx: &egui::Context) {
        let options = Options {
            folder: self.folder.clone(),
            desktop_shortcut: self.desktop_shortcut,
            start_menu_shortcut: self.start_menu_shortcut,
        };
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
        let folder = self.folder.clone();
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

    fn install_page(&mut self, ui: &mut egui::Ui) {
        let Some(size) = self.install_size else {
            ui.label("This setup program does not contain VizX (it may be damaged).");
            return;
        };
        ui.label(format!(
            "VizX {} will be installed for you (no administrator rights needed). \
             It takes {}.",
            env!("CARGO_PKG_VERSION"),
            megabytes(size)
        ));
        ui.add_space(12.0);
        ui.label("Install folder");
        ui.horizontal(|ui| {
            let mut text = self.folder.to_string_lossy().into_owned();
            let field = egui::TextEdit::singleline(&mut text).desired_width(400.0);
            if ui.add(field).changed() {
                self.folder = PathBuf::from(text);
            }
            if ui.button("Browse...").clicked()
                && let Some(chosen) = rfd::FileDialog::new()
                    .set_title("Where should VizX be installed?")
                    .pick_folder()
            {
                self.folder = install::folder_for_choice(&chosen);
            }
        });

        let folder_problem = install::check_folder(&self.folder).err();
        let free = install::free_space(&self.folder);
        let space_problem = free.filter(|free| *free < size + SPARE_BYTES).map(|free| {
            format!(
                "Not enough space on this drive: {} free, {} needed. Choose another drive.",
                megabytes(free),
                megabytes(size + SPARE_BYTES)
            )
        });
        let is_update = install::is_installation(&self.folder);
        match (&folder_problem, &space_problem) {
            (Some(problem), _) | (None, Some(problem)) => {
                ui.colored_label(egui::Color32::from_rgb(240, 110, 100), problem);
            }
            (None, None) => {
                let note = match (is_update, free) {
                    (true, _) => "VizX is already installed here: it will be updated.".to_owned(),
                    (false, Some(free)) => format!("{} free on this drive.", megabytes(free)),
                    (false, None) => String::new(),
                };
                ui.label(egui::RichText::new(note).weak());
            }
        }

        ui.add_space(8.0);
        ui.checkbox(&mut self.start_menu_shortcut, "Start menu shortcut");
        ui.checkbox(&mut self.desktop_shortcut, "Desktop shortcut");

        ui.with_layout(egui::Layout::bottom_up(egui::Align::RIGHT), |ui| {
            let can_install = folder_problem.is_none() && space_problem.is_none();
            let button = egui::Button::new(egui::RichText::new("Install").size(16.0))
                .min_size(egui::vec2(120.0, 32.0));
            if ui.add_enabled(can_install, button).clicked() {
                self.start_install(ui.ctx());
            }
        });
    }

    fn uninstall_page(&mut self, ui: &mut egui::Ui) {
        if !install::is_installation(&self.folder) {
            ui.label(format!(
                "There is no VizX installation in {}.",
                self.folder.display()
            ));
            return;
        }
        ui.label(format!(
            "VizX will be removed from {}, with its shortcuts.",
            self.folder.display()
        ));
        ui.add_space(6.0);
        ui.label(
            "Its pre-rendered frames and the stems it made (the data folder) are removed \
             too. Your projects, songs and pictures stay where they are.",
        );
        ui.with_layout(egui::Layout::bottom_up(egui::Align::RIGHT), |ui| {
            let button = egui::Button::new(egui::RichText::new("Uninstall").size(16.0))
                .min_size(egui::vec2(120.0, 32.0));
            if ui.add(button).clicked() {
                self.start_uninstall(ui.ctx());
            }
        });
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
                    ui.label(format!("VizX is installed in {}.", self.folder.display()));
                    ui.with_layout(egui::Layout::bottom_up(egui::Align::RIGHT), |ui| {
                        ui.horizontal(|ui| {
                            if ui.button("Close").clicked() {
                                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                            }
                            let start =
                                egui::Button::new(egui::RichText::new("Start VizX").size(16.0))
                                    .min_size(egui::vec2(120.0, 32.0));
                            if ui.add(start).clicked() {
                                let _ = std::process::Command::new(
                                    self.folder.join(install::APP_EXECUTABLE),
                                )
                                .current_dir(&self.folder)
                                .spawn();
                                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                            }
                        });
                    });
                }
                Stage::Failed(problem) => {
                    ui.colored_label(
                        egui::Color32::from_rgb(240, 110, 100),
                        if self.is_uninstaller {
                            "VizX could not be removed:"
                        } else {
                            "VizX could not be installed:"
                        },
                    );
                    ui.label(problem);
                    ui.with_layout(egui::Layout::bottom_up(egui::Align::RIGHT), |ui| {
                        if ui.button("Back").clicked() {
                            set(&self.stage, Stage::Ready);
                        }
                    });
                }
            }
        });
    }
}
