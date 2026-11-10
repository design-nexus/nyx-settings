//! `nyx display`: displays from a terminal, starting with virtual monitors (one wide
//! screen split into two or three). It changes the same settings as Nyx Settings'
//! Displays page (`virtual-monitors.toml`, through nyx-desktop), and works when Settings
//! can't start, which makes `nyx display virtual disable --all` the way back from a
//! layout that went wrong.

use anyhow::{Context, Result, bail};
use nyx_desktop::Desktop;
use nyx_desktop::hypr::Instance;
use nyx_desktop::virtual_monitors::{self as vm, Display, Layout, Paths, Settings};
use std::io::{BufRead, IsTerminal, Write};
use std::time::Duration;

const USAGE: &str = "\
Usage: nyx display virtual <command> [options]

Commands:
  list                         the screens, how each is split, and Hyprland's view
  enable <screen>              split a screen (with its saved layout, else two equal)
  disable <screen> | --all     give a screen back whole; --all turns virtual monitors off
  split <screen> --count N     N equal regions (1 to 3)
  layout <screen> --preset P   P: full, equal, center-focused
  layout <screen> --ratios A,B[,C]
  layout <screen> --profile NAME
  merge <screen>               join the regions for now (nothing closes)
  restore <screen>             split it again, workspaces back where they were
  profiles                     the saved layouts
  profiles save NAME <screen>  save a screen's layout as NAME
  profiles delete NAME
  target focus|pointer         which region workspace shortcuts act on
  fullscreen region|physical   what fullscreen does on a region
  confirm                      keep a change that is waiting to be confirmed

<screen> is a connector (DP-3), one of its regions (DP-3.v2) or desc:<description>.

Options:
  --yes        apply without waiting for confirmation
  --json       list as JSON
  --timeout N  seconds a change waits to be kept (default: the saved setting)

A change takes effect at once. In a terminal it asks whether to keep it, and Hyprland
puts the previous layout back unless you answer y in time; only a kept change is saved.";

struct Opts {
    yes: bool,
    json: bool,
    all: bool,
    count: Option<u8>,
    preset: Option<String>,
    ratios: Option<Vec<f64>>,
    profile: Option<String>,
    timeout: Option<u32>,
    rest: Vec<String>,
}

fn parse(args: &[String]) -> Result<Opts> {
    let mut o = Opts {
        yes: false,
        json: false,
        all: false,
        count: None,
        preset: None,
        ratios: None,
        profile: None,
        timeout: None,
        rest: Vec::new(),
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut value = |name: &str| it.next().cloned().with_context(|| format!("{name} needs a value"));
        match a.as_str() {
            "--yes" | "-y" => o.yes = true,
            "--json" => o.json = true,
            "--all" => o.all = true,
            "--count" => o.count = Some(value("--count")?.parse().context("--count is a number")?),
            "--preset" => o.preset = Some(value("--preset")?),
            "--profile" => o.profile = Some(value("--profile")?),
            "--timeout" => o.timeout = Some(value("--timeout")?.parse().context("--timeout is a number of seconds")?),
            "--ratios" => {
                let v = value("--ratios")?;
                o.ratios = Some(
                    v.split(',')
                        .map(|r| r.trim().parse::<f64>().context("--ratios is numbers separated by commas"))
                        .collect::<Result<_>>()?,
                );
            }
            "-h" | "--help" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            s if s.starts_with("--") => bail!("unknown option {s}"),
            _ => o.rest.push(a.clone()),
        }
    }
    Ok(o)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(e) = run(&args) {
        eprintln!("nyx display: {e:#}");
        std::process::exit(1);
    }
}

fn run(args: &[String]) -> Result<()> {
    match args.first().map(String::as_str) {
        Some("virtual") => virtual_cmd(&parse(&args[1..])?),
        None | Some("-h" | "--help" | "help") => {
            println!("{USAGE}");
            Ok(())
        }
        Some(other) => bail!("unknown command {other}; try `nyx display virtual list`"),
    }
}

/// A physical screen as Hyprland has it: an ordinary monitor or a split one's host.
struct Screen {
    name: String,
    stable_id: String,
    description: String,
    width: i32,
    scale: f64,
}

fn screens(d: &Desktop) -> Vec<Screen> {
    let mut out: Vec<Screen> = d
        .monitors
        .iter()
        .filter(|m| d.region(&m.name).is_none() && !m.disabled)
        .map(|m| Screen {
            name: m.name.clone(),
            stable_id: m.stable_id(),
            description: m.description.clone(),
            width: m.width,
            scale: m.scale,
        })
        .collect();
    out.extend(d.hosts.iter().map(|h| Screen {
        name: h.name.clone(),
        stable_id: h.stable_id.clone(),
        description: h.description.clone(),
        width: h.width,
        scale: h.scale,
    }));
    out
}

fn find_screen(d: &Desktop, which: &str) -> Result<Screen> {
    let which = d.region(which).map_or(which, |r| r.host.as_str());
    let desc = which.strip_prefix("desc:").map(str::trim);
    screens(d)
        .into_iter()
        .find(|s| s.name == which || s.stable_id == which || desc.is_some_and(|w| s.description.starts_with(w)))
        .with_context(|| format!("no screen {which}; `nyx display virtual list` shows them"))
}

fn screen_arg(o: &Opts) -> Result<&str> {
    o.rest.get(1).map(String::as_str).context("which screen? (a connector like DP-3)")
}

fn load() -> Result<(Settings, Paths)> {
    let paths = Paths::default_paths()?;
    Ok((Settings::load(&paths)?, paths))
}

fn virtual_cmd(o: &Opts) -> Result<()> {
    let cmd = o.rest.first().map(String::as_str).unwrap_or("list");
    // Recovery first: it must work whatever else is wrong.
    if cmd == "disable" && o.all {
        return disable_all();
    }
    let hypr = Instance::find().context("Hyprland isn't running")?;
    let status = vm::status(&hypr)?;
    let desktop = Desktop::snapshot(&hypr)?;
    match cmd {
        "list" => list(o, &desktop, &status),
        "enable" => {
            let s = find_screen(&desktop, screen_arg(o)?)?;
            change(o, &hypr, &s, true, |d| {
                d.enabled = true;
                if d.layout.regions() < 2 {
                    d.layout = Layout::equal(2);
                }
            })
        }
        "disable" => {
            let s = find_screen(&desktop, screen_arg(o)?)?;
            change(o, &hypr, &s, false, |d| d.enabled = false)
        }
        "split" => {
            let n = o.count.context("split needs --count N")?;
            let s = find_screen(&desktop, screen_arg(o)?)?;
            change(o, &hypr, &s, true, |d| {
                d.enabled = n > 1;
                d.layout = Layout::equal(n);
                d.profile.clear();
            })
        }
        "layout" => {
            let s = find_screen(&desktop, screen_arg(o)?)?;
            let (settings, _) = load()?;
            let (layout, profile) = if let Some(r) = &o.ratios {
                (Layout::custom(r.clone()), String::new())
            } else if let Some(p) = &o.profile {
                (settings.profiles.get(p).cloned().with_context(|| format!("no saved layout {p}"))?, p.clone())
            } else {
                let layout = match o.preset.as_deref() {
                    Some("full") => Layout::full(),
                    Some("equal") => Layout::equal(o.count.unwrap_or(2)),
                    Some("center-focused" | "center") => Layout::center_focused(),
                    Some(p) => bail!("unknown preset {p} (full, equal, center-focused)"),
                    None => bail!("layout needs --preset, --ratios or --profile"),
                };
                (layout, String::new())
            };
            change(o, &hypr, &s, true, |d| {
                d.enabled = layout.regions() > 1;
                d.layout = layout;
                d.profile = profile;
            })
        }
        "merge" => vm::merge(&hypr, &find_screen(&desktop, screen_arg(o)?)?.name),
        "restore" => vm::restore(&hypr, &find_screen(&desktop, screen_arg(o)?)?.name),
        "confirm" => vm::confirm(&hypr),
        "profiles" => profiles(o, &desktop),
        "target" | "fullscreen" => {
            let (mut settings, paths) = load()?;
            match (cmd, o.rest.get(1).map(String::as_str)) {
                ("target", Some("focus")) => settings.workspace_target = vm::Target::Focus,
                ("target", Some("pointer")) => settings.workspace_target = vm::Target::Pointer,
                ("fullscreen", Some("region")) => settings.fullscreen = vm::Fullscreen::Region,
                ("fullscreen", Some("physical")) => settings.fullscreen = vm::Fullscreen::Physical,
                _ => bail!("{cmd} needs {}", if cmd == "target" { "focus or pointer" } else { "region or physical" }),
            }
            vm::apply_options(&hypr, &settings)?;
            settings.save(&paths)
        }
        other => bail!("unknown command {other}\n\n{USAGE}"),
    }
}

/// Turn everything off: now in Hyprland (if it's reachable), and saved for next time.
fn disable_all() -> Result<()> {
    let mut problems = Vec::new();
    match Instance::find() {
        Ok(hypr) => {
            if let Err(e) = vm::disable_all(&hypr) {
                problems.push(format!("Hyprland: {e:#}"));
            }
        }
        Err(e) => problems.push(format!("{e:#}")),
    }
    let paths = Paths::default_paths()?;
    // A broken settings file mustn't stand in the way: start from the defaults then.
    let mut settings = Settings::load(&paths).unwrap_or_default();
    settings.enabled = false;
    settings.save(&paths)?;
    println!("Virtual monitors are off: every screen is whole, and stays so at the next login.");
    if !problems.is_empty() {
        println!("(Not changed in the running session: {})", problems.join("; "));
    }
    Ok(())
}

fn change(o: &Opts, hypr: &Instance, screen: &Screen, risky: bool, f: impl FnOnce(&mut Display)) -> Result<()> {
    let (mut settings, paths) = load()?;
    let display = {
        let d = settings.entry(&screen.stable_id, &screen.name, &screen.description);
        f(d);
        d.clone()
    };
    if display.enabled {
        vm::validate(&display.layout, screen.width, screen.scale)?;
        settings.enabled = true;
    }
    let ask = risky && display.enabled && !o.yes;
    if ask && !std::io::stdin().is_terminal() {
        bail!("not a terminal, so there's no one to confirm the change: add --yes to apply it straight away");
    }
    let seconds = o.timeout.unwrap_or(settings.confirm_seconds).max(5);
    vm::apply(hypr, &settings, &display, ask.then_some(seconds))?;
    if ask {
        print!("Keep this layout? [y/N] (it goes back in {seconds} seconds) ");
        std::io::stdout().flush()?;
        if !answered_yes(Duration::from_secs(seconds as u64)) {
            println!("\nPut back as it was.");
            return Ok(());
        }
        vm::confirm(hypr)?;
    }
    settings.save(&paths)?;
    println!("{}", describe(&display));
    Ok(())
}

fn answered_yes(within: Duration) -> bool {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let _ = std::io::stdin().lock().read_line(&mut line);
        let _ = tx.send(line);
    });
    rx.recv_timeout(within).is_ok_and(|l| matches!(l.trim().to_lowercase().as_str(), "y" | "yes"))
}

fn describe(d: &Display) -> String {
    if !d.enabled || d.layout.regions() < 2 {
        return format!("{} is one screen.", d.connector);
    }
    let r = d.layout.proportions().unwrap_or_default();
    let total: f64 = r.iter().sum();
    let parts: Vec<String> = r.iter().map(|x| format!("{:.0}%", x / total * 100.0)).collect();
    format!("{} is split into {} regions: {}.", d.connector, r.len(), parts.join(" | "))
}

fn list(o: &Opts, d: &Desktop, status: &vm::Status) -> Result<()> {
    let (settings, _) = load()?;
    if o.json {
        let screens: Vec<serde_json::Value> = screens(d)
            .iter()
            .map(|s| {
                let saved = settings.display(&s.stable_id, &s.name);
                let live = status.groups.iter().find(|g| g.host == s.name);
                serde_json::json!({
                    "screen": s.name, "id": s.stable_id, "description": s.description,
                    "split": saved.is_some_and(|d| d.enabled), "layout": saved.map(|d| d.layout.preset.as_str()),
                    "proportions": saved.and_then(|d| d.layout.proportions().ok()),
                    "state": live.map(|g| g.state.as_str()),
                    "regions": live.map(|g| g.regions.iter().map(|r| serde_json::json!({"name": r.name, "width": r.width, "workspace": r.active_workspace})).collect::<Vec<_>>()),
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({"on": settings.enabled && status.available, "screens": screens}))?
        );
        return Ok(());
    }
    println!(
        "Virtual monitors: {}{}",
        if settings.enabled { "on" } else { "off" },
        if status.killed { " (turned off for this session)" } else { "" }
    );
    if let Some(left) = status.pending_confirm {
        println!("A change is waiting to be kept ({left} s left): nyx display virtual confirm");
    }
    for s in screens(d) {
        let saved = settings.display(&s.stable_id, &s.name);
        let live = status.groups.iter().find(|g| g.host == s.name);
        println!("\n{} ({})", s.name, s.description);
        match (saved, live) {
            (_, Some(g)) => {
                println!(
                    "  {} into {} regions",
                    if g.merged() { "merged for now; normally split" } else { "split" },
                    g.regions.len()
                );
                for r in &g.regions {
                    println!("    {}: {} px wide, workspaces from {}", r.name, r.width, r.workspace_base + 1);
                }
            }
            (Some(d), None) if d.enabled => println!("  set to split ({}), not split now", d.layout.preset.as_str()),
            _ => println!("  one screen"),
        }
    }
    Ok(())
}

fn profiles(o: &Opts, d: &Desktop) -> Result<()> {
    let (mut settings, paths) = load()?;
    match o.rest.get(1).map(String::as_str) {
        None => {
            for (name, l) in &settings.profiles {
                let r = l.proportions().unwrap_or_default();
                let total: f64 = r.iter().sum();
                println!("{name}: {}", r.iter().map(|x| format!("{:.0}%", x / total * 100.0)).collect::<Vec<_>>().join(" | "));
            }
            Ok(())
        }
        Some("save") => {
            let name = o.rest.get(2).context("profiles save NAME <screen>")?;
            let s = find_screen(d, o.rest.get(3).context("profiles save NAME <screen>")?)?;
            let layout =
                settings.display(&s.stable_id, &s.name).map(|d| d.layout.clone()).context("that screen has no layout to save")?;
            settings.profiles.insert(name.clone(), layout);
            settings.save(&paths)
        }
        Some("delete") => {
            let name = o.rest.get(2).context("profiles delete NAME")?;
            settings.profiles.remove(name).with_context(|| format!("no saved layout {name}"))?;
            settings.save(&paths)
        }
        Some(other) => bail!("unknown profiles command {other}"),
    }
}
