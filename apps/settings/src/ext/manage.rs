//! The extension catalog, and installing, updating and removing extensions.
//! Blocking; the page runs these on a worker thread and `settings --ext` runs them directly.

use super::{Extension, load, run_command};
use crate::{cmd, paths};
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// The catalog on GitHub. `SETTINGS_EXT_INDEX` (a URL or a file) replaces it, for testing.
pub const INDEX_URL: &str = "https://raw.githubusercontent.com/design-nexus/nyx-settings/main/extensions/index.json";

/// Where extensions came from before they moved into the nyx repository.
const OLD_REPO: &str = "settings-extensions";

/// The catalog this version shipped with, used until (or if) GitHub can't be reached.
const BUILT_IN: &str = include_str!("../../data/extensions.json");

const GIT_TIMEOUT: Duration = Duration::from_secs(120);
/// Install scripts may build a Rust program.
const INSTALL_TIMEOUT: Duration = Duration::from_secs(900);

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Entry {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// What it works with, e.g. "SteelSeries, HyperX and Logitech headsets".
    #[serde(default)]
    pub devices: String,
    /// A git URL.
    pub repo: String,
    /// The extension's folder inside the repo, when it isn't the whole repo.
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub needs: Vec<String>,
    /// More folders of the repo it needs (shared code), checked out with it.
    #[serde(default)]
    pub also: Vec<String>,
    #[serde(default = "default_icon")]
    pub icon: String,
}

fn default_icon() -> String {
    "application-x-addon-symbolic".into()
}

#[derive(Debug, Deserialize)]
struct Index {
    extensions: Vec<Entry>,
}

pub fn parse_index(text: &str) -> Result<Vec<Entry>> {
    Ok(serde_json::from_str::<Index>(text)?.extensions)
}

fn cache_file() -> PathBuf {
    paths::ext_cache_dir().join("index.json")
}

/// The catalog: fetched from GitHub, else the last one fetched, else the built-in one.
pub fn catalog(fetch: bool) -> Vec<Entry> {
    let source = std::env::var("SETTINGS_EXT_INDEX").unwrap_or_else(|_| INDEX_URL.to_string());
    if fetch {
        let text = if source.contains("://") {
            cmd::output(&["curl", "-fsSL", "--max-time", "10", &source])
        } else {
            std::fs::read_to_string(&source).ok()
        };
        if let Some(t) = text
            && let Ok(list) = parse_index(&t)
        {
            let _ = cmd::atomic_write(&cache_file(), &t);
            return list;
        }
    }
    std::fs::read_to_string(cache_file())
        .ok()
        .and_then(|t| parse_index(&t).ok())
        .or_else(|| parse_index(BUILT_IN).ok())
        .unwrap_or_default()
}

pub fn entry(id: &str, fetch: bool) -> Option<Entry> {
    catalog(fetch).into_iter().find(|e| e.id == id)
}

fn git(args: &[&str], cwd: Option<&Path>) -> Result<String> {
    let mut c = Command::new("git");
    c.args(args).env("GIT_TERMINAL_PROMPT", "0");
    if let Some(d) = cwd {
        c.current_dir(d);
    }
    run_command(c, "git", GIT_TIMEOUT)
}

/// Run the extension's install script, if it has one.
fn run_install(ext: &Extension) -> Result<()> {
    let script = ext.manifest.install.trim();
    if script.is_empty() {
        return Ok(());
    }
    let mut c = Command::new("bash");
    c.arg(script).current_dir(&ext.dir).env("SETTINGS_EXTENSION_DIR", &ext.dir);
    let out = run_command(c, &format!("{} install", ext.manifest.name), INSTALL_TIMEOUT);
    if let Err(e) = &out {
        let _ = cmd::atomic_write(&paths::ext_cache_dir().join(format!("{}.install.log", ext.id())), format!("{e:#}\n"));
    }
    out.map(|_| ())
}

/// Clone `repo` (just `path` inside it, when given), check it's an extension and
/// install it. `expect_id` guards a catalog install against a repo with another id.
pub fn install_from(repo: &str, path: &str, also: &[String], expect_id: Option<&str>) -> Result<Extension> {
    if !cmd::present("git") {
        bail!("git is needed to install extensions");
    }
    let base = paths::ext_dir();
    std::fs::create_dir_all(&base)?;
    let tmp = base.join(format!(".incoming-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let tmp_s = tmp.to_string_lossy().to_string();
    let result = (|| -> Result<Extension> {
        if path.is_empty() {
            git(&["clone", "--depth", "1", repo, &tmp_s], None)?;
        } else {
            git(&["clone", "--depth", "1", "--filter=blob:none", "--sparse", repo, &tmp_s], None)?;
            let mut set = vec!["sparse-checkout", "set", path];
            set.extend(also.iter().map(String::as_str));
            git(&set, Some(&tmp))?;
        }
        let staged = load(&tmp)?;
        if let Some(id) = expect_id
            && staged.id() != id
        {
            bail!("that repository holds \"{}\", not \"{id}\"", staged.id());
        }
        let dest = base.join(staged.id());
        if dest.exists() {
            bail!("{} is already installed", staged.manifest.name);
        }
        std::fs::rename(&tmp, &dest).context("couldn't move the extension into place")?;
        let ext = load(&dest)?;
        if let Err(e) = run_install(&ext) {
            let _ = std::fs::remove_dir_all(&dest);
            return Err(e.context(format!("{} couldn't be set up", ext.manifest.name)));
        }
        Ok(ext)
    })();
    let _ = std::fs::remove_dir_all(&tmp);
    let ext = result?;
    if let Ok(p) = ext.query_pages() {
        super::remember_pages(ext.id(), &p);
    }
    Ok(ext)
}

pub fn install(id: &str) -> Result<Extension> {
    let e = entry(id, true).with_context(|| format!("there's no extension called \"{id}\" in the catalog"))?;
    install_from(&e.repo, &e.path, &e.also, Some(&e.id))
}

/// Installed from the repository extensions used to live in.
fn from_old_repo(ext: &Extension) -> bool {
    git(&["remote", "get-url", "origin"], Some(&ext.root)).is_ok_and(|u| u.contains(OLD_REPO))
}

/// Install `ext` again from where the catalog has it now, keeping the old copy
/// until the new one is in place.
fn move_to_catalog(ext: &Extension) -> Result<bool> {
    let id = ext.id().to_string();
    let e = entry(&id, true).with_context(|| format!("\"{id}\" isn't in the catalog any more"))?;
    let aside = ext.root.with_file_name(format!(".{id}-moving"));
    let _ = std::fs::remove_dir_all(&aside);
    std::fs::rename(&ext.root, &aside)?;
    match install_from(&e.repo, &e.path, &e.also, Some(&id)) {
        Ok(_) => {
            let _ = std::fs::remove_dir_all(&aside);
            Ok(true)
        }
        Err(err) => {
            let _ = std::fs::rename(&aside, &ext.root);
            Err(err)
        }
    }
}

/// Whether a newer version of this extension is upstream. Fetches only the newest
/// commit (nothing in the checkout changes) and compares the extension's own
/// folder, so a change elsewhere in a shared repository doesn't count.
pub fn has_update(ext: &Extension) -> Result<bool> {
    if !ext.root.join(".git").exists() {
        return Ok(false);
    }
    if from_old_repo(ext) {
        return Ok(true);
    }
    let branch = git(&["rev-parse", "--abbrev-ref", "HEAD"], Some(&ext.root))?;
    git(&["fetch", "--depth", "1", "origin", branch.trim()], Some(&ext.root))?;
    let rel = ext.dir.strip_prefix(&ext.root).map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
    let tree = |rev: &str| {
        let spec = if rel.is_empty() { format!("{rev}^{{tree}}") } else { format!("{rev}:{rel}") };
        git(&["rev-parse", &spec], Some(&ext.root)).map(|s| s.trim().to_string())
    };
    let here = tree("HEAD")?;
    // Gone upstream: nothing to update to.
    let Ok(there) = tree("FETCH_HEAD") else { return Ok(false) };
    Ok(here != there)
}

/// Pull the latest version and set it up again. Returns whether anything changed.
pub fn update(ext: &Extension) -> Result<bool> {
    if !ext.root.join(".git").exists() {
        bail!("{} wasn't installed from git", ext.manifest.name);
    }
    if from_old_repo(ext) {
        return move_to_catalog(ext);
    }
    let before = git(&["rev-parse", "HEAD"], Some(&ext.root)).unwrap_or_default();
    // Fetch just the newest commit and move to it. A `pull` in this shallow,
    // partial checkout fails once more than one commit has landed upstream.
    // Untracked build output (bin/, target/, vendor/) is left alone.
    let branch = git(&["rev-parse", "--abbrev-ref", "HEAD"], Some(&ext.root))?;
    git(&["fetch", "--depth", "1", "origin", branch.trim()], Some(&ext.root))?;
    git(&["reset", "--hard", "FETCH_HEAD"], Some(&ext.root))?;
    let after = git(&["rev-parse", "HEAD"], Some(&ext.root)).unwrap_or_default();
    let changed = before != after;
    if changed {
        let fresh = load(&ext.root)?;
        run_install(&fresh)?;
        if let Ok(p) = fresh.query_pages() {
            super::remember_pages(fresh.id(), &p);
        }
    }
    Ok(changed)
}

/// Turn an installed extension on or off without removing it.
pub fn set_enabled(id: &str, on: bool) {
    crate::prefs::update(|p| {
        p.disabled_extensions.retain(|d| d != id);
        if !on {
            p.disabled_extensions.push(id.to_string());
        }
    });
}

pub fn remove(ext: &Extension) -> Result<()> {
    // Only ever delete inside the extensions folder.
    let base = paths::ext_dir();
    if !ext.root.starts_with(&base) || ext.root == base {
        bail!("{} isn't in {}", paths::pretty(&ext.root), paths::pretty(&base));
    }
    std::fs::remove_dir_all(&ext.root).with_context(|| format!("couldn't remove {}", paths::pretty(&ext.root)))?;
    super::forget_pages(ext.id());
    if !super::enabled(ext.id()) {
        set_enabled(ext.id(), true);
    }
    Ok(())
}

/// `settings --ext …`
pub fn cli(args: &[String]) -> Result<()> {
    let verb = args.first().map(String::as_str).unwrap_or("list");
    let arg = args.get(1).map(String::as_str).unwrap_or("");
    match verb {
        "list" => {
            let installed = super::installed();
            let mark_for = |id: &str| match installed.iter().any(|i| i.id() == id) {
                true if super::enabled(id) => "installed",
                true => "disabled",
                false => "",
            };
            for e in catalog(true) {
                let mark = mark_for(&e.id);
                println!("{:<12} {:<10} {}", e.id, mark, e.name);
            }
            for i in installed.iter().filter(|i| !catalog(false).iter().any(|e| e.id == i.id())) {
                println!("{:<12} {:<10} {} (local)", i.id(), mark_for(i.id()), i.manifest.name);
            }
            Ok(())
        }
        "install" if arg.contains("://") || arg.starts_with("git@") => {
            let path = args.get(2).map(String::as_str).unwrap_or("");
            let e = install_from(arg, path, &[], None)?;
            println!("Installed {}", e.manifest.name);
            Ok(())
        }
        "install" if !arg.is_empty() => {
            let e = install(arg)?;
            println!("Installed {}", e.manifest.name);
            Ok(())
        }
        "update" => {
            let targets: Vec<Extension> = super::installed().into_iter().filter(|e| arg.is_empty() || e.id() == arg).collect();
            if targets.is_empty() {
                bail!("nothing to update");
            }
            for e in targets {
                let changed = update(&e)?;
                println!("{}: {}", e.manifest.name, if changed { "updated" } else { "up to date" });
            }
            Ok(())
        }
        "enable" | "disable" if !arg.is_empty() => {
            let e = super::find(arg).with_context(|| format!("\"{arg}\" isn't installed"))?;
            set_enabled(e.id(), verb == "enable");
            // The theme hook may now be needed, or not.
            crate::backend::themehook::ensure();
            println!("{} {}", e.manifest.name, if verb == "enable" { "turned on" } else { "turned off" });
            Ok(())
        }
        "remove" if !arg.is_empty() => {
            let e = super::find(arg).with_context(|| format!("\"{arg}\" isn't installed"))?;
            remove(&e)?;
            println!("Removed {}", e.manifest.name);
            Ok(())
        }
        _ => bail!("usage: settings --ext list | install ID|URL [PATH] | update [ID] | enable ID | disable ID | remove ID"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_catalog_parses() {
        let list = parse_index(BUILT_IN).unwrap();
        assert!(list.iter().any(|e| e.id == "asus"));
        for e in &list {
            assert!(!e.repo.is_empty() && !e.name.is_empty(), "{e:?}");
        }
    }

    #[test]
    fn index_defaults() {
        let l = parse_index(r#"{"extensions":[{"id":"x","name":"X","repo":"https://example.com/x.git"}]}"#).unwrap();
        assert_eq!(l[0].icon, "application-x-addon-symbolic");
        assert!(l[0].path.is_empty() && l[0].needs.is_empty());
    }

    #[test]
    fn turns_extensions_off_and_on() {
        let scratch = crate::ext::tests::scratch("enabled");
        // SAFETY: only this test changes XDG_CONFIG_HOME; prefs are read from it fresh.
        unsafe { std::env::set_var("XDG_CONFIG_HOME", &scratch) };
        assert!(crate::ext::enabled("demo"));
        set_enabled("demo", false);
        set_enabled("demo", false);
        assert!(!crate::ext::enabled("demo"));
        assert_eq!(crate::prefs::read().disabled_extensions, ["demo"], "listed once");
        set_enabled("demo", true);
        assert!(crate::ext::enabled("demo"));
        unsafe { std::env::remove_var("XDG_CONFIG_HOME") };
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn installs_from_a_local_repo() {
        if !cmd::present("git") {
            return;
        }
        let scratch = crate::ext::tests::scratch("install");
        let repo = scratch.join("repo");
        let ext = repo.join("demo");
        std::fs::create_dir_all(&ext).unwrap();
        std::fs::write(repo.join("index.json"), "{}").unwrap();
        std::fs::write(
            ext.join("extension.toml"),
            "id = \"demo\"\nname = \"Demo\"\nexec = \"demo.sh\"\ninstall = \"setup.sh\"\n",
        )
        .unwrap();
        std::fs::write(ext.join("demo.sh"), "#!/bin/sh\necho '[]'\n").unwrap();
        std::fs::write(ext.join("setup.sh"), "touch built\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(ext.join("demo.sh"), std::fs::Permissions::from_mode(0o755)).unwrap();
        let r = repo.to_string_lossy().to_string();
        for a in [
            vec!["init", "-q", "-b", "main", &r],
            vec!["-C", &r, "add", "."],
            vec!["-C", &r, "-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "x"],
        ] {
            assert!(Command::new("git").args(&a).status().unwrap().success());
        }

        // SAFETY: only this test changes XDG_DATA_HOME / XDG_CACHE_HOME.
        unsafe {
            std::env::set_var("XDG_DATA_HOME", scratch.join("data"));
            std::env::set_var("XDG_CACHE_HOME", scratch.join("cache"));
        }
        let url = format!("file://{r}");
        assert!(install_from(&url, "demo", &[], Some("other")).is_err());
        let e = install_from(&url, "demo", &[], Some("demo")).unwrap();
        assert_eq!(e.root, crate::paths::ext_dir().join("demo"));
        assert!(e.dir.join("built").exists(), "install script ran");
        assert!(install_from(&url, "demo", &[], None).unwrap_err().to_string().contains("already installed"));
        assert!(!update(&e).unwrap());
        assert!(!has_update(&e).unwrap());
        // A change outside the extension's folder doesn't count.
        std::fs::write(repo.join("index.json"), "{\"x\":1}").unwrap();
        assert!(
            Command::new("git")
                .args(["-C", &r, "-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qam", "other"])
                .status()
                .unwrap()
                .success()
        );
        assert!(!has_update(&e).unwrap(), "only its own folder matters");
        // Several commits upstream (a plain shallow pull fails on these).
        for n in 1..=3 {
            std::fs::write(ext.join("demo.sh"), format!("#!/bin/sh\necho '[]' # v{n}\n")).unwrap();
            assert!(
                Command::new("git")
                    .args(["-C", &r, "-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qam", "next"])
                    .status()
                    .unwrap()
                    .success()
            );
        }
        std::fs::write(e.dir.join("bin-output"), "kept").unwrap();
        assert!(has_update(&e).unwrap());
        assert!(update(&e).unwrap());
        assert!(std::fs::read_to_string(e.dir.join("demo.sh")).unwrap().contains("# v3"));
        assert!(e.dir.join("bin-output").exists(), "untracked build output survives");
        assert!(!update(&e).unwrap());
        remove(&e).unwrap();
        assert!(!e.root.exists());
        unsafe {
            std::env::remove_var("XDG_DATA_HOME");
            std::env::remove_var("XDG_CACHE_HOME");
        }
        let _ = std::fs::remove_dir_all(&scratch);
    }
}
