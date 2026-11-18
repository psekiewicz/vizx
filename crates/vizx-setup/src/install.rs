//! Installing VizX into a folder and removing it again.
//!
//! Everything happens for the current user only, so no administrator rights
//! are needed: the files go into a folder the user can write to (by default
//! `%LOCALAPPDATA%\Programs\VizX`), the shortcuts into the user's Start
//! menu and desktop, and the entry in "Apps & features" into
//! `HKEY_CURRENT_USER`.
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
/// Where "Apps & features" finds VizX.
const UNINSTALL_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\VizX";

/// What the user chose before installing.
#[derive(Debug, Clone)]
pub struct Options {
    pub folder: PathBuf,
    pub desktop_shortcut: bool,
    pub start_menu_shortcut: bool,
}

/// Where VizX goes unless the user chooses otherwise.
pub fn default_folder() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\VizX-apps"));
    base.join("Programs").join(APP_NAME)
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

/// The special folders (as .NET names them) that get a shortcut.
fn shortcut_places(options: &Options) -> Vec<&'static str> {
    let mut places = Vec::new();
    if options.start_menu_shortcut {
        places.push("Programs");
    }
    if options.desktop_shortcut {
        places.push("Desktop");
    }
    places
}

fn create_shortcuts(options: &Options) -> Result<(), String> {
    let places = shortcut_places(options);
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

/// Removes the shortcuts that start the VizX in `folder` (shortcuts of
/// another VizX installation stay).
fn remove_shortcuts(folder: &Path) {
    let target = folder.join(APP_EXECUTABLE);
    let script = format!(
        "$shell = New-Object -ComObject WScript.Shell\n\
         foreach ($place in 'Programs', 'Desktop') {{\n\
           $path = Join-Path ([Environment]::GetFolderPath($place)) '{APP_NAME}.lnk'\n\
           if ((Test-Path -LiteralPath $path) -and \
               $shell.CreateShortcut($path).TargetPath -eq {target}) {{\n\
             Remove-Item -LiteralPath $path -Force\n\
           }}\n\
         }}\n",
        target = powershell_quoted(&target.to_string_lossy()),
    );
    let _ = run_powershell(&script);
}

fn registry_add(name: &str, kind: &str, value: &str) -> Result<(), String> {
    let status = hidden("reg.exe")
        .args([
            "add",
            UNINSTALL_KEY,
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

/// Adds VizX to "Apps & features", where it can be uninstalled.
fn register(folder: &Path, size_bytes: u64) -> Result<(), String> {
    let uninstaller = folder.join(UNINSTALLER);
    let program = folder.join(APP_EXECUTABLE);
    registry_add("DisplayName", "REG_SZ", APP_NAME)?;
    registry_add("DisplayVersion", "REG_SZ", env!("CARGO_PKG_VERSION"))?;
    registry_add("Publisher", "REG_SZ", "VizX")?;
    registry_add("InstallLocation", "REG_SZ", &folder.to_string_lossy())?;
    registry_add("DisplayIcon", "REG_SZ", &program.to_string_lossy())?;
    registry_add(
        "UninstallString",
        "REG_SZ",
        &format!("\"{}\"", uninstaller.to_string_lossy()),
    )?;
    registry_add(
        "EstimatedSize",
        "REG_DWORD",
        &(size_bytes / 1024).min(u64::from(u32::MAX)).to_string(),
    )?;
    registry_add("NoModify", "REG_DWORD", "1")?;
    registry_add("NoRepair", "REG_DWORD", "1")
}

fn unregister() {
    let _ = hidden("reg.exe")
        .args(["delete", UNINSTALL_KEY, "/f"])
        .status();
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

    // VizX keeps its large files in here, next to itself.
    fs::create_dir_all(folder.join(DATA_FOLDER)).map_err(|error| error.to_string())?;
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
    register(folder, total_bytes)?;
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
/// program while it runs.
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
    unregister();

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
}
