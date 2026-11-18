//! The game files installer: bringing the player's own copy of a game's files into the app, for every engine that
//! runs on them, as each engine says ([`engine_core::GameFiles`]: where the player puts them, how they are known,
//! where they are kept, what to tell the player). The menu's "Game data" pages show it (`super::pages`).
//!
//! The headset can't convert a disc. The player puts the files (a folder, or a zip of one) in
//! `Download/<the engine's drop>` on the headset (over USB, or any file app); the import moves a folder in (no second
//! copy when the storage allows it, else copies) or unzips a zip into the app's files, with the folders found beside
//! them. The next launch starts on that engine; one carried already can be played at once. The scripts in
//! `scripts/quest` do the same over adb.
//!
//! On Quest, reading `Download` needs "All files access" for the app; the import asks for it (see
//! [`crate::set_storage_hooks`]).
use crate::engines::Engine;
use bevy::prelude::*;
use engine_core::GameFiles;
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Where a copy of the files can be found.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Source {
    /// The folder they are in (the folders beside it, if any, come too).
    Folder(PathBuf),
    /// A zip with them inside.
    Zip(PathBuf),
}

impl Source {
    pub(crate) fn describe(&self) -> String {
        let (kind, path) = match self {
            Source::Folder(p) => ("folder", p),
            Source::Zip(p) => ("zip", p),
        };
        format!("{kind} {}", path.display())
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) enum Status {
    #[default]
    Idle,
    /// Last scan's finds (empty: nothing there).
    Scanned(Vec<Source>),
    Working { what: String, done: u64, total: u64 },
    Done(Result<String, String>),
}

/// The engines whose game files the player brings, in the order the menu lists them ([`offer`]).
#[derive(Resource, Default, Clone)]
pub(crate) struct Offered(pub Vec<(Engine, GameFiles)>);

impl Offered {
    pub(crate) fn of(&self, engine: Engine) -> Option<&GameFiles> {
        self.0.iter().find(|(e, _)| *e == engine).map(|(_, f)| f)
    }
}

/// The installer offers `engine`'s game files.
pub(crate) fn offer(app: &mut App, engine: Engine, files: GameFiles) {
    let mut offered = app.world_mut().get_resource_or_init::<Offered>();
    if offered.of(engine).is_none() {
        offered.0.push((engine, files));
    }
}

/// Each engine's import, shared with its worker thread.
#[derive(Resource, Clone, Default)]
pub(crate) struct Import {
    status: Arc<Mutex<HashMap<Engine, Status>>>,
    /// The app's files directory (where the game files are kept).
    pub files: PathBuf,
}

impl Import {
    #[cfg(test)]
    pub(crate) fn new(files: PathBuf) -> Self {
        Self { files, ..default() }
    }

    pub(crate) fn status(&self, engine: Engine) -> Status {
        self.status.lock().ok().and_then(|s| s.get(&engine).cloned()).unwrap_or_default()
    }

    fn set(&self, engine: Engine, status: Status) {
        if let Ok(mut s) = self.status.lock() {
            s.insert(engine, status);
        }
    }

    /// An import is running (one at a time).
    pub(crate) fn working(&self) -> bool {
        self.status.lock().is_ok_and(|s| s.values().any(|s| matches!(s, Status::Working { .. })))
    }

    /// The files are kept in the app.
    pub(crate) fn installed(&self, files: &GameFiles) -> bool {
        files.in_folder(&files.kept_in(&self.files))
    }

    pub(crate) fn scan(&self, engine: Engine, files: &GameFiles) {
        let found = scan_dirs(files.drop).iter().flat_map(|d| scan(d, files)).collect();
        self.set(engine, Status::Scanned(found));
    }

    /// Move / unzip `source` into the app's files on a worker thread.
    pub(crate) fn start(&self, engine: Engine, files: GameFiles, source: Source) {
        if self.working() {
            return;
        }
        self.set(engine, Status::Working { what: "Starting".into(), done: 0, total: 0 });
        let this = self.clone();
        std::thread::Builder::new()
            .name("game-files-import".into())
            .spawn(move || {
                let result = this.run(engine, &files, &source);
                match &result {
                    Ok(msg) => info!("IMPORT {} done: {msg}", engine.name()),
                    Err(e) => warn!("IMPORT {} failed: {e}", engine.name()),
                }
                this.set(engine, Status::Done(result));
            })
            .map_err(|e| self.set(engine, Status::Done(Err(e.to_string()))))
            .ok();
    }

    fn run(&self, engine: Engine, files: &GameFiles, source: &Source) -> Result<String, String> {
        info!("IMPORT {} from {}", engine.name(), source.describe());
        let kept = files.kept_in(&self.files);
        // (into a folder of its own first, swapped in once whole: until then the files are as they were, and an
        // import cut short is never taken for them)
        let staged = self.files.join(format!(".importing-{}", files.folder));
        if staged.exists() {
            fs::remove_dir_all(&staged).map_err(|e| format!("{}: {e}", staged.display()))?;
        }
        let progress = |what: &str, done: u64, total: u64| {
            self.set(engine, Status::Working { what: what.to_string(), done, total });
        };
        match source {
            // (a folder is moved in when the storage allows: one that isn't theirs is left where it is)
            Source::Folder(from) if !files.in_folder(from) => return Err(format!("no {} in it", files.marker)),
            Source::Folder(from) => {
                replace_dir(from, &staged, &progress).map_err(|e| format!("{}: {e}", files.folder))?;
                for (name, to) in files.beside {
                    if let Some(beside) = from.parent().map(|p| p.join(name)).filter(|p| p.is_dir()) {
                        replace_dir(&beside, &self.files.join(to), &progress).map_err(|e| format!("{name}: {e}"))?;
                    }
                }
            }
            Source::Zip(zip) => unzip(zip, files, &staged, &self.files, &progress)?,
        }
        if !files.in_folder(&staged) {
            let _ = fs::remove_dir_all(&staged);
            return Err(format!("no {} in it", files.marker));
        }
        if kept.exists() {
            fs::remove_dir_all(&kept).map_err(|e| format!("{}: {e}", kept.display()))?;
        }
        fs::rename(&staged, &kept).map_err(|e| format!("{}: {e}", kept.display()))?;
        // (the next launch starts on it)
        let choice = crate::engines::engine_file(&self.files);
        fs::write(&choice, engine.key()).map_err(|e| format!("{}: {e}", choice.display()))?;
        Ok(format!("{} files imported.", engine.name()))
    }
}

pub(crate) struct ImportPlugin;

impl Plugin for ImportPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Offered>().add_systems(PreStartup, |files: Res<crate::xr::XrFiles>, mut commands: Commands| {
            commands.insert_resource(Import { files: files.0.clone(), ..default() });
        });
    }
}

/// Where to look: `Download/<drop>` (and `<drop>`) on the headset, `~/Downloads/<drop>` elsewhere, or
/// `SK8_IMPORT_DIR/<drop>`.
fn scan_dirs(drop: &str) -> Vec<PathBuf> {
    if let Some(dir) = std::env::var_os("SK8_IMPORT_DIR") {
        return vec![Path::new(&dir).join(drop)];
    }
    if cfg!(target_os = "android") {
        vec![Path::new("/sdcard/Download").join(drop), Path::new("/sdcard").join(drop)]
    } else {
        std::env::var_os("HOME").map(|h| vec![Path::new(&h).join("Downloads").join(drop)]).unwrap_or_default()
    }
}

/// Copies of the files in `dir`: folders holding them at most two levels down, and zips there.
pub(crate) fn scan(dir: &Path, files: &GameFiles) -> Vec<Source> {
    let mut found = Vec::new();
    let visit = |d: &Path, depth: u32, found: &mut Vec<Source>| -> Vec<PathBuf> {
        if files.in_folder(d) {
            found.push(Source::Folder(d.to_path_buf()));
            return Vec::new();
        }
        let mut next = Vec::new();
        for entry in fs::read_dir(d).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() && depth < 2 {
                next.push(path);
            } else if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip")) {
                found.push(Source::Zip(path));
            }
        }
        next
    };
    let mut level = vec![dir.to_path_buf()];
    for depth in 0..=2 {
        let mut next = Vec::new();
        for d in &level {
            next.extend(visit(d, depth, &mut found));
        }
        level = next;
    }
    found.sort_by_key(|s| matches!(s, Source::Zip(_)));
    found
}

/// Put `from` at `to` (replacing it): a rename when the storage allows,
/// else a copy.
fn replace_dir(from: &Path, to: &Path, progress: &dyn Fn(&str, u64, u64)) -> io::Result<()> {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)?;
    }
    if to.exists() {
        fs::remove_dir_all(to)?;
    }
    if fs::rename(from, to).is_ok() {
        return Ok(());
    }
    let files = list_files(from)?;
    let total: u64 = files.iter().map(|(_, len)| len).sum();
    let mut done = 0;
    for (path, len) in files {
        let rel = path.strip_prefix(from).map_err(io::Error::other)?;
        let out = to.join(rel);
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(&path, &out)?;
        done += len;
        progress(&format!("Copying {}", rel.display()), done, total);
    }
    Ok(())
}

fn list_files(dir: &Path) -> io::Result<Vec<(PathBuf, u64)>> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in fs::read_dir(&d)? {
            let entry = entry?;
            let meta = entry.metadata()?;
            if meta.is_dir() {
                stack.push(entry.path());
            } else {
                out.push((entry.path(), meta.len()));
            }
        }
    }
    Ok(out)
}

/// The files out of `zip`: what is under the files' prefix to `to`, and the folders beside that prefix to theirs under
/// `app_files`.
fn unzip(zip: &Path, files: &GameFiles, to: &Path, app_files: &Path, progress: &dyn Fn(&str, u64, u64)) -> Result<(), String> {
    let file = fs::File::open(zip).map_err(|e| format!("{}: {e}", zip.display()))?;
    let mut archive = zip::ZipArchive::new(io::BufReader::new(file)).map_err(|e| format!("{}: {e}", zip.display()))?;
    let names: Vec<String> = archive.file_names().map(str::to_string).collect();
    let prefix = files
        .prefix_in(names.iter().map(String::as_str))
        .ok_or_else(|| format!("{} has no {}", zip.display(), files.marker))?;
    // (the folders beside the files' folder: under the same parent)
    let parent = prefix.trim_end_matches('/').rsplit_once('/').map_or(String::new(), |(p, _)| format!("{p}/"));
    let mut roots = vec![(prefix, to.to_path_buf())];
    roots.extend(files.beside.iter().map(|(name, to)| (format!("{parent}{name}/"), app_files.join(to))));
    for (_, dir) in &roots {
        if dir.exists() {
            fs::remove_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
    }
    let total: u64 = (0..archive.len()).filter_map(|i| archive.by_index_raw(i).ok().map(|f| f.size())).sum();
    let mut done = 0;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        let name = entry.name().replace('\\', "/");
        let size = entry.size();
        // (the files' own prefix first: the longest match, as it may hold no folder named like one beside it)
        let Some((rel, root)) = roots.iter().find_map(|(p, root)| name.strip_prefix(p.as_str()).map(|rel| (rel.to_string(), root)))
        else {
            continue;
        };
        // No absolute paths or `..` out of the target.
        let safe = PathBuf::from(&rel);
        let inside = safe.components().all(|c| matches!(c, std::path::Component::Normal(_)));
        if entry.enclosed_name().is_none() || !inside || entry.is_dir() || rel.is_empty() {
            continue;
        }
        let out = root.join(safe);
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        let mut file = fs::File::create(&out).map_err(|e| format!("{}: {e}", out.display()))?;
        io::copy(&mut entry, &mut file).map_err(|e| format!("{}: {e}", out.display()))?;
        done += size;
        progress(&format!("Unzipping {rel}"), done, total);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const SKATE3: GameFiles = GameFiles {
        drop: "Skate3",
        folder: "assets",
        marker: "private/game.json",
        beside: &[("audio-pcm-cache", "Skate3RustEngine/audio-pcm-cache")],
        how: &[],
    };
    const THUG2: GameFiles = GameFiles { drop: "THUG2", folder: "thug2", marker: "Data", beside: &[], how: &[] };

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("game-files-import-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn scans_folders_and_zips() {
        let dir = temp("scan");
        fs::create_dir_all(dir.join("quest-data/assets/private")).unwrap();
        fs::write(dir.join("quest-data/assets/private/game.json"), b"{}").unwrap();
        fs::create_dir_all(dir.join("quest-data/audio-pcm-cache")).unwrap();
        fs::write(dir.join("quest-data/audio-pcm-cache/a.v3.pcm"), b"pcm").unwrap();
        fs::write(dir.join("bundle.zip"), b"").unwrap();
        let found = scan(&dir, &SKATE3);
        assert_eq!(found, vec![Source::Folder(dir.join("quest-data/assets")), Source::Zip(dir.join("bundle.zip"))]);
        // Moving it in, the sound banks beside it too; the next launch starts on it.
        let files = dir.join("files");
        let import = Import { files: files.clone(), ..default() };
        assert!(!import.installed(&SKATE3));
        import.run(Engine::Skate3, &SKATE3, &found[0]).unwrap();
        assert!(import.installed(&SKATE3));
        assert_eq!(fs::read(files.join("Skate3RustEngine/audio-pcm-cache/a.v3.pcm")).unwrap(), b"pcm");
        assert_eq!(fs::read_to_string(files.join("engine.txt")).unwrap(), "skate3");
        let _ = fs::remove_dir_all(&dir);
    }

    /// A zip of a game folder in any case: what holds its marker kept, what doesn't left out, nothing out of the target.
    #[test]
    fn unzips_what_is_the_games() {
        let dir = temp("zip");
        let zip_path = dir.join("THUG2.zip");
        {
            let mut zip = zip::ZipWriter::new(fs::File::create(&zip_path).unwrap());
            let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
            for (name, data) in [("THUG2/DATA/pre/qb.prx", "qb"), ("THUG2/thug2.exe", "exe"), ("other/readme.txt", "x"), ("THUG2/../evil", "no")] {
                zip.start_file(name, options).unwrap();
                zip.write_all(data.as_bytes()).unwrap();
            }
            zip.finish().unwrap();
        }
        assert_eq!(scan(&dir, &THUG2), vec![Source::Zip(zip_path.clone())]);
        let files = dir.join("files");
        let import = Import { files: files.clone(), ..default() };
        import.run(Engine::Thug2, &THUG2, &Source::Zip(zip_path)).unwrap();
        assert!(import.installed(&THUG2));
        assert_eq!(fs::read_to_string(files.join("thug2/DATA/pre/qb.prx")).unwrap(), "qb");
        assert!(files.join("thug2/thug2.exe").is_file() && !files.join("thug2/readme.txt").exists());
        assert!(!files.join("evil").exists() && !dir.join("evil").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    /// Files are taken for the game's only once whole: one without the marker (or cut short: its folder of its own
    /// left over) leaves those there as they were.
    #[test]
    fn an_import_counts_only_once_whole() {
        let dir = temp("whole");
        let files = dir.join("files");
        fs::create_dir_all(files.join("thug2/Data")).unwrap();
        fs::write(files.join("thug2/Data/old"), b"old").unwrap();
        // (one cut short before: never taken for the files)
        fs::create_dir_all(files.join(".importing-thug2/Data")).unwrap();
        fs::create_dir_all(dir.join("drop/NotTheGame")).unwrap();
        fs::write(dir.join("drop/NotTheGame/x"), b"x").unwrap();
        let import = Import { files: files.clone(), ..default() };
        assert!(import.run(Engine::Thug2, &THUG2, &Source::Folder(dir.join("drop/NotTheGame"))).is_err());
        assert_eq!(fs::read(files.join("thug2/Data/old")).unwrap(), b"old");
        assert!(dir.join("drop/NotTheGame/x").is_file(), "left where it was");
        assert!(!files.join(".importing-thug2").exists(), "the one cut short cleared");
        let _ = fs::remove_dir_all(&dir);
    }
}
