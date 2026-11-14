//! Extensions: optional device support installed from GitHub.
//!
//! An extension is a folder in `~/.local/share/settings/extensions/<id>` with an
//! `extension.toml` (at its root or one folder down) and a helper program. The
//! helper describes its pages as JSON and Settings draws them with its own rows,
//! so extensions look native and can be written in any language.
//! See `protocol.rs` for what the helper prints.

pub mod manage;
pub mod media;
pub mod pictures;
pub mod protocol;
pub mod render;

use crate::paths;
use anyhow::{Context, Result, bail};
use protocol::PageInfo;
use serde::Deserialize;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub const MANIFEST: &str = "extension.toml";

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    /// Existing Settings sidebar group; omitted extensions stay under Hardware.
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub description: String,
    /// The helper, relative to the manifest's folder (or a program on PATH).
    pub exec: String,
    /// Put before every command, e.g. `["--extension"]` for an app that also has a window.
    #[serde(default)]
    pub args: Vec<String>,
    /// Programs that must be on PATH for the extension to work.
    #[serde(default)]
    pub needs: Vec<String>,
    /// How to get what `needs` lists, shown when something is missing (markup).
    #[serde(default)]
    pub needs_hint: String,
    /// Run after install and update, from the manifest's folder (builds or fetches the helper).
    #[serde(default)]
    pub install: String,
    /// Events the helper wants: `theme-changed`.
    #[serde(default)]
    pub hooks: Vec<String>,
    /// Config files for the page's "Open config" button, `~` allowed.
    #[serde(default)]
    pub files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Extension {
    pub manifest: Manifest,
    /// The installed folder (`extensions/<id>`).
    pub root: PathBuf,
    /// Where `extension.toml` is.
    pub dir: PathBuf,
}

impl Extension {
    pub fn id(&self) -> &str {
        &self.manifest.id
    }

    pub fn exec(&self) -> PathBuf {
        let e = &self.manifest.exec;
        if e.contains('/') { self.dir.join(e) } else { PathBuf::from(e) }
    }

    pub fn missing(&self) -> Vec<String> {
        self.manifest.needs.iter().filter(|p| !crate::cmd::present(p)).cloned().collect()
    }

    pub fn wants(&self, hook: &str) -> bool {
        self.manifest.hooks.iter().any(|h| h == hook)
    }

    pub fn files(&self) -> Vec<PathBuf> {
        self.manifest
            .files
            .iter()
            .map(|f| match f.strip_prefix("~/") {
                Some(rest) => paths::home().join(rest),
                None => PathBuf::from(f),
            })
            .collect()
    }

    /// Run the helper with a time limit; stdout on success, stderr as the error.
    pub fn call(&self, args: &[&str], timeout: Duration) -> Result<String> {
        let mut c = Command::new(self.exec());
        c.args(&self.manifest.args)
            .args(args)
            .current_dir(&self.dir)
            .env("SETTINGS_EXTENSION_DIR", &self.dir)
            .env("SETTINGS_EXTENSION_STATE", paths::state_dir().join("extensions").join(self.id()))
            .env("SETTINGS_TEMP_UNIT", if crate::units::fahrenheit() { "F" } else { "C" })
            .env("SETTINGS_KINDS", protocol::KINDS);
        run_command(c, &self.manifest.name, timeout)
    }

    /// Ask which pages it has right now (none: no matching hardware).
    pub fn query_pages(&self) -> Result<Vec<PageInfo>> {
        if !self.missing().is_empty() {
            return Ok(vec![]);
        }
        protocol::parse_pages(&self.call(&["pages"], Duration::from_secs(10))?)
    }
}

/// Run a command with a time limit: stdout on success, stderr (or the exit status) as the error.
pub fn run_command(mut c: Command, what: &str, timeout: Duration) -> Result<String> {
    let mut child = c
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("couldn't start {}", c.get_program().to_string_lossy()))?;
    // Read both pipes on threads so a chatty child can't fill one and stall.
    let mut out = child.stdout.take().context("no stdout")?;
    let mut err = child.stderr.take().context("no stderr")?;
    let out_t = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = out.read_to_string(&mut s);
        s
    });
    let err_t = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = err.read_to_string(&mut s);
        s
    });
    let start = Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait()? {
            break s;
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            bail!("{what} didn't answer in {} s", timeout.as_secs().max(1));
        }
        std::thread::sleep(Duration::from_millis(15));
    };
    let stdout = out_t.join().unwrap_or_default();
    let stderr = err_t.join().unwrap_or_default();
    if !status.success() {
        // The last few lines say what went wrong; the rest is build noise.
        let lines: Vec<&str> = stderr.trim().lines().collect();
        let tail = lines[lines.len().saturating_sub(4)..].join("\n");
        bail!("{}", if tail.is_empty() { format!("{what} failed ({status})") } else { tail });
    }
    Ok(stdout)
}

/// Find `extension.toml` in a folder or one level down (a sparse checkout of a bigger repo).
pub fn find_manifest(root: &Path) -> Option<PathBuf> {
    find_within(root, 3)
}

/// The manifest at `dir`, or in a folder under it up to `depth` levels down
/// (an extension in a shared repository: `extensions/<id>`). Build output and
/// hidden folders are skipped.
fn find_within(dir: &Path, depth: u32) -> Option<PathBuf> {
    let here = dir.join(MANIFEST);
    if here.is_file() {
        return Some(here);
    }
    if depth == 0 {
        return None;
    }
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_dir()
                && !p.file_name().is_some_and(|n| {
                    let n = n.to_string_lossy();
                    n.starts_with('.') || ["target", "bin", "vendor", "node_modules"].contains(&n.as_ref())
                })
        })
        .collect();
    dirs.sort();
    dirs.into_iter().find_map(|d| find_within(&d, depth - 1))
}

pub fn load(root: &Path) -> Result<Extension> {
    let path = find_manifest(root).with_context(|| format!("no {MANIFEST} in {}", paths::pretty(root)))?;
    let text = std::fs::read_to_string(&path)?;
    let manifest: Manifest = toml::from_str(&text).with_context(|| format!("{} is not valid", paths::pretty(&path)))?;
    if manifest.id.is_empty() || !manifest.id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        bail!("{}: id must be letters, digits, - or _", paths::pretty(&path));
    }
    let dir = path.parent().map(Path::to_path_buf).unwrap_or_else(|| root.to_path_buf());
    Ok(Extension { manifest, root: root.to_path_buf(), dir })
}

/// Every installed extension, sorted by name. Broken ones are skipped.
pub fn installed() -> Vec<Extension> {
    let Ok(rd) = std::fs::read_dir(paths::ext_dir()) else { return vec![] };
    let mut out: Vec<Extension> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir() && !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')))
        .filter_map(|p| match load(&p) {
            Ok(e) => Some(e),
            Err(err) => {
                eprintln!("nyx-settings: extension skipped: {err:#}");
                None
            }
        })
        .collect();
    out.sort_by_key(|e| e.manifest.name.to_lowercase());
    out
}

/// Whether an installed extension is turned on (Settings → Extensions).
pub fn enabled(id: &str) -> bool {
    !crate::prefs::read().disabled_extensions.iter().any(|d| d == id)
}

/// Installed extensions that are turned on.
pub fn active() -> Vec<Extension> {
    let off = crate::prefs::read().disabled_extensions;
    installed().into_iter().filter(|e| !off.iter().any(|d| d == e.id())).collect()
}

pub fn find(id: &str) -> Option<Extension> {
    installed().into_iter().find(|e| e.id() == id)
}

// ----- Remembered pages -----

fn cache_file(id: &str) -> PathBuf {
    paths::ext_cache_dir().join(format!("{id}.pages.json"))
}

pub fn cached_pages(id: &str) -> Option<Vec<PageInfo>> {
    std::fs::read_to_string(cache_file(id)).ok().and_then(|t| protocol::parse_pages(&t).ok())
}

/// Store what `pages` printed; returns whether it changed.
pub fn remember_pages(id: &str, pages: &[PageInfo]) -> bool {
    if cached_pages(id).as_deref() == Some(pages) {
        return false;
    }
    let json = serde_json::Value::Array(
        pages
            .iter()
            .map(|p| {
                serde_json::json!({"id": p.id, "title": p.title, "icon": p.icon, "description": p.description, "keywords": p.keywords})
            })
            .collect(),
    );
    let _ = crate::cmd::atomic_write(&cache_file(id), json.to_string());
    true
}

pub fn forget_pages(id: &str) {
    let _ = std::fs::remove_file(cache_file(id));
}

/// An extension page in the sidebar.
#[derive(Debug, Clone)]
pub struct PageRef {
    pub ext: Extension,
    pub page: PageInfo,
}

impl PageRef {
    /// The sidebar id: `<extension>.<page>`, so extensions can't clash with each other or built-in pages.
    pub fn section_id(&self) -> String {
        format!("{}.{}", self.ext.id(), self.page.id)
    }
}

/// Pages of every installed extension: remembered ones when we have them (fast),
/// else asked for now. `fresh` always asks.
pub fn all_pages(fresh: bool) -> Vec<PageRef> {
    let mut out = Vec::new();
    for ext in active() {
        let pages = match (fresh, cached_pages(ext.id())) {
            (false, Some(p)) => p,
            _ => {
                let p = ext.query_pages().unwrap_or_else(|e| {
                    eprintln!("nyx-settings: {}: {e:#}", ext.manifest.name);
                    vec![]
                });
                remember_pages(ext.id(), &p);
                p
            }
        };
        out.extend(pages.into_iter().map(|page| PageRef { ext: ext.clone(), page }));
    }
    out
}

/// Ask every extension again; true if any page list changed.
pub fn refresh_pages() -> bool {
    let mut changed = false;
    for ext in active() {
        if let Ok(p) = ext.query_pages() {
            changed |= remember_pages(ext.id(), &p);
        }
    }
    changed
}

/// Tell the extensions that asked about it that the Omarchy theme changed.
pub fn theme_changed() -> Result<()> {
    let mut errors = Vec::new();
    for ext in active().into_iter().filter(|e| e.wants("theme-changed") && e.missing().is_empty()) {
        if let Err(e) = ext.call(&["theme-changed"], Duration::from_secs(20)) {
            errors.push(format!("{}: {e:#}", ext.manifest.name));
        }
    }
    if errors.is_empty() { Ok(()) } else { bail!("{}", errors.join("; ")) }
}

pub fn any_wants(hook: &str) -> bool {
    active().iter().any(|e| e.wants(hook))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("settings-ext-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn write_ext(root: &Path, sub: &str, script: &str) {
        let dir = root.join(sub);
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        std::fs::write(
            dir.join(MANIFEST),
            "id = \"fake\"\nname = \"Fake\"\nexec = \"bin/fake\"\nargs = [\"--ext\"]\nhooks = [\"theme-changed\"]\nfiles = [\"~/.config/fake.toml\"]\n",
        )
        .unwrap();
        let exe = dir.join("bin/fake");
        std::fs::write(&exe, script).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn loads_from_root_or_one_level_down() {
        let root = scratch("load");
        write_ext(&root.join("a"), "", "#!/bin/sh\n");
        write_ext(&root.join("b"), "extension", "#!/bin/sh\n");
        let a = load(&root.join("a")).unwrap();
        let b = load(&root.join("b")).unwrap();
        assert!(a.manifest.category.is_empty());
        let manifest = root.join("a").join(MANIFEST);
        let text = std::fs::read_to_string(&manifest).unwrap();
        std::fs::write(&manifest, format!("{text}category = \"Desktop\"\n")).unwrap();
        assert_eq!(load(&root.join("a")).unwrap().manifest.category, "Desktop");
        assert_eq!(a.dir, root.join("a"));
        assert_eq!(b.dir, root.join("b/extension"));
        assert_eq!(b.exec(), root.join("b/extension/bin/fake"));
        assert!(b.wants("theme-changed"));
        assert_eq!(b.files(), [paths::home().join(".config/fake.toml")]);
        assert!(load(&root.join("missing")).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn rejects_odd_ids() {
        let root = scratch("ids");
        std::fs::write(root.join(MANIFEST), "id = \"../x\"\nname = \"X\"\nexec = \"x\"\n").unwrap();
        assert!(load(&root).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn calls_the_helper() {
        let root = scratch("call");
        write_ext(
            &root,
            "",
            "#!/bin/sh\n[ \"$1\" = --ext ] || exit 9\nshift\ncase \"$1\" in\n pages) echo '[{\"id\":\"main\",\"title\":\"Fake\"}]' ;;\n fail) echo 'it broke' >&2; exit 3 ;;\n slow) sleep 5 ;;\n env) echo \"$SETTINGS_EXTENSION_DIR\" ;;\nesac\n",
        );
        let e = load(&root).unwrap();
        let pages = e.query_pages().unwrap();
        assert_eq!(pages[0].id, "main");
        assert_eq!(PageRef { ext: e.clone(), page: pages[0].clone() }.section_id(), "fake.main");
        assert_eq!(e.call(&["fail"], Duration::from_secs(5)).unwrap_err().to_string(), "it broke");
        assert!(e.call(&["slow"], Duration::from_millis(200)).unwrap_err().to_string().contains("didn't answer"));
        assert_eq!(e.call(&["env"], Duration::from_secs(5)).unwrap().trim(), root.to_string_lossy());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_programs_mean_no_pages() {
        let root = scratch("needs");
        write_ext(&root, "", "#!/bin/sh\necho '[{\"id\":\"main\",\"title\":\"Fake\"}]'\n");
        let mut e = load(&root).unwrap();
        e.manifest.needs = vec!["definitely-not-a-program-xyz".into()];
        assert_eq!(e.missing(), ["definitely-not-a-program-xyz"]);
        assert!(e.query_pages().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }
}
