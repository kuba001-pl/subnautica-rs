//! Filesystem edge for M9f. Both the dedicated and embedded server use this.
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use sn_session::{Server, ServerConfig};

pub const AUTOSAVE_US: u64 = 30_000_000;

pub struct SaveStore {
    dir: PathBuf,
    // OS releases the lock even after a crash; the lock file may remain.
    _lock: File,
}

pub fn default_dir() -> Result<PathBuf, String> {
    let env_path = |key| std::env::var_os(key).map(PathBuf::from);
    let base = if cfg!(target_os = "windows") {
        env_path("LOCALAPPDATA").ok_or("LOCALAPPDATA is unset; supply --save-dir")?
    } else if cfg!(target_os = "macos") {
        env_path("HOME")
            .ok_or("HOME is unset; supply --save-dir")?
            .join("Library/Application Support")
    } else {
        env_path("XDG_DATA_HOME")
            .filter(|p| p.is_absolute())
            .or_else(|| env_path("HOME").map(|p| p.join(".local/share")))
            .ok_or("HOME/XDG_DATA_HOME unset; supply --save-dir")?
    };
    Ok(base.join("subnautica-rs/saves/default"))
}

fn resolved(path: &Path) -> Result<PathBuf, String> {
    let absolute = std::path::absolute(path).map_err(|e| e.to_string())?;
    if absolute.components().any(|c| c == Component::ParentDir) {
        return Err("save path must not contain parent traversal (..)".into());
    }
    if absolute.exists() {
        return absolute.canonicalize().map_err(|e| e.to_string());
    }
    let parent = absolute.parent().ok_or("save path has no parent")?;
    let name = absolute
        .file_name()
        .ok_or("save path has no directory name")?;
    Ok(resolved(parent)?.join(name))
}

impl SaveStore {
    pub fn open(dir: PathBuf, game_root: &Path) -> Result<Self, String> {
        let dir = resolved(&dir)?;
        let game = resolved(game_root)?;
        if dir.starts_with(&game)
            || dir
                .components()
                .any(|c| c.as_os_str().to_string_lossy().eq_ignore_ascii_case("out"))
        {
            return Err("save directory must be outside the game install and out/".into());
        }
        fs::create_dir_all(&dir).map_err(|e| format!("create save directory: {e}"))?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(dir.join("server.lock"))
            .map_err(|e| format!("save lock: {e}"))?;
        lock.try_lock()
            .map_err(|e| format!("save slot is already in use or cannot be locked: {e}"))?;
        Ok(Self { dir, _lock: lock })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn read(path: &Path) -> Result<Option<Vec<u8>>, String> {
        let file = match File::open(path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(format!("read {}: {e}", path.display())),
        };
        let mut bytes = Vec::new();
        file.take(sn_session::save::MAX_SAVE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("read {}: {e}", path.display()))?;
        Ok(Some(bytes))
    }

    /// Explicit backup recovery is a read-only choice; never silently reset.
    pub fn load(&self, config: ServerConfig, now_us: u64, backup: bool) -> Result<Server, String> {
        let path = self
            .dir
            .join(if backup { "world.sav.bak" } else { "world.sav" });
        match Self::read(&path)? {
            Some(bytes) => Server::from_save(config, &bytes, now_us).map_err(|e| {
                format!(
                    "{}: {e}; save retained; recover with --load-backup",
                    path.display()
                )
            }),
            None if backup => Err("no backup save exists".into()),
            None => {
                if self.dir.join("world.sav.bak").exists()
                    || self.dir.join("world.sav.tmp").exists()
                {
                    return Err("primary save missing but backup/temp exists; recover explicitly with --load-backup".into());
                }
                Ok(Server::new(config, now_us))
            }
        }
    }

    /// Validate before replacing. The backup is always a previously valid save.
    /// If `recovering`, preserve the good backup while replacing the bad primary.
    pub fn write(
        &self,
        server: &Server,
        config: &ServerConfig,
        now_us: u64,
        recovering: bool,
    ) -> Result<String, String> {
        let primary = self.dir.join("world.sav");
        let backup = self.dir.join("world.sav.bak");
        let temp = self.dir.join("world.sav.tmp");
        let bytes = server.save_bytes(now_us);
        Server::from_save(config.clone(), &bytes, 0)
            .map_err(|e| format!("refuse invalid new save: {e}"))?;
        if !recovering && let Some(old) = Self::read(&primary)? {
            Server::from_save(config.clone(), &old, 0)
                .map_err(|e| format!("existing save changed/corrupted: {e}; files retained"))?;
            let mut b = File::create(&backup).map_err(|e| format!("backup: {e}"))?;
            b.write_all(&old)
                .and_then(|()| b.sync_all())
                .map_err(|e| format!("backup flush: {e}"))?;
        }
        let mut f = File::create(&temp).map_err(|e| format!("save temp: {e}"))?;
        f.write_all(&bytes)
            .and_then(|()| f.sync_all())
            .map_err(|e| format!("save flush: {e}"))?;
        drop(f);
        // Windows rename cannot replace an existing file. Backup is durable
        // before removal; a crash in this gap requires explicit backup recovery.
        if primary.exists() {
            fs::remove_file(&primary).map_err(|e| format!("replace save: {e}"))?;
        }
        fs::rename(&temp, &primary)
            .map_err(|e| format!("install save: {e}; backup/temp retained"))?;
        Ok(format!(
            "save: {} bytes, tick {}, item hash {:016x}, players {}, locker {}, removed {}, dropped {} at {}",
            bytes.len(),
            server.clock(now_us).0.0,
            sn_protocol::Fnv64::new()
                .bytes(&server.world().items.to_bytes())
                .finish(),
            server.world().items.players.len(),
            server.world().items.locker.len(),
            server.world().items.gone.len(),
            server.world().items.objects.len(),
            primary.display()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sn_protocol::{AuroraStart, WorldSettings};

    fn config() -> ServerConfig {
        ServerConfig {
            build: "synthetic".into(),
            fingerprint: 17,
            password: None,
            max_players: 8,
            settings: WorldSettings {
                slot_seed: Some(7),
                lifepod: None,
                aurora: AuroraStart::Held { exploded: false },
                clock_start: 123.,
                time_scale: 1.,
            },
            tick_us: 20_000,
            max_speed: 30.,
            rules: Default::default(),
        }
    }
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "sn-rs-save-test-{}-{}",
                std::process::id(),
                sn_net::random_seed()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn restart_backup_and_explicit_corruption_recovery() {
        let temp = Temp::new();
        let dir = temp.0.join("slot");
        let cfg = config();
        let store = SaveStore::open(dir.clone(), &temp.0.join("game")).unwrap();
        let s = store.load(cfg.clone(), 0, false).unwrap();
        store.write(&s, &cfg, 1_000_000, false).unwrap();
        store.write(&s, &cfg, 2_000_000, false).unwrap();
        assert_eq!(store.load(cfg.clone(), 0, false).unwrap().clock(0).0.0, 100);
        assert_eq!(store.load(cfg.clone(), 0, true).unwrap().clock(0).0.0, 50);
        fs::write(dir.join("world.sav"), b"broken").unwrap();
        assert!(store.load(cfg.clone(), 0, false).is_err());
        assert!(store.write(&s, &cfg, 3_000_000, false).is_err());
        let backup_before = fs::read(dir.join("world.sav.bak")).unwrap();
        let recovered = store.load(cfg.clone(), 0, true).unwrap();
        store.write(&recovered, &cfg, 0, true).unwrap();
        assert_eq!(fs::read(dir.join("world.sav.bak")).unwrap(), backup_before);
        drop(store);
        let reopened = SaveStore::open(dir, &temp.0.join("game")).unwrap();
        assert_eq!(reopened.load(cfg, 0, false).unwrap().clock(0).0.0, 50);
    }

    #[test]
    fn slot_exclusion_and_protected_paths() {
        let temp = Temp::new();
        let game = temp.0.join("game");
        let dir = temp.0.join("slot");
        let store = SaveStore::open(dir.clone(), &game).unwrap();
        assert!(SaveStore::open(dir.clone(), &game).is_err());
        assert!(SaveStore::open(game.join("saves"), &game).is_err());
        assert!(SaveStore::open(temp.0.join("out/slot"), &game).is_err());
        assert!(!game.exists());
        drop(store);
        assert!(SaveStore::open(dir, &game).is_ok());
    }
}
