//! Project storage with crash-safe autosave.
//!
//! A tiny key/value layer (files in the app's data folder on desktop, `localStorage` in the
//! browser) with the original Wobbleworks recovery scheme on top: every project has two slots,
//! a save writes the spare slot and only then flips the index to it, so a crash mid-save can
//! never destroy both. Slots missing from the index (a crash between the two writes) are found
//! and listed again on startup.

use serde::{Deserialize, Serialize};

use crate::project;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ProjectMeta {
    pub id: String,
    pub name: String,
    pub modified: f64,
    pub w: usize,
    pub h: usize,
    /// Which slot holds the last good save: "a" or "b".
    pub good: String,
    /// Small PNG thumbnail, base64.
    pub thumb: Option<String>,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Index {
    pub projects: Vec<ProjectMeta>,
}

/// Raw key/value storage.
trait Kv {
    fn get(&self, key: &str) -> Option<String>;
    fn set(&self, key: &str, value: &str) -> Result<(), String>;
    fn remove(&self, key: &str);
    fn keys(&self) -> Vec<String>;
}

#[cfg(not(target_arch = "wasm32"))]
mod backend {
    use std::path::PathBuf;

    pub struct Files {
        pub dir: PathBuf,
    }

    fn file_name(key: &str) -> String {
        // Keys are ours ("index", "p:<id>:a"); map them to safe file names.
        key.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '.' }).collect::<String>() + ".wobstore"
    }

    fn key_of(name: &str) -> Option<String> {
        let stem = name.strip_suffix(".wobstore")?;
        // "p.<id>.a" back to "p:<id>:a"; ids never contain dots.
        let parts: Vec<&str> = stem.split('.').collect();
        Some(match parts.as_slice() {
            ["p", id, slot] => format!("p:{id}:{slot}"),
            _ => stem.to_string(),
        })
    }

    impl super::Kv for Files {
        fn get(&self, key: &str) -> Option<String> {
            std::fs::read_to_string(self.dir.join(file_name(key))).ok()
        }

        fn set(&self, key: &str, value: &str) -> Result<(), String> {
            std::fs::create_dir_all(&self.dir).map_err(|e| format!("can't create {}: {e}", self.dir.display()))?;
            let path = self.dir.join(file_name(key));
            let tmp = path.with_extension("tmp");
            std::fs::write(&tmp, value).map_err(|e| format!("can't write {}: {e}", tmp.display()))?;
            std::fs::rename(&tmp, &path).map_err(|e| format!("can't save {}: {e}", path.display()))
        }

        fn remove(&self, key: &str) {
            let _ = std::fs::remove_file(self.dir.join(file_name(key)));
        }

        fn keys(&self) -> Vec<String> {
            let Ok(rd) = std::fs::read_dir(&self.dir) else { return Vec::new() };
            rd.filter_map(Result::ok).filter_map(|e| e.file_name().to_str().and_then(key_of)).collect()
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod backend {
    pub struct Local;

    const PREFIX: &str = "wobbleworks:";

    fn storage() -> Option<web_sys::Storage> {
        web_sys::window()?.local_storage().ok().flatten()
    }

    impl super::Kv for Local {
        fn get(&self, key: &str) -> Option<String> {
            storage()?.get_item(&format!("{PREFIX}{key}")).ok().flatten()
        }

        fn set(&self, key: &str, value: &str) -> Result<(), String> {
            let s = storage().ok_or("this browser has no local storage")?;
            s.set_item(&format!("{PREFIX}{key}"), value).map_err(|_| "browser storage is full: export a .wob to keep this drawing safe".to_string())
        }

        fn remove(&self, key: &str) {
            if let Some(s) = storage() {
                let _ = s.remove_item(&format!("{PREFIX}{key}"));
            }
        }

        fn keys(&self) -> Vec<String> {
            let Some(s) = storage() else { return Vec::new() };
            let n = s.length().unwrap_or(0);
            (0..n).filter_map(|i| s.key(i).ok().flatten()).filter_map(|k| k.strip_prefix(PREFIX).map(str::to_string)).collect()
        }
    }
}

pub struct Store {
    kv: Box<dyn Kv>,
}

/// In-memory storage, for tests.
#[cfg(test)]
#[derive(Default)]
pub struct Memory(std::sync::Mutex<std::collections::BTreeMap<String, String>>);

#[cfg(test)]
impl Kv for Memory {
    fn get(&self, key: &str) -> Option<String> {
        self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(key).cloned()
    }
    fn set(&self, key: &str, value: &str) -> Result<(), String> {
        self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(key.into(), value.into());
        Ok(())
    }
    fn remove(&self, key: &str) {
        self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner).remove(key);
    }
    fn keys(&self) -> Vec<String> {
        self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner).keys().cloned().collect()
    }
}

#[derive(Default, Serialize, Deserialize)]
struct Session {
    open: bool,
}

impl Store {
    /// The platform store (`None` if there's nowhere to save).
    pub fn open() -> Option<Store> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let dir = eframe::storage_dir("WobbleWorks")?.join("projects");
            Some(Store { kv: Box::new(backend::Files { dir }) })
        }
        #[cfg(target_arch = "wasm32")]
        {
            Some(Store { kv: Box::new(backend::Local) })
        }
    }

    #[cfg(test)]
    pub fn memory() -> Store {
        Store { kv: Box::new(Memory::default()) }
    }

    /// Where projects live, for the About box.
    pub fn location() -> String {
        #[cfg(not(target_arch = "wasm32"))]
        {
            eframe::storage_dir("WobbleWorks").map_or_else(|| "nowhere (no data folder)".into(), |d| d.join("projects").display().to_string())
        }
        #[cfg(target_arch = "wasm32")]
        {
            "this browser's local storage".into()
        }
    }

    pub fn index(&self) -> Index {
        self.kv.get("index").and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    fn write_index(&self, idx: &Index) -> Result<(), String> {
        let s = serde_json::to_string(idx).map_err(|e| e.to_string())?;
        self.kv.set("index", &s)
    }

    /// Projects, most recently changed first.
    pub fn list(&self) -> Vec<ProjectMeta> {
        let mut v = self.index().projects;
        v.sort_by(|a, b| b.modified.total_cmp(&a.modified));
        v
    }

    /// Save a project's JSON. Writes the spare slot, then flips the index to it.
    pub fn save(&self, mut meta: ProjectMeta, json: &str) -> Result<(), String> {
        let mut idx = self.index();
        let slot = match idx.projects.iter().find(|p| p.id == meta.id) {
            Some(p) if p.good == "a" => "b",
            _ => "a",
        };
        self.kv.set(&format!("p:{}:{slot}", meta.id), &project::pack(json)?)?;
        meta.good = slot.into();
        match idx.projects.iter_mut().find(|p| p.id == meta.id) {
            Some(p) => *p = meta,
            None => idx.projects.push(meta),
        }
        self.write_index(&idx)
    }

    /// Load a project's JSON, falling back to the other slot if the good one is unreadable.
    /// The flag says the backup slot was used.
    pub fn load(&self, id: &str) -> Result<(String, bool), String> {
        let idx = self.index();
        let good = idx.projects.iter().find(|p| p.id == id).map_or("a", |p| if p.good == "b" { "b" } else { "a" });
        let other = if good == "a" { "b" } else { "a" };
        let read = |slot: &str| -> Option<String> {
            let raw = self.kv.get(&format!("p:{id}:{slot}"))?;
            let text = project::unpack(&raw).ok()?;
            project::from_json(&text).is_ok().then_some(text)
        };
        if let Some(t) = read(good) {
            return Ok((t, false));
        }
        read(other).map(|t| (t, true)).ok_or_else(|| "both autosave slots for that project are unreadable".to_string())
    }

    pub fn delete(&self, id: &str) -> Result<(), String> {
        self.kv.remove(&format!("p:{id}:a"));
        self.kv.remove(&format!("p:{id}:b"));
        let mut idx = self.index();
        idx.projects.retain(|p| p.id != id);
        self.write_index(&idx)
    }

    /// Put slots that are on disk but missing from the index back into it. Returns how many.
    pub fn recover(&self) -> usize {
        let mut idx = self.index();
        let mut found: Vec<String> = Vec::new();
        for k in self.kv.keys() {
            let mut parts = k.split(':');
            if let (Some("p"), Some(id), Some("a" | "b"), None) = (parts.next(), parts.next(), parts.next(), parts.next())
                && !idx.projects.iter().any(|p| p.id == id)
                && !found.iter().any(|f| f == id)
            {
                found.push(id.to_string());
            }
        }
        let mut added = 0;
        for id in found {
            let mut best: Option<(project::Loaded, f64, &str)> = None;
            for slot in ["a", "b"] {
                let Some(raw) = self.kv.get(&format!("p:{id}:{slot}")) else { continue };
                let Ok(text) = project::unpack(&raw) else { continue };
                let modified = serde_json::from_str::<serde_json::Value>(&text).ok().and_then(|v| v.get("modified")?.as_f64()).unwrap_or(0.0);
                let Ok(l) = project::from_json(&text) else { continue };
                if best.as_ref().is_none_or(|b| modified > b.1) {
                    best = Some((l, modified, slot));
                }
            }
            if let Some((l, modified, slot)) = best {
                idx.projects.push(ProjectMeta {
                    id: id.clone(),
                    name: format!("{} (recovered)", l.name),
                    modified,
                    w: l.doc.w,
                    h: l.doc.h,
                    good: slot.into(),
                    thumb: None,
                });
                added += 1;
            }
        }
        if added > 0 {
            let _ = self.write_index(&idx);
        }
        added
    }

    /// Mark the session open; returns whether the last one ended without closing.
    pub fn begin_session(&self) -> bool {
        let crashed = self.kv.get("session").and_then(|s| serde_json::from_str::<Session>(&s).ok()).is_some_and(|s| s.open);
        let _ = self.kv.set("session", "{\"open\":true}");
        crashed
    }

    pub fn end_session(&self) {
        let _ = self.kv.set("session", "{\"open\":false}");
    }
}

/// A fresh project id.
pub fn new_id() -> String {
    let t = crate::platform::now_ms() as u64;
    let r = crate::brush::rnd((t as u32) ^ (crate::model::next_id() as u32).wrapping_mul(2_654_435_761));
    format!("{t:x}{:06x}", (r * 16_777_215.0) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Doc;

    fn json(name: &str, modified: f64) -> String {
        let f = project::to_file(&Doc::new(20, 20), name, Some("x"), modified, 1.0, &mut project::PngCache::default());
        project::to_json(&f).unwrap()
    }

    fn meta(id: &str) -> ProjectMeta {
        ProjectMeta { id: id.into(), name: "P".into(), ..Default::default() }
    }

    #[test]
    fn saves_alternate_slots_and_load_falls_back() {
        let s = Store::memory();
        s.save(meta("p1"), &json("one", 1.0)).unwrap();
        assert_eq!(s.index().projects[0].good, "a");
        s.save(meta("p1"), &json("two", 2.0)).unwrap();
        assert_eq!(s.index().projects[0].good, "b");
        let (t, backup) = s.load("p1").unwrap();
        assert!(!backup && t.contains("two"));
        // Corrupt the good slot: the other one still opens.
        s.kv.set("p:p1:b", "z:garbage").unwrap();
        let (t, backup) = s.load("p1").unwrap();
        assert!(backup && t.contains("one"));
        s.kv.set("p:p1:a", "nope").unwrap();
        assert!(s.load("p1").is_err());
    }

    #[test]
    fn unlisted_slots_are_recovered() {
        let s = Store::memory();
        s.save(meta("p1"), &json("kept", 1.0)).unwrap();
        s.kv.set("p:lost:a", &project::pack(&json("lost", 5.0)).unwrap()).unwrap();
        s.kv.set("p:junk:a", "z:@@@").unwrap();
        assert_eq!(s.recover(), 1);
        let l = s.list();
        assert_eq!(l[0].id, "lost");
        assert!(l[0].name.contains("recovered"));
        assert_eq!(s.recover(), 0);
        s.delete("lost").unwrap();
        assert_eq!(s.list().len(), 1);
    }

    #[test]
    fn session_flag_detects_crashes() {
        let s = Store::memory();
        assert!(!s.begin_session());
        assert!(s.begin_session());
        s.end_session();
        assert!(!s.begin_session());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn file_backend_round_trips() {
        let dir = std::env::temp_dir().join(format!("wobble-store-{}", std::process::id()));
        let s = Store { kv: Box::new(backend::Files { dir: dir.clone() }) };
        s.save(meta("abc"), &json("disk", 1.0)).unwrap();
        assert!(s.load("abc").unwrap().0.contains("disk"));
        assert!(s.kv.keys().iter().any(|k| k == "p:abc:a"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn ids_are_unique() {
        let a = new_id();
        let b = new_id();
        assert_ne!(a, b);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
