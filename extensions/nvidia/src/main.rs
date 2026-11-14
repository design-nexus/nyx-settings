//! NVIDIA graphics through nvidia-smi: what the GPU is doing (temperature,
//! use, memory, clocks, power), which programs use it, the power limit and
//! persistence mode. Fans are shown only: on Wayland the driver offers no
//! way to set them.

mod mock;

use anyhow::{Context, Result, bail};
use nyx_ext::page::{self, Page, desc};
use nyx_ext::{Extension, sys};
use serde_json::{Value, json};

fn smi(args: &[&str]) -> Result<String> {
    if mock::on() { mock::smi(args) } else { sys::run("nvidia-smi", args) }
}

fn smi_root(args: &[&str]) -> Result<String> {
    if mock::on() { mock::smi(args) } else { sys::run_root("nvidia-smi", args) }
}

/// Whether this Settings draws a preview pane beside the groups.
fn preview_supported() -> bool {
    std::env::var("SETTINGS_KINDS").is_ok_and(|k| k.split(' ').any(|x| x == "preview"))
}

const FIELDS: &str = "index,name,driver_version,temperature.gpu,utilization.gpu,memory.used,memory.total,power.draw,power.limit,power.min_limit,power.max_limit,clocks.gr,clocks.mem,fan.speed,persistence_mode,pstate,pci.bus_id";

#[derive(Debug, Clone, Default, PartialEq)]
struct Gpu {
    index: u32,
    name: String,
    driver: String,
    temp: Option<f64>,
    util: Option<f64>,
    mem_used: Option<f64>,
    mem_total: Option<f64>,
    power: Option<f64>,
    limit: Option<f64>,
    limit_min: Option<f64>,
    limit_max: Option<f64>,
    clock: Option<f64>,
    mem_clock: Option<f64>,
    fan: Option<f64>,
    persistence: bool,
    pstate: String,
    bus: String,
}

/// A number from nvidia-smi's csv ("[N/A]", "[Not Supported]" are none).
fn num(s: &str) -> Option<f64> {
    s.trim().split_whitespace().next()?.parse().ok()
}

fn parse(text: &str) -> Vec<Gpu> {
    text.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split(',').map(str::trim).collect();
            if f.len() < 17 {
                return None;
            }
            Some(Gpu {
                index: f[0].parse().ok()?,
                name: f[1].to_string(),
                driver: f[2].to_string(),
                temp: num(f[3]),
                util: num(f[4]),
                mem_used: num(f[5]),
                mem_total: num(f[6]),
                power: num(f[7]),
                limit: num(f[8]),
                limit_min: num(f[9]),
                limit_max: num(f[10]),
                clock: num(f[11]),
                mem_clock: num(f[12]),
                fan: num(f[13]),
                persistence: f[14] == "Enabled",
                pstate: f[15].to_string(),
                bus: f[16].to_string(),
            })
        })
        .collect()
}

/// (pid, program, MiB) for programs using a GPU.
fn parse_apps(text: &str) -> Vec<(String, String, String)> {
    text.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split(',').map(str::trim).collect();
            (f.len() >= 3).then(|| (f[0].to_string(), f[1].rsplit('/').next().unwrap_or(f[1]).to_string(), f[2].to_string()))
        })
        .collect()
}

enum State {
    Ok(Vec<Gpu>),
    /// The driver was updated; the computer needs a restart to load it.
    Mismatch,
    /// nvidia-smi failed some other way.
    Failed(String),
}

fn query() -> State {
    match smi(&[&format!("--query-gpu={FIELDS}"), "--format=csv,noheader,nounits"]) {
        Ok(t) => State::Ok(parse(&t)),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("version mismatch") { State::Mismatch } else { State::Failed(msg) }
        }
    }
}

fn one(v: Option<f64>, unit: &str) -> String {
    v.map(|x| format!("{}{unit}", if x.fract() == 0.0 { format!("{x:.0}") } else { format!("{x:.1}") })).unwrap_or_else(|| "—".into())
}

/// 8840 → "8,840".
fn thousands(n: f64) -> String {
    let s = format!("{:.0}", n.round());
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// The PCI address as lspci writes it; nvidia-smi pads the domain to eight digits.
fn pci(bus: &str) -> String {
    match bus.split_once(':') {
        Some((domain, rest)) if domain.len() == 8 && domain.starts_with("0000") => format!("{}:{rest}", &domain[4..]),
        _ => bus.to_string(),
    }
}

fn megabytes(mib: &str) -> String {
    mib.parse::<f64>().map(|m| format!("{} MB", thousands(m))).unwrap_or_else(|_| format!("{mib} MB"))
}

/// A row with an icon, for the pane layout's left column.
fn icon(mut row: Value, name: &str) -> Value {
    row["icon"] = json!(name);
    row
}

const PROGRAM_ICON: &str = "application-x-executable-symbolic";

/// The page with a preview pane: controls and programs on the left, the card and
/// what it's doing on the right.
fn describe_pane(g: &Gpu, apps: &[(String, String, String)]) -> Value {
    let mut control = Vec::new();
    if let (Some(lim), Some(lo), Some(hi)) = (g.limit, g.limit_min, g.limit_max)
        && hi > lo
    {
        let row = page::slider("limit", "Power limit", lim, (lo.ceil(), hi.floor(), 5.0), " W");
        control.push(icon(desc(row, "Lower is cooler and quieter"), "power-profile-performance-symbolic"));
    }
    let persistence = page::switch("persistence", "Keep the driver loaded", g.persistence);
    control.push(icon(desc(persistence, "Programs start on the GPU faster"), "applications-system-symbolic"));
    let note = if control.len() > 1 {
        "Keeping the driver loaded costs a little power while idle. Both ask for your password; the power limit goes back to the default at restart."
    } else {
        "Keeping the driver loaded costs a little power while idle. It asks for your password."
    };
    let app_rows: Vec<Value> = if apps.is_empty() {
        vec![icon(page::info("none", "Programs", "None right now"), PROGRAM_ICON)]
    } else {
        apps.iter().map(|(pid, name, mem)| icon(page::info(&format!("app{pid}"), name, &megabytes(mem)), PROGRAM_ICON)).collect()
    };

    let gb = |m: Option<f64>| one(m.map(|m| m / 1024.0), "");
    let memory = if g.mem_used.is_some() || g.mem_total.is_some() { format!("{} of {} GB", gb(g.mem_used), gb(g.mem_total)) } else { "—".into() };
    let programs = match apps.len() {
        0 => "No programs".to_string(),
        1 => "1 program".to_string(),
        n => format!("{n} programs"),
    };
    let clock = g.clock.map(|c| format!("{} MHz", thousands(c)));
    let state = [Some(g.pstate.clone()).filter(|p| !p.is_empty()), clock].into_iter().flatten().collect::<Vec<_>>().join(" · ");
    let stats = json!([
        { "label": "Temperature", "value": one(g.temp, " °C"), "sub": g.fan.map(|f| format!("Fan {}", one(Some(f), "%"))).unwrap_or_default(),
          "tone": if g.temp.is_some_and(|t| t >= 90.0) { "warn" } else { "" } },
        { "label": "In use", "value": one(g.util, "%"), "sub": state },
        { "label": "Memory", "value": memory, "sub": programs },
        { "label": "Power", "value": one(g.power, " W"), "sub": g.limit.map(|l| format!("Limit {}", one(Some(l), " W"))).unwrap_or_default() },
    ]);
    let mut blocks = vec![json!({ "kind": "picture", "picture": "gpu" }), json!({ "kind": "stats", "items": stats })];
    if apps.is_empty() {
        blocks.push(json!({ "kind": "caption", "text": "No programs are using its memory right now." }));
    } else {
        let share = |mem: &str| {
            let (m, total) = (mem.parse::<f64>().ok()?, g.mem_total.filter(|t| *t > 0.0)?);
            let pct = m * 100.0 / total;
            Some(if pct < 1.0 { "Under 1% of its memory".to_string() } else { format!("{pct:.0}% of its memory") })
        };
        let items: Vec<Value> = apps
            .iter()
            .map(|(_, name, mem)| json!({ "icon": PROGRAM_ICON, "label": name, "sub": share(mem).unwrap_or_default(), "value": megabytes(mem) }))
            .collect();
        blocks.push(json!({ "kind": "list", "title": "Memory by program", "items": items }));
    }
    let title = g.name.trim_start_matches("NVIDIA ").trim_start_matches("GeForce ");
    let mut line = format!("Driver {}", g.driver);
    if !g.bus.is_empty() {
        line.push_str(&format!(" · PCI {}", pci(&g.bus)));
    }
    line.push_str(" · updated every 3 s");
    let mut page = Page::new("").poll(3).group("Control", note, control).group("Programs using it", "", app_rows).build();
    page["preview"] = json!({ "title": title, "line": line, "blocks": blocks });
    page
}

struct Nvidia;

impl Extension for Nvidia {
    fn pages(&self) -> Value {
        if !mock::on() && !sys::present("nvidia-smi") {
            return json!([]);
        }
        match query() {
            State::Ok(gpus) => Value::Array(
                gpus.iter()
                    .map(|g| {
                        page::entry(&format!("gpu{}", g.index), &g.name.trim_start_matches("NVIDIA ").to_string(), "video-display-symbolic",
                            "Temperature, use, memory, power limit and which programs use the GPU.",
                            "nvidia gpu graphics card temperature power limit vram memory cuda persistence")
                    })
                    .collect(),
            ),
            // Still a page, to say what's wrong.
            _ => json!([page::entry("gpu0", "NVIDIA", "video-display-symbolic", "NVIDIA graphics.", "nvidia gpu graphics")]),
        }
    }

    fn describe(&self, id: &str) -> Result<Value> {
        let gpus = match query() {
            State::Ok(g) => g,
            State::Mismatch => {
                return Ok(Page::new("")
                    .banner("The NVIDIA driver was updated. Restart the computer to finish: until then the GPU can't be read, and games and CUDA may not start.", true)
                    .build());
            }
            State::Failed(e) => return Ok(Page::new("").banner(&format!("nvidia-smi couldn't read the GPU: {}", e.replace('<', "&lt;")), true).build()),
        };
        let g = gpus.into_iter().find(|g| format!("gpu{}", g.index) == id).context("that GPU isn't there any more")?;
        let apps = parse_apps(&smi(&["--query-compute-apps=pid,process_name,used_memory", "--format=csv,noheader,nounits"]).unwrap_or_default());
        if preview_supported() {
            return Ok(describe_pane(&g, &apps));
        }
        let mut now = vec![
            page::info("temp", "Temperature", &one(g.temp, " °C")),
            page::info("util", "In use", &one(g.util, " %")),
            page::info("mem", "Memory", &format!("{} of {}", one(g.mem_used.map(|m| m / 1024.0), " GB"), one(g.mem_total.map(|m| m / 1024.0), " GB"))),
            page::info("power", "Power draw", &one(g.power, " W")),
            page::info("clock", "Clocks", &format!("{} graphics · {} memory", one(g.clock, " MHz"), one(g.mem_clock, " MHz"))),
        ];
        if g.fan.is_some() {
            now.push(desc(page::info("fan", "Fan", &one(g.fan, " %")), "Set by the driver; it can't be changed on Wayland."));
        }
        now.push(page::info("pstate", "Performance state", &format!("{} (P0 is fastest, P8 idle)", g.pstate)));
        let mut control = Vec::new();
        if let (Some(lim), Some(lo), Some(hi)) = (g.limit, g.limit_min, g.limit_max)
            && hi > lo
        {
            control.push(desc(
                page::slider("limit", "Power limit", lim, (lo.ceil(), hi.floor(), 5.0), " W"),
                "Lower runs cooler and quieter, a little slower. Asks for your password; back to the default at restart.",
            ));
        }
        control.push(desc(
            page::switch("persistence", "Keep the driver loaded", g.persistence),
            "Programs start using the GPU faster, at the cost of a little power while it's idle. Asks for your password.",
        ));
        let app_rows: Vec<Value> = if apps.is_empty() {
            vec![page::info("none", "Programs", "None right now")]
        } else {
            apps.iter().map(|(pid, name, mem)| page::info(&format!("app{pid}"), name, &format!("{mem} MB"))).collect()
        };
        let info = vec![page::info("driver", "Driver", &g.driver), page::info("bus", "PCI address", &g.bus)];
        Ok(Page::new(&format!("<b>{}</b> · driver {}", g.name, g.driver))
            .poll(3)
            .group("Now", "", now)
            .group("Programs using it", "", app_rows)
            .group("Control", "", control)
            .group("Card", "", info)
            .build())
    }

    fn set(&self, page: &str, key: &str, value: &str) -> Result<Option<Value>> {
        let index = page.trim_start_matches("gpu");
        if index.parse::<u32>().is_err() {
            bail!("no such GPU");
        }
        match key {
            "limit" => {
                let w = value.parse::<f64>()?.round() as u32;
                smi_root(&["-i", index, "-pl", &w.to_string()])?;
                Ok(nyx_ext::toast(&format!("Power limit {w} W")))
            }
            "persistence" => {
                smi_root(&["-i", index, "-pm", if value == "true" { "1" } else { "0" }])?;
                Ok(None)
            }
            _ => bail!("unknown setting {key}"),
        }
    }
}

fn main() {
    nyx_ext::run(&Nvidia);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_gpus() {
        let t = "0, NVIDIA GeForce RTX 5080 Laptop GPU, 615.78, 45, 12, 1024, 16303, 18.52, 150.00, 80.00, 175.00, 1200, 7000, [N/A], Disabled, P8, 00000000:01:00.0\n";
        let g = &parse(t)[0];
        assert_eq!(g.name, "NVIDIA GeForce RTX 5080 Laptop GPU");
        assert_eq!(g.temp, Some(45.0));
        assert_eq!(g.limit_max, Some(175.0));
        assert_eq!(g.fan, None);
        assert!(!g.persistence);
        assert_eq!(g.pstate, "P8");
        assert!(parse("garbage\n").is_empty());
    }

    #[test]
    fn reads_programs() {
        let a = parse_apps("1234, /usr/bin/ollama, 5300\n");
        assert_eq!(a, vec![("1234".into(), "ollama".into(), "5300".into())]);
        assert_eq!(one(Some(16.0), " GB"), "16 GB");
        assert_eq!(one(None, " W"), "—");
        assert_eq!(megabytes("8840"), "8,840 MB");
        assert_eq!(thousands(212.0), "212");
        assert_eq!(pci("00000000:01:00.0"), "0000:01:00.0");
    }

    /// The pretend GPU, with and without the pane layout. Environment variables are
    /// the whole process's, so this is one test.
    #[test]
    fn pretend_gpu_pages() {
        let state = std::env::temp_dir().join(format!("nvidia-mock-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&state);
        unsafe {
            std::env::set_var("SETTINGS_EXTENSION_STATE", &state);
            std::env::set_var("NVIDIA_MOCK", "rtx-5080-laptop");
            std::env::remove_var("SETTINGS_KINDS");
        }
        assert_eq!(Nvidia.pages()[0]["title"], "GeForce RTX 5080 Laptop GPU");

        let old = Nvidia.describe("gpu0").unwrap();
        assert!(old.get("preview").is_none());
        let titles: Vec<&str> = old["groups"].as_array().unwrap().iter().map(|g| g["title"].as_str().unwrap()).collect();
        assert_eq!(titles, ["Now", "Programs using it", "Control", "Card"]);
        assert!(old["groups"].as_array().unwrap().iter().flat_map(|g| g["rows"].as_array().unwrap()).all(|r| r.get("icon").is_none()));
        assert_eq!(old["groups"][1]["rows"][1], json!({ "kind": "info", "key": "app4410", "title": "nyx-ai-core", "value": "8840 MB" }));

        unsafe { std::env::set_var("SETTINGS_KINDS", "info switch slider preview icon") };
        let new = Nvidia.describe("gpu0").unwrap();
        let groups = new["groups"].as_array().unwrap();
        let titles: Vec<&str> = groups.iter().map(|g| g["title"].as_str().unwrap()).collect();
        assert_eq!(titles, ["Control", "Programs using it"]);
        for row in groups.iter().flat_map(|g| g["rows"].as_array().unwrap()) {
            assert!(row["icon"].as_str().is_some_and(|i| i.ends_with("-symbolic")), "{row}");
        }
        let pv = &new["preview"];
        assert_eq!(pv["title"], "RTX 5080 Laptop GPU");
        assert_eq!(pv["line"], "Driver 580.95 · PCI 0000:01:00.0 · updated every 3 s");
        assert_eq!(pv["blocks"][0], json!({ "kind": "picture", "picture": "gpu" }));
        let stats: Vec<&str> = pv["blocks"][1]["items"].as_array().unwrap().iter().map(|i| i["value"].as_str().unwrap()).collect();
        assert_eq!(stats, ["61 °C", "38%", "9.2 of 16 GB", "84 W"]);
        assert_eq!(pv["blocks"][1]["items"][1]["sub"], "P2 · 1,980 MHz");
        let list = &pv["blocks"][2];
        assert_eq!(list["kind"], "list");
        assert_eq!(list["items"][1]["label"], "nyx-ai-core");
        assert_eq!(list["items"][1]["value"], "8,840 MB");
        assert_eq!(list["items"][1]["sub"], "54% of its memory");

        // Changes land on the pretend GPU only.
        assert!(mock::on());
        Nvidia.set("gpu0", "limit", "120").unwrap();
        Nvidia.set("gpu0", "persistence", "true").unwrap();
        assert!(Nvidia.set("gpu0", "limit", "500").is_err());
        let after = Nvidia.describe("gpu0").unwrap();
        assert_eq!(after["groups"][0]["rows"][0]["value"], 120.0);
        assert_eq!(after["groups"][0]["rows"][1]["value"], true);
        assert_eq!(after["preview"]["blocks"][1]["items"][3]["sub"], "Limit 120 W");
        let _ = std::fs::remove_dir_all(&state);
    }
}
