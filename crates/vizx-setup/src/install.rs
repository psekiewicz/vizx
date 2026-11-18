//! Installing VizX into a folder and removing it again.
//!
//! VizX is installed either for the current user only or for everyone on
//! the computer ([`Scope`]):
//!
//! | | Only for me | For everyone |
//! |---|---|---|
//! | Folder (default) | `%LOCALAPPDATA%\Programs\VizX` | `%ProgramFiles%\VizX` |
//! | Shortcuts | the user's Start menu and desktop | everyone's Start menu and the public desktop |
//! | "Apps & features" | `HKEY_CURRENT_USER` | `HKEY_LOCAL_MACHINE` |
//! | Administrator rights | only as a last resort | always |
//!
//! "Only for me" asks for administrator rights only when the chosen folder
//! cannot be written without them (for example a folder in Program Files):
//! the setup program then starts itself again with those rights (Windows
//! asks the user), see [`run_elevated`].
//!
//! The installer writes down every file it puts into the folder (the
//! manifest). The uninstaller removes exactly those files, VizX's `data`
//! folder and the folders left empty, never anything else, so a folder
//! chosen by mistake cannot lose files that were there before.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::payload;

/// Name of the application, for shortcuts and "Apps & features".
pub const APP_NAME: &str = "VizX";
/// The program the shortcuts start.
pub const APP_EXECUTABLE: &str = "vizx.exe";
/// The uninstaller in the install folder (the setup program without its
/// payload).
pub const UNINSTALLER: &str = "uninstall.exe";
/// The list of installed files, one per line, relative to the folder.
pub const MANIFEST: &str = "vizx-install-manifest.txt";
/// VizX's own large files (pre-rendered frames, stems): VizX uses a `data`
/// folder next to it when there is one.
const DATA_FOLDER: &str = "data";
/// Where "Apps & features" finds VizX, under HKCU or HKLM.
const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\VizX";

/// Who VizX is installed for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Only the current user; no administrator rights needed (normally).
    CurrentUser,
    /// Everyone on this computer; needs administrator rights.
    AllUsers,
}

impl Scope {
    pub const ALL: [Self; 2] = [Self::CurrentUser, Self::AllUsers];

    /// Name shown to the user.
    pub fn label(self) -> &'static str {
        match self {
            Self::CurrentUser => "Only for me",
            Self::AllUsers => "For everyone on this computer (administrator rights needed)",
        }
    }

    /// How the scope is passed on the command line.
    pub fn argument(self) -> &'static str {
        match self {
            Self::CurrentUser => "user",
            Self::AllUsers => "all",
        }
    }

    pub fn from_argument(argument: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|scope| scope.argument() == argument)
    }

    /// The registry key of the "Apps & features" entry, as reg.exe names it.
    fn registry_key(self) -> String {
        let root = match self {
            Self::CurrentUser => "HKCU",
            Self::AllUsers => "HKLM",
        };
        format!(r"{root}\{UNINSTALL_KEY}")
    }

    /// The special folders (as .NET names them) for the Start menu and
    /// desktop shortcuts.
    fn shortcut_folders(self) -> (&'static str, &'static str) {
        match self {
            Self::CurrentUser => ("Programs", "Desktop"),
            Self::AllUsers => ("CommonPrograms", "CommonDesktopDirectory"),
        }
    }
}

/// What the user chose before installing.
#[derive(Debug, Clone)]
pub struct Options {
    pub folder: PathBuf,
    pub scope: Scope,
    pub desktop_shortcut: bool,
    pub start_menu_shortcut: bool,
}

impl Options {
    /// The command line that makes the setup program install with these
    /// options when it starts again with administrator rights.
    pub fn elevated_arguments(&self) -> String {
        let mut arguments = format!(
            "--elevated --scope {} --folder \"{}\"",
            self.scope.argument(),
            self.folder.display()
        );
        if !self.desktop_shortcut {
            arguments.push_str(" --no-desktop");
        }
        if !self.start_menu_shortcut {
            arguments.push_str(" --no-start-menu");
        }
        arguments
    }

    /// The options a command line describes (see [`Self::elevated_arguments`]
    /// and the silent mode of the setup program).
    pub fn from_arguments(arguments: &[String]) -> Self {
        let value = |name: &str| {
            arguments
                .iter()
                .position(|argument| argument == name)
                .and_then(|index| arguments.get(index + 1))
        };
        let has = |name: &str| arguments.iter().any(|argument| argument == name);
        let scope = value("--scope")
            .and_then(|scope| Scope::from_argument(scope))
            .unwrap_or(if has("--all-users") {
                Scope::AllUsers
            } else {
                Scope::CurrentUser
            });
        Self {
            folder: value("--folder").map_or_else(|| default_folder(scope), PathBuf::from),
            scope,
            desktop_shortcut: !has("--no-shortcuts") && !has("--no-desktop"),
            start_menu_shortcut: !has("--no-shortcuts") && !has("--no-start-menu"),
        }
    }
}

/// Where VizX goes for `scope` unless the user chooses otherwise.
pub fn default_folder(scope: Scope) -> PathBuf {
    let (variable, inside) = match scope {
        Scope::CurrentUser => ("LOCALAPPDATA", Some("Programs")),
        Scope::AllUsers => ("ProgramFiles", None),
    };
    let base = std::env::var_os(variable)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Program Files"));
    match inside {
        Some(inside) => base.join(inside).join(APP_NAME),
        None => base.join(APP_NAME),
    }
}

/// The folder to install into when the user picked `chosen` in the folder
/// dialog: a `VizX` folder inside it, unless it is one already, so VizX
/// never spreads its files among others.
pub fn folder_for_choice(chosen: &Path) -> PathBuf {
    let is_vizx_folder = chosen
        .file_name()
        .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case(APP_NAME));
    if is_vizx_folder {
        chosen.to_path_buf()
    } else {
        chosen.join(APP_NAME)
    }
}

/// Whether VizX may be installed into `folder`: a new or empty folder, or
/// an earlier VizX installation (which is updated). Otherwise the reason.
pub fn check_folder(folder: &Path) -> Result<(), String> {
    if !folder.is_absolute() {
        return Err("Choose a full folder path, such as D:\\Programs\\VizX.".to_owned());
    }
    let Ok(mut entries) = fs::read_dir(folder) else {
        return Ok(()); // does not exist yet
    };
    let is_empty = entries.next().is_none();
    if is_empty || folder.join(MANIFEST).is_file() {
        Ok(())
    } else {
        Err("This folder already holds other files. Choose an empty or a new folder.".to_owned())
    }
}

/// Whether this program may write into `folder` (or, if it does not exist
/// yet, into the nearest folder above it that does), found out by trying.
pub fn can_write(folder: &Path) -> bool {
    let Some(existing) = folder.ancestors().find(|path| path.is_dir()) else {
        return false;
    };
    let probe = existing.join(format!(".vizx-write-test-{}", std::process::id()));
    match fs::File::create(&probe) {
        Ok(_) => {
            let _ = fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// Whether this program runs with administrator rights.
#[cfg(windows)]
pub fn is_elevated() -> bool {
    // SAFETY: takes no arguments and only reads the process token.
    unsafe { windows_sys::Win32::UI::Shell::IsUserAnAdmin() != 0 }
}

#[cfg(not(windows))]
pub fn is_elevated() -> bool {
    false
}

/// Whether installing with `options` needs administrator rights this
/// program does not have: always for everyone, and for the current user
/// only when the folder cannot be written otherwise.
pub fn install_needs_elevation(options: &Options) -> bool {
    !is_elevated() && (options.scope == Scope::AllUsers || !can_write(&options.folder))
}

/// Whether removing the VizX in `folder` needs administrator rights this
/// program does not have.
pub fn uninstall_needs_elevation(folder: &Path) -> bool {
    !is_elevated() && (installed_scope(folder) == Scope::AllUsers || !can_write(folder))
}

/// Starts this program again with administrator rights and `arguments`.
/// Windows asks the user first; declining is reported as an error.
#[cfg(windows)]
pub fn run_elevated(arguments: &str) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    let wide = |text: &std::ffi::OsStr| -> Vec<u16> {
        text.encode_wide().chain(std::iter::once(0)).collect()
    };
    let program = std::env::current_exe().map_err(|error| error.to_string())?;
    let verb = wide("runas".as_ref());
    let file = wide(program.as_os_str());
    let parameters = wide(arguments.as_ref());
    const SW_SHOWNORMAL: i32 = 1;
    // SAFETY: every string is NUL-terminated and outlives the call; no
    // window or directory is passed (null).
    let result = unsafe {
        windows_sys::Win32::UI::Shell::ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            parameters.as_ptr(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    // Values above 32 mean it started; the others are error codes.
    if result as isize > 32 {
        Ok(())
    } else {
        Err("Administrator rights were not granted, so nothing was changed.".to_owned())
    }
}

#[cfg(not(windows))]
pub fn run_elevated(_arguments: &str) -> Result<(), String> {
    Err("Administrator rights are only requested on Windows.".to_owned())
}

/// Free space on the drive of `folder`, in bytes, if it can be found out.
#[cfg(windows)]
pub fn free_space(folder: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    // The folder may not exist yet: ask about the nearest one that does.
    let existing = folder.ancestors().find(|path| path.is_dir())?;
    let wide: Vec<u16> = existing
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut available = 0u64;
    // SAFETY: `wide` is a NUL-terminated path; the other pointers are
    // either valid for writing a u64 or null, as the function allows.
    let succeeded = unsafe {
        windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut available,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    (succeeded != 0).then_some(available)
}

#[cfg(not(windows))]
pub fn free_space(_folder: &Path) -> Option<u64> {
    None
}

/// A command run without a console window popping up.
fn hidden(program: &str) -> Command {
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

/// `text` inside single quotes for PowerShell.
fn powershell_quoted(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

fn run_powershell(script: &str) -> Result<(), String> {
    let status = hidden("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ])
        .status()
        .map_err(|error| format!("PowerShell could not be started: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("PowerShell reported an error".to_owned())
    }
}

fn create_shortcuts(options: &Options) -> Result<(), String> {
    let (start_menu, desktop) = options.scope.shortcut_folders();
    let mut places = Vec::new();
    if options.start_menu_shortcut {
        places.push(start_menu);
    }
    if options.desktop_shortcut {
        places.push(desktop);
    }
    if places.is_empty() {
        return Ok(());
    }
    let target = options.folder.join(APP_EXECUTABLE);
    let mut script = String::from("$shell = New-Object -ComObject WScript.Shell\n");
    for place in places {
        script.push_str(&format!(
            "$link = $shell.CreateShortcut((Join-Path ([Environment]::GetFolderPath('{place}')) \
             '{APP_NAME}.lnk'))\n\
             $link.TargetPath = {target}\n\
             $link.WorkingDirectory = {folder}\n\
             $link.Description = 'VizX music visualizer editor'\n\
             $link.Save()\n",
            target = powershell_quoted(&target.to_string_lossy()),
            folder = powershell_quoted(&options.folder.to_string_lossy()),
        ));
    }
    run_powershell(&script)
}

/// Removes the shortcuts (the user's and everyone's) that start the VizX
/// in `folder`; shortcuts of another VizX installation stay.
fn remove_shortcuts(folder: &Path) {
    let target = folder.join(APP_EXECUTABLE);
    let script = format!(
        "$shell = New-Object -ComObject WScript.Shell\n\
         foreach ($place in 'Programs', 'Desktop', 'CommonPrograms', \
                  'CommonDesktopDirectory') {{\n\
           $path = Join-Path ([Environment]::GetFolderPath($place)) '{APP_NAME}.lnk'\n\
           if ((Test-Path -LiteralPath $path) -and \
               $shell.CreateShortcut($path).TargetPath -eq {target}) {{\n\
             Remove-Item -LiteralPath $path -Force -ErrorAction SilentlyContinue\n\
           }}\n\
         }}\n",
        target = powershell_quoted(&target.to_string_lossy()),
    );
    let _ = run_powershell(&script);
}

fn registry_add(scope: Scope, name: &str, kind: &str, value: &str) -> Result<(), String> {
    let status = hidden("reg.exe")
        .args([
            "add",
            &scope.registry_key(),
            "/v",
            name,
            "/t",
            kind,
            "/d",
            value,
            "/f",
        ])
        .status()
        .map_err(|error| format!("the registry could not be changed: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("the registry value {name} could not be written"))
    }
}

/// The folder the "Apps & features" entry of `scope` points to, if there
/// is one.
fn registered_folder(scope: Scope) -> Option<PathBuf> {
    let output = hidden("reg.exe")
        .args(["query", &scope.registry_key(), "/v", "InstallLocation"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let line = text.lines().find(|line| line.contains("InstallLocation"))?;
    let (_, value) = line.split_once("REG_SZ")?;
    Some(PathBuf::from(value.trim()))
}

fn same_folder(a: &Path, b: &Path) -> bool {
    let normal = |path: &Path| {
        path.to_string_lossy()
            .trim_end_matches(['\\', '/'])
            .to_lowercase()
    };
    normal(a) == normal(b)
}

/// Who the VizX in `folder` was installed for, as its entry in "Apps &
/// features" tells.
pub fn installed_scope(folder: &Path) -> Scope {
    let is_for_everyone = registered_folder(Scope::AllUsers)
        .is_some_and(|registered| same_folder(&registered, folder));
    if is_for_everyone {
        Scope::AllUsers
    } else {
        Scope::CurrentUser
    }
}

/// Adds VizX to "Apps & features", where it can be uninstalled.
fn register(scope: Scope, folder: &Path, size_bytes: u64) -> Result<(), String> {
    let uninstaller = folder.join(UNINSTALLER);
    let program = folder.join(APP_EXECUTABLE);
    let add = |name: &str, kind: &str, value: &str| registry_add(scope, name, kind, value);
    add("DisplayName", "REG_SZ", APP_NAME)?;
    add("DisplayVersion", "REG_SZ", env!("CARGO_PKG_VERSION"))?;
    add("Publisher", "REG_SZ", "VizX")?;
    add("InstallLocation", "REG_SZ", &folder.to_string_lossy())?;
    add("DisplayIcon", "REG_SZ", &program.to_string_lossy())?;
    add(
        "UninstallString",
        "REG_SZ",
        &format!("\"{}\"", uninstaller.to_string_lossy()),
    )?;
    add(
        "EstimatedSize",
        "REG_DWORD",
        &(size_bytes / 1024).min(u64::from(u32::MAX)).to_string(),
    )?;
    add("NoModify", "REG_DWORD", "1")?;
    add("NoRepair", "REG_DWORD", "1")
}

/// Removes the "Apps & features" entries that point to `folder` (an entry
/// of another VizX installation stays).
fn unregister(folder: &Path) {
    for scope in Scope::ALL {
        if registered_folder(scope).is_some_and(|registered| same_folder(&registered, folder)) {
            let _ = hidden("reg.exe")
                .args(["delete", &scope.registry_key(), "/f"])
                .status();
        }
    }
}

/// What is being done right now, for the progress bar.
#[derive(Debug, Clone, Default)]
pub struct Progress {
    pub file: String,
    pub done_bytes: u64,
    pub total_bytes: u64,
}

/// Installs the VizX inside the installer `installer` as `options` say.
/// `progress` hears how far it is.
pub fn install(
    installer: &Path,
    options: &Options,
    mut progress: impl FnMut(Progress),
) -> Result<(), String> {
    check_folder(&options.folder)?;
    if install_needs_elevation(options) {
        return Err("Installing here needs administrator rights.".to_owned());
    }
    let folder = &options.folder;
    let total_bytes = {
        let mut file = fs::File::open(installer).map_err(|error| error.to_string())?;
        payload::read_trailer(&mut file)
            .map_err(|error| error.to_string())?
            .ok_or("This setup program holds no VizX.")?
            .total_size
    };
    fs::create_dir_all(folder)
        .map_err(|error| format!("The folder could not be created: {error}"))?;

    let files = payload::unpack(installer, folder, |file, done_bytes| {
        progress(Progress {
            file: file.to_owned(),
            done_bytes,
            total_bytes,
        });
    })
    .map_err(|error| {
        if error.kind() == std::io::ErrorKind::PermissionDenied {
            format!("{error}\n\nIf VizX is running, close it and try again.")
        } else {
            error.to_string()
        }
    })?;

    // A `data` folder next to VizX keeps its large files there. Only when
    // the user can write to it: a folder made with administrator rights
    // (as in Program Files) is not, and VizX then uses the user's
    // temporary folder instead.
    if options.scope == Scope::CurrentUser && !is_elevated() {
        fs::create_dir_all(folder.join(DATA_FOLDER)).map_err(|error| error.to_string())?;
    }
    payload::write_stub(installer, &folder.join(UNINSTALLER))
        .map_err(|error| format!("The uninstaller could not be written: {error}"))?;

    // Files of an earlier version that this one no longer has are kept
    // in the list, so uninstalling removes them too.
    let mut listed = read_manifest(folder);
    for file in files
        .iter()
        .map(|file| file.to_string_lossy().replace('\\', "/"))
    {
        if !listed.contains(&file) {
            listed.push(file);
        }
    }
    if !listed.iter().any(|file| file == UNINSTALLER) {
        listed.push(UNINSTALLER.to_owned());
    }
    fs::write(folder.join(MANIFEST), listed.join("\n"))
        .map_err(|error| format!("The list of files could not be written: {error}"))?;

    progress(Progress {
        file: "Creating shortcuts".to_owned(),
        done_bytes: total_bytes,
        total_bytes,
    });
    create_shortcuts(options)?;
    // Installed again for the other scope: the old entry would point to
    // the same folder twice.
    unregister(folder);
    register(options.scope, folder, total_bytes)?;
    Ok(())
}

fn read_manifest(folder: &Path) -> Vec<String> {
    fs::read_to_string(folder.join(MANIFEST))
        .map(|text| {
            text.lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// Whether `folder` holds a VizX installed by this installer.
pub fn is_installation(folder: &Path) -> bool {
    folder.join(MANIFEST).is_file()
}

/// Whether the VizX in `folder` is running: Windows lets nobody write to a
/// program while it runs. (Asked only once this program may write to the
/// folder, so a refusal means the program is in use.)
fn is_running(folder: &Path) -> bool {
    let program = folder.join(APP_EXECUTABLE);
    program.is_file()
        && fs::OpenOptions::new()
            .write(true)
            .open(&program)
            .is_err_and(|error| error.kind() != std::io::ErrorKind::NotFound)
}

/// Removes the VizX installed in `folder`: the installed files, its `data`
/// folder, the folders left empty, its shortcuts and its entry in "Apps &
/// features". The running uninstaller removes itself (and the folder, if
/// nothing else is in it) a moment after it has closed.
///
/// Nothing is removed while VizX is running, so it is never left half
/// uninstalled.
pub fn uninstall(folder: &Path) -> Result<(), String> {
    if !is_installation(folder) {
        return Err(format!(
            "{} does not hold a VizX installation.",
            folder.display()
        ));
    }
    if uninstall_needs_elevation(folder) {
        return Err("Removing VizX from here needs administrator rights.".to_owned());
    }
    if is_running(folder) {
        return Err("VizX is running. Close it and try again.".to_owned());
    }

    let listed = read_manifest(folder);
    let mut parents = Vec::new();
    for name in &listed {
        if name == UNINSTALLER {
            continue; // still running; removed afterwards
        }
        let Ok(relative) = payload_path(name) else {
            continue;
        };
        let path = folder.join(&relative);
        if let Err(error) = fs::remove_file(&path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            return Err(format!(
                "{} could not be removed: {error}\n\nIf VizX is running, close it and try again.",
                path.display()
            ));
        }
        let mut parent = relative.parent();
        while let Some(folder_inside) = parent.filter(|path| !path.as_os_str().is_empty()) {
            parents.push(folder_inside.to_path_buf());
            parent = folder_inside.parent();
        }
    }
    let _ = fs::remove_dir_all(folder.join(DATA_FOLDER));
    // Deepest first, so a folder is empty by the time it is reached.
    parents.sort_by(|a, b| {
        b.components()
            .count()
            .cmp(&a.components().count())
            .then_with(|| a.cmp(b))
    });
    parents.dedup();
    for parent in parents {
        let _ = fs::remove_dir(folder.join(parent)); // only if empty
    }
    let _ = fs::remove_file(folder.join(MANIFEST));
    remove_shortcuts(folder);
    unregister(folder);

    // Once this program has closed: remove it, then the folder if it is
    // empty (`rmdir` without /s never removes anything else).
    let uninstaller = folder.join(UNINSTALLER);
    let script = format!(
        "ping -n 3 127.0.0.1 >nul & del /f /q \"{}\" & rmdir \"{}\"",
        uninstaller.display(),
        folder.display()
    );
    let mut command = hidden("cmd.exe");
    #[cfg(windows)]
    {
        // cmd.exe has quoting rules of its own: the line goes to it as it
        // is, and /s makes it drop just the outer quotes.
        use std::os::windows::process::CommandExt;
        command.raw_arg(format!("/d /s /c \"{script}\""));
    }
    let _ = command.spawn();
    Ok(())
}

/// A manifest line as a path inside the folder (plain names only).
fn payload_path(name: &str) -> Result<PathBuf, ()> {
    let path = PathBuf::from(name);
    let is_plain = !name.is_empty()
        && path
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)));
    if is_plain { Ok(path) } else { Err(()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vizx_gets_a_folder_of_its_own() {
        assert_eq!(
            folder_for_choice(Path::new(r"D:\Programs")),
            PathBuf::from(r"D:\Programs\VizX")
        );
        assert_eq!(
            folder_for_choice(Path::new(r"D:\Apps\vizx")),
            PathBuf::from(r"D:\Apps\vizx")
        );
    }

    #[test]
    fn only_new_empty_or_vizx_folders_are_accepted() {
        let root = std::env::temp_dir().join(format!("vizx-setup-check-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();

        assert!(check_folder(&root.join("new")).is_ok());
        assert!(check_folder(&root).is_ok()); // empty
        assert!(can_write(&root.join("new")));
        fs::write(root.join("letter.txt"), "mine").unwrap();
        assert!(check_folder(&root).is_err());
        fs::write(root.join(MANIFEST), "vizx.exe").unwrap();
        assert!(check_folder(&root).is_ok()); // an earlier VizX
        assert!(check_folder(Path::new("relative\\VizX")).is_err());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn manifest_lines_never_point_outside_the_folder() {
        assert!(payload_path("presets/a.milk").is_ok());
        assert!(payload_path("../../Windows/x").is_err());
        assert!(payload_path(r"C:\Windows\x").is_err());
    }

    #[test]
    fn options_survive_the_trip_through_the_command_line() {
        let options = Options {
            folder: PathBuf::from(r"C:\Program Files\VizX"),
            scope: Scope::AllUsers,
            desktop_shortcut: false,
            start_menu_shortcut: true,
        };
        // Split like Windows splits a command line with quotes.
        let line = options.elevated_arguments();
        let mut arguments = Vec::new();
        let mut current = String::new();
        let mut is_quoted = false;
        for character in line.chars() {
            match character {
                '"' => is_quoted = !is_quoted,
                ' ' if !is_quoted => arguments.push(std::mem::take(&mut current)),
                _ => current.push(character),
            }
        }
        arguments.push(current);
        assert!(arguments.contains(&"--elevated".to_owned()));

        let back = Options::from_arguments(&arguments);
        assert_eq!(back.folder, options.folder);
        assert_eq!(back.scope, Scope::AllUsers);
        assert!(!back.desktop_shortcut && back.start_menu_shortcut);

        let silent = Options::from_arguments(&["--all-users".to_owned()]);
        assert_eq!(silent.scope, Scope::AllUsers);
        assert_eq!(silent.folder, default_folder(Scope::AllUsers));
    }

    #[test]
    fn folders_compare_without_case_or_trailing_slash() {
        assert!(same_folder(
            Path::new(r"C:\Program Files\VizX\"),
            Path::new(r"c:\program files\vizx")
        ));
        assert!(!same_folder(Path::new(r"D:\VizX"), Path::new(r"D:\VizX2")));
    }
}
