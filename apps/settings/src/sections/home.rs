//! Home: this computer at a glance, live activity, storage, and every update
//! waiting to be installed.

use crate::backend::sysinfo::{self, CpuTimes};
use crate::backend::{pkgupdates, updates};
use crate::charts::{self, LineChart, Meter, Ring};
use crate::widgets::{self, Page};
use crate::{cmd, units, window};
use gtk::glib;
use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn esc(t: &str) -> String {
    glib::markup_escape_text(t).to_string()
}

fn pct(f: f64) -> String {
    format!("{:.0}%", f * 100.0)
}

/// A card in the controls column or the preview, styled like the preview's cards.
fn card(class: &str, spacing: i32, vertical: bool) -> gtk::Box {
    let c = if vertical { widgets::vbox(spacing) } else { widgets::hbox(spacing) };
    c.add_css_class("cn-hm-card");
    c.add_css_class(class);
    c
}

// ---------- This computer ----------

pub(crate) struct Hero {
    pub(crate) count: gtk::Label,
    pub(crate) caption: gtk::Label,
    pub(crate) badge: gtk::Button,
    uptime: gtk::Label,
}

fn fact(grid: &gtk::Grid, i: i32, title: &str, value: &str) -> gtk::Label {
    let cell = widgets::vbox(1);
    cell.append(&widgets::label(title, "home-fact-title"));
    let v = widgets::label(value, "home-fact");
    v.set_selectable(true);
    // Selectable by mouse, but not a focus stop: a focused label shows a text cursor.
    v.set_focusable(false);
    v.set_ellipsize(gtk::pango::EllipsizeMode::End);
    v.set_tooltip_text(Some(value));
    cell.append(&v);
    grid.attach(&cell, i % 3, i / 3, 1, 1);
    v
}

/// "4.0.0.r6694.g821ae58" → "4.0.0": the build suffix of a development version.
fn short_version(v: &str) -> String {
    match v.split_once(".r") {
        Some((base, rest)) if rest.split_once(".g").is_some_and(|(n, _)| n.chars().all(|c| c.is_ascii_digit())) => {
            base.to_string()
        }
        _ => v.to_string(),
    }
}

/// The computer's card (its name, model and the updates badge) and its facts under it.
fn hero(page: &Page) -> Hero {
    let g = page.plain_group("");
    let me = card("cn-hm-me", 14, false);
    let logo = nyx_ui::widgets::logo(44);
    logo.set_valign(gtk::Align::Center);
    me.append(&logo);
    let names = widgets::vbox(1);
    names.set_hexpand(true);
    names.set_valign(gtk::Align::Center);
    names.append(&widgets::label(&sysinfo::hostname(), "home-host"));
    let model = sysinfo::model();
    if !model.is_empty() {
        let m = widgets::label(&model, "dim");
        // Long model names wrap instead of widening the window.
        m.set_wrap(true);
        m.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        m.set_xalign(0.0);
        names.append(&m);
    }
    me.append(&names);

    let badge = gtk::Button::new();
    badge.add_css_class("home-badge");
    badge.set_valign(gtk::Align::Center);
    let inner = widgets::vbox(0);
    let count = widgets::label("…", "home-badge-count");
    count.set_xalign(0.5);
    let caption = widgets::label("Checking for updates", "home-badge-caption");
    caption.set_xalign(0.5);
    caption.set_wrap(true);
    caption.set_max_width_chars(9);
    caption.set_justify(gtk::Justification::Center);
    inner.append(&count);
    inner.append(&caption);
    badge.set_child(Some(&inner));
    me.append(&badge);
    let about = gtk::Button::with_label("About");
    about.add_css_class("flat");
    about.set_valign(gtk::Align::Center);
    about.connect_clicked(|_| window::navigate("about"));
    me.append(&about);
    g.add(&me);

    let grid = gtk::Grid::new();
    grid.add_css_class("cn-hm-facts");
    grid.set_column_homogeneous(true);
    grid.set_column_spacing(16);
    grid.set_row_spacing(10);
    let omarchy = cmd::output(&["omarchy-version"]).unwrap_or_default();
    let channel = cmd::output(&["omarchy-version-channel"]).unwrap_or_default();
    let full_version = omarchy.clone();
    let omarchy = short_version(&omarchy);
    let omarchy = match (omarchy.is_empty(), channel.is_empty()) {
        (true, _) => "—".to_string(),
        (false, true) => omarchy,
        (false, false) => format!("{omarchy} · {channel}"),
    };
    let hypr = cmd::output(&["hyprctl", "version", "-j"])
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v.get("tag").and_then(|x| x.as_str()).map(String::from))
        .unwrap_or_else(|| "—".into());
    // Without Omarchy (NyxOS, say), the system's own name.
    let system = full_version.is_empty().then(os_name).flatten();
    let v = match &system {
        Some(name) => fact(&grid, 0, "System", name),
        None => fact(&grid, 0, "Omarchy", &omarchy),
    };
    if !full_version.is_empty() {
        v.set_tooltip_text(Some(&full_version));
    }
    fact(&grid, 1, "Kernel", &cmd::output(&["uname", "-r"]).unwrap_or_default());
    fact(&grid, 2, "Processor", &sysinfo::cpu_model());
    fact(&grid, 3, "Memory", &sysinfo::bytes_text(sysinfo::memory().total));
    fact(&grid, 4, "Hyprland", &hypr);
    let uptime = fact(&grid, 5, "Up for", &sysinfo::duration_text(sysinfo::uptime_secs()));
    g.add(&grid);
    Hero { count, caption, badge, uptime }
}

// ---------- Gauges ----------

#[derive(Clone)]
struct Gauge {
    ring: Ring,
    value: gtk::Label,
    detail: gtk::Label,
}

fn gauge(row: &gtk::Box, title: &str, go_to: Option<&'static str>) -> Gauge {
    let cell = widgets::vbox(4);
    cell.add_css_class("cn-hm-gauge");
    cell.set_hexpand(true);
    let ring = charts::ring(64);
    let value = widgets::label("", "cn-hm-gauge-value");
    value.set_xalign(0.5);
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&ring.area));
    overlay.add_overlay(&value);
    overlay.set_halign(gtk::Align::Center);
    overlay.set_margin_bottom(4);
    cell.append(&overlay);
    let t = widgets::label(title, "stat-label");
    t.set_xalign(0.5);
    cell.append(&t);
    let detail = widgets::label("", "stat-detail");
    detail.set_xalign(0.5);
    detail.set_ellipsize(gtk::pango::EllipsizeMode::End);
    detail.set_max_width_chars(14);
    cell.append(&detail);
    if let Some(id) = go_to {
        cell.add_css_class("clickable");
        cell.set_cursor_from_name(Some("pointer"));
        cell.set_tooltip_text(Some("Open its settings"));
        let click = gtk::GestureClick::new();
        click.connect_released(move |_, _, _, _| window::navigate(id));
        cell.add_controller(click);
    }
    row.append(&cell);
    Gauge { ring, value, detail }
}

struct Gauges {
    cpu: Gauge,
    memory: Gauge,
    disk: Gauge,
    battery: Option<Gauge>,
    temp: Option<Gauge>,
}

fn gauges(pv: &widgets::Preview) -> Gauges {
    let row = card("cn-hm-gauges", 4, false);
    row.set_homogeneous(true);
    let cpu = gauge(&row, "Processor", None);
    let memory = gauge(&row, "Memory", None);
    let disk = gauge(&row, "Disk", Some("disks"));
    let battery = sysinfo::battery().map(|_| gauge(&row, "Battery", Some("power")));
    if let Some(b) = &battery {
        b.ring.warn_when_low();
    }
    let temp = sysinfo::cpu_temp().map(|_| gauge(&row, "Temperature", None));
    pv.add(&row);
    Gauges { cpu, memory, disk, battery, temp }
}

// ---------- Activity ----------

/// A titled card holding a chart, with a live readout on the right.
fn chart_card(title: &str, legend: &[&str], chart: &impl IsA<gtk::Widget>) -> (gtk::Box, gtk::Label) {
    let c = card("cn-hm-chart", 8, true);
    let head = widgets::hbox(8);
    let t = widgets::label(title, "cn-hm-chart-title");
    t.set_hexpand(true);
    head.append(&t);
    let value = widgets::label("", "stat-detail");
    value.set_xalign(1.0);
    head.append(&value);
    c.append(&head);
    c.append(chart);
    if !legend.is_empty() {
        let l = widgets::hbox(14);
        for (i, name) in legend.iter().enumerate() {
            let item = widgets::hbox(6);
            let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            dot.add_css_class("legend-dot");
            if i > 0 {
                dot.add_css_class("second");
            }
            dot.set_valign(gtk::Align::Center);
            item.append(&dot);
            item.append(&widgets::label(name, "stat-detail"));
            l.append(&item);
        }
        c.append(&l);
    }
    (c, value)
}

struct Activity {
    cpu: LineChart,
    cpu_value: gtk::Label,
    cores: gtk::Box,
    core_meters: RefCell<Vec<Meter>>,
    memory: LineChart,
    memory_value: gtk::Label,
    net: LineChart,
    net_value: gtk::Label,
}

/// The processor with each core on the left; memory and network stacked beside it.
fn activity(pv: &widgets::Preview) -> Activity {
    let row = widgets::hbox(12);
    row.add_css_class("cn-hm-charts");
    row.set_homogeneous(true);
    row.set_vexpand(true);
    let cpu = charts::line_chart(1, Some(1.0), 0.0, 56);
    let (cpu_card, cpu_value) = chart_card("Processor", &[], &cpu.area);
    let cores = widgets::vbox(6);
    cpu_card.append(&cores);
    row.append(&cpu_card);
    let side = widgets::vbox(12);
    let memory = charts::line_chart(2, Some(1.0), 0.0, 36);
    let (memory_card, memory_value) = chart_card("Memory", &["Memory", "Swap"], &memory.area);
    side.append(&memory_card);
    let net = charts::line_chart(2, None, 64.0 * 1024.0, 36);
    let (net_card, net_value) = chart_card("Network", &["Download", "Upload"], &net.area);
    side.append(&net_card);
    row.append(&side);
    pv.add(&row);
    Activity { cpu, cpu_value, cores, core_meters: RefCell::default(), memory, memory_value, net, net_value }
}

impl Activity {
    /// One meter per core, in two columns.
    fn set_cores(&self, usage: &[f64]) {
        let mut meters = self.core_meters.borrow_mut();
        if meters.len() != usage.len() {
            while let Some(c) = self.cores.first_child() {
                self.cores.remove(&c);
            }
            meters.clear();
            let grid = gtk::Grid::new();
            grid.set_column_homogeneous(true);
            grid.set_column_spacing(16);
            grid.set_row_spacing(5);
            let rows = usage.len().div_ceil(if usage.len() > 8 { 4 } else { 2 });
            for i in 0..usage.len() {
                let line = widgets::hbox(8);
                let l = widgets::label(&format!("{:>2}", i + 1), "stat-detail");
                l.add_css_class("mono");
                line.append(&l);
                let m = charts::meter(5);
                m.area.set_hexpand(true);
                line.append(&m.area);
                grid.attach(&line, (i / rows) as i32, (i % rows) as i32, 1, 1);
                meters.push(m);
            }
            self.cores.append(&grid);
        }
        for (m, u) in meters.iter().zip(usage) {
            m.set(vec![*u]);
        }
    }
}

// ---------- Live sampling ----------

/// "13.2 of 32 GB": the unit once when both share it.
fn of_total(used: u64, total: u64) -> String {
    let (u, t) = (sysinfo::bytes_text(used), sysinfo::bytes_text(total));
    match (u.rsplit_once(' '), t.rsplit_once(' ')) {
        (Some((un, uu)), Some((_, tu))) if uu == tu => format!("{un} of {t}"),
        _ => format!("{u} of {t}"),
    }
}

struct Sampler {
    cpu: Vec<CpuTimes>,
    net: (u64, u64),
    at: Instant,
}

fn sample(s: &mut Sampler, gauges: &Gauges, act: &Activity, hero: &Hero) {
    charts::refresh_palette();
    let cpu = sysinfo::cpu_times();
    let usage: Vec<f64> = cpu.iter().zip(&s.cpu).map(|(a, b)| a.usage_since(b)).collect();
    s.cpu = cpu;
    if let Some(total) = usage.first() {
        gauges.cpu.ring.set(*total);
        gauges.cpu.value.set_label(&pct(*total));
        act.cpu.push(&[*total]);
        act.cpu_value.set_label(&pct(*total));
        act.set_cores(&usage[1..]);
    }
    gauges.cpu.detail.set_label(&format!("{} cores", usage.len().saturating_sub(1)));

    let m = sysinfo::memory();
    let frac = |a: u64, b: u64| if b == 0 { 0.0 } else { a as f64 / b as f64 };
    let mem = frac(m.used, m.total);
    gauges.memory.ring.set(mem);
    gauges.memory.value.set_label(&pct(mem));
    gauges.memory.detail.set_label(&of_total(m.used, m.total));
    act.memory.push(&[mem, frac(m.swap_used, m.swap_total)]);
    act.memory_value.set_label(&format!("{} used", sysinfo::bytes_text(m.used)));

    let net = sysinfo::net_bytes();
    let secs = s.at.elapsed().as_secs_f64().max(0.001);
    let down = net.0.saturating_sub(s.net.0) as f64 / secs;
    let up = net.1.saturating_sub(s.net.1) as f64 / secs;
    s.net = net;
    s.at = Instant::now();
    act.net.push(&[down, up]);
    act.net_value.set_label(&format!("↓ {} · ↑ {}", sysinfo::rate_text(down), sysinfo::rate_text(up)));

    if let (Some(t), Some(b)) = (&gauges.battery, sysinfo::battery()) {
        t.ring.set(b.percent as f64 / 100.0);
        t.value.set_label(&format!("{}%", b.percent));
        t.detail.set_label(&b.status);
        t.detail.set_tooltip_text(b.health.map(|h| format!("{} · {h}% health", b.status)).as_deref());
    }
    if let (Some(t), Some(c)) = (&gauges.temp, sysinfo::cpu_temp()) {
        t.ring.set(c / 100.0);
        t.value.set_label(&format!("{:.0}°", units::from_celsius(c)));
        t.detail.set_label("Processor");
    }
    hero.uptime.set_label(&sysinfo::duration_text(sysinfo::uptime_secs()));
}

/// Sample once a second, only while the page is on screen.
fn start_sampling(page: &Page, gauges: Gauges, act: Activity, hero: Rc<Hero>) {
    let state = Rc::new(RefCell::new(Sampler { cpu: sysinfo::cpu_times(), net: sysinfo::net_bytes(), at: Instant::now() }));
    let parts = Rc::new((gauges, act));
    // Fill everything in straight away (CPU since boot until the first tick).
    {
        let mut s = state.borrow_mut();
        let since_boot = s.cpu.iter().map(|_| CpuTimes::default()).collect();
        s.cpu = since_boot;
        sample(&mut s, &parts.0, &parts.1, &hero);
    }
    let running = Rc::new(std::cell::Cell::new(false));
    let root = page.root.clone();
    let tick = move |root: &gtk::ScrolledWindow| {
        if running.replace(true) {
            return;
        }
        let (state, parts, hero, running, root) = (state.clone(), parts.clone(), hero.clone(), running.clone(), root.clone());
        glib::timeout_add_local(Duration::from_secs(1), move || {
            if !root.is_mapped() {
                running.set(false);
                return glib::ControlFlow::Break;
            }
            sample(&mut state.borrow_mut(), &parts.0, &parts.1, &hero);
            glib::ControlFlow::Continue
        });
    };
    root.connect_map(tick.clone());
    if root.is_mapped() {
        tick(&root);
    }
}

// ---------- Storage ----------

/// Every disk, and the package count, in one card. Also fills the disk gauge.
fn storage(pv: &widgets::Preview, gauge: Gauge) {
    let c = card("cn-hm-storage", 10, true);
    // A drive reporting trouble comes first.
    for d in crate::backend::udisks::load().unwrap_or_default().iter().filter(|d| d.health.supported && !d.health.ok()) {
        c.append(&widgets::banner(
            &format!("{} reports a problem with its health. Back up what's on it, then see Disks.", d.title()),
            true,
        ));
    }
    let disks = gtk::Button::with_label("Disks");
    disks.connect_clicked(|_| crate::window::navigate("disks"));
    let head = widgets::icon_row(
        "drive-harddisk-symbolic",
        "Storage",
        "Partitions, formatting, drive health and cleaning up space.",
        Some(disks.upcast_ref()),
    );
    head.add_css_class("cn-hm-storage-head");
    widgets::keywords("disks partitions formatting drive health cleaning space");
    // The description becomes the summary once the disks are read.
    let summary = head.first_child().and_then(|t| t.next_sibling()).and_then(|t| t.last_child()).and_downcast::<gtk::Label>();
    c.append(&head);
    let list = gtk::Grid::new();
    list.add_css_class("cn-hm-disks");
    list.set_column_spacing(14);
    list.set_row_spacing(10);
    c.append(&list);
    pv.add(&c);
    cmd::background(
        || (sysinfo::disks(), sysinfo::packages()),
        move |(disks, pk)| {
            let mut parts = Vec::new();
            if let Some(d) = disks.iter().find(|d| d.mount == "/").or(disks.first()) {
                let f = if d.size == 0 { 0.0 } else { d.used as f64 / d.size as f64 };
                gauge.ring.set(f);
                gauge.value.set_label(&pct(f));
                gauge.detail.set_label(&format!("{} free", sysinfo::bytes_text(d.size.saturating_sub(d.used))));
                parts.push(format!("{} used of {}", sysinfo::bytes_text(d.used), sysinfo::bytes_text(d.size)));
            }
            if pk.total > 0 {
                parts.push(format!("{} packages", pk.total));
            }
            if let Some(l) = &summary
                && !parts.is_empty()
            {
                l.set_text(&parts.join(" · "));
            }
            widgets::begin_section("home");
            let mut n = 0;
            let mut line = |name: &str, m: &charts::Meter, short: &str, desc: &str| {
                let l = widgets::label(name, "cn-hm-disk-name");
                l.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
                l.set_max_width_chars(14);
                l.set_tooltip_text(Some(desc));
                list.attach(&l, 0, n, 1, 1);
                m.area.set_hexpand(true);
                m.area.set_valign(gtk::Align::Center);
                m.area.set_tooltip_text(Some(desc));
                list.attach(&m.area, 1, n, 1, 1);
                let v = widgets::label(short, "stat-detail");
                v.set_xalign(1.0);
                list.attach(&v, 2, n, 1, 1);
                n += 1;
            };
            for d in disks {
                let m = charts::meter(6);
                let used = if d.size == 0 { 0.0 } else { d.used as f64 / d.size as f64 };
                m.set(vec![used]);
                let desc = format!(
                    "{} of {} used · {} free",
                    sysinfo::bytes_text(d.used),
                    sysinfo::bytes_text(d.size),
                    sysinfo::bytes_text(d.size.saturating_sub(d.used))
                );
                line(&d.mount, &m, &format!("{} free", sysinfo::bytes_text(d.size.saturating_sub(d.used))), &desc);
                widgets::keywords("disk storage space free full drive partition");
            }
            if pk.total > 0 {
                let m = charts::meter(6);
                m.never_warn();
                let f = pk.explicit as f64 / pk.total as f64;
                m.set(vec![f, 1.0 - f]);
                let desc = format!(
                    "{} installed · {} by name, {} as dependencies · {} from the AUR",
                    pk.total,
                    pk.explicit,
                    pk.total - pk.explicit,
                    pk.foreign
                );
                line("Packages", &m, &format!("{} from the AUR", pk.foreign), &desc);
                widgets::keywords("pacman aur installed software");
            }
        },
    );
}

// ---------- Updates ----------

/// One card listing packages as name, old → new.
fn package_list(pkgs: &[pkgupdates::Pkg]) -> gtk::Grid {
    let grid = gtk::Grid::new();
    grid.add_css_class("update-list");
    grid.set_column_spacing(16);
    grid.set_row_spacing(6);
    for (i, p) in pkgs.iter().enumerate() {
        let i = i as i32;
        // Long names and versions shorten, so the list never widens the column.
        let name = widgets::label(&p.name, "update-name");
        name.set_hexpand(true);
        name.set_selectable(true);
        name.set_ellipsize(gtk::pango::EllipsizeMode::End);
        name.set_tooltip_text(Some(&p.name));
        grid.attach(&name, 0, i, 1, 1);
        let old = widgets::label(&p.old, "mono");
        old.add_css_class("dim");
        old.set_xalign(1.0);
        old.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        old.set_max_width_chars(14);
        old.set_tooltip_text(Some(&p.old));
        grid.attach(&old, 1, i, 1, 1);
        grid.attach(&widgets::label("→", "dim"), 2, i, 1, 1);
        let new = widgets::label(&p.new, "mono");
        new.add_css_class("accent-text");
        new.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        new.set_max_width_chars(14);
        new.set_tooltip_text(Some(&p.new));
        grid.attach(&new, 3, i, 1, 1);
    }
    grid
}

fn list_block(list: &gtk::Box, title: &str, desc: &str, pkgs: &[pkgupdates::Pkg], open: bool) {
    if pkgs.is_empty() {
        return;
    }
    let (wrap, content) = widgets::disclosure(&format!("{title} ({})", pkgs.len()), desc);
    let names: Vec<&str> = pkgs.iter().map(|p| p.name.as_str()).collect();
    widgets::keywords(&names.join(" "));
    content.append(&package_list(pkgs));
    if open {
        content.set_visible(true);
        if let Some(arrow) =
            wrap.first_child().and_then(|b| b.first_child()).and_then(|i| i.first_child()).and_downcast::<gtk::Image>()
        {
            arrow.set_icon_name(Some("pan-down-symbolic"));
        }
    }
    list.append(&wrap);
}

struct UpdatesUi {
    list: gtk::Box,
    hero: Rc<Hero>,
}

fn show_updates(ui: &Rc<UpdatesUi>, s: &pkgupdates::Status, checking: bool) {
    let list = &ui.list;
    // Filled after the page is built, so not through Group::add: show the card here.
    list.set_visible(true);
    widgets::forget_rows(list);
    while let Some(c) = list.first_child() {
        list.remove(&c);
    }
    widgets::begin_section("home");

    let app = updates::cached();
    let ext_count = app.extensions.len();
    let total = s.count() + ext_count + usize::from(app.settings_update());

    // The hero badge and the sidebar.
    let h = &ui.hero;
    if checking && s.checked == 0 {
        h.count.set_label("…");
        h.caption.set_label("Checking for updates");
    } else if total == 0 {
        h.count.set_label("✓");
        h.caption.set_label("Up to date");
    } else {
        h.count.set_label(&total.to_string());
        h.caption.set_label(if total == 1 { "update available" } else { "updates available" });
    }
    if total == 0 {
        h.badge.remove_css_class("pending")
    } else {
        h.badge.add_css_class("pending")
    }
    window::set_nav_badge("home", (total > 0).then(|| total.to_string()).as_deref());

    let title = if checking {
        "Checking for updates…".to_string()
    } else if total == 0 {
        "Everything is up to date".to_string()
    } else if total == 1 {
        "1 update available".to_string()
    } else {
        format!("{total} updates available")
    };
    let mut desc =
        if s.checked == 0 { "Not checked yet.".into() } else { format!("Checked {}.", units::ago((now() - s.checked) as i64)) };
    for e in &s.errors {
        desc.push_str(&format!(" {}.", esc(e)));
    }
    let buttons = widgets::hbox(6);
    let check = widgets::icon_button("view-refresh-symbolic", if checking { "Checking…" } else { "Check now" });
    check.set_sensitive(!checking);
    {
        let ui = ui.clone();
        check.connect_clicked(move |_| run_check(&ui, true));
    }
    buttons.append(&check);
    if s.count() > 0 {
        let b = nyx_ui::widgets::labeled_button("folder-download-symbolic", "Update");
        b.add_css_class("suggested-action");
        b.set_tooltip_text(Some("Runs omarchy-update in a terminal"));
        b.connect_clicked(|_| cmd::spawn(&["omarchy-launch-floating-terminal-with-presentation", "omarchy-update"]));
        buttons.append(&b);
    }
    let r = widgets::icon_row("software-update-available-symbolic", &title, &desc, Some(buttons.upcast_ref()));
    widgets::keywords("update upgrade check packages pacman update now");
    list.append(&r);

    if let Some(o) = &s.omarchy {
        let (r, _) = widgets::button_row("Omarchy update", &esc(o), "Release notes", |_| {
            cmd::spawn(&["xdg-open", "https://github.com/basecamp/omarchy/releases"]);
        });
        widgets::tag_row(&r, "New");
        widgets::keywords("omarchy version release");
        list.append(&r);
    }
    if app.settings_update() || ext_count > 0 {
        let what = match (app.settings_update(), ext_count) {
            (true, 0) => format!("Settings {} is available.", app.latest),
            (true, n) => format!("Settings {} and {n} extension update(s) are available.", app.latest),
            (false, n) => format!("{n} extension update(s) are available."),
        };
        let (r, _) = widgets::button_row("Settings & extensions", &what, "Open About", |_| window::navigate("about"));
        widgets::keywords("settings app extensions");
        list.append(&r);
    }
    list_block(list, "System packages", "From the Arch and Omarchy repositories.", &s.system, s.system.len() <= 8);
    list_block(list, "AUR", "Built from the Arch User Repository.", &s.aur, s.aur.len() <= 8);
    list_block(list, "Firmware", "Device firmware from LVFS (fwupd).", &s.firmware, true);
}

fn run_check(ui: &Rc<UpdatesUi>, force: bool) {
    show_updates(ui, &pkgupdates::cached(), true);
    let ui = ui.clone();
    cmd::background(
        move || {
            let app = force.then(|| updates::check(true));
            (pkgupdates::check(force), app)
        },
        move |(s, app)| {
            show_updates(&ui, &s, false);
            if let Some(app) = app {
                window::after_update_check(app);
            }
        },
    );
}

pub(crate) fn updates_group(page: &Page, hero: Rc<Hero>) {
    let g = page.group("Updates");
    let ui = Rc::new(UpdatesUi { list: g.list.clone(), hero: hero.clone() });
    let cached = pkgupdates::cached();
    if pkgupdates::stale(&cached) {
        run_check(&ui, false);
    } else {
        show_updates(&ui, &cached, false);
    }
    let (root, target) = (page.root.clone(), g.wrapper.clone());
    hero.badge.connect_clicked(move |_| {
        let scroller = widgets::scroller_for(target.upcast_ref()).unwrap_or_else(|| root.clone());
        if let Some(child) = scroller.child()
            && let Some(p) = target.compute_point(&child, &gtk::graphene::Point::new(0.0, 0.0))
        {
            scroller.vadjustment().set_value(p.y() as f64);
        }
    });
}

// ---------- Quick actions ----------

fn quick_actions(page: &Page) {
    let g = page.plain_group("Quick actions");
    // A flow, so the buttons go two by two in a narrow column.
    let row = gtk::FlowBox::new();
    row.set_selection_mode(gtk::SelectionMode::None);
    row.set_homogeneous(true);
    row.set_min_children_per_line(2);
    row.set_max_children_per_line(4);
    row.set_column_spacing(8);
    row.set_row_spacing(8);
    row.add_css_class("cn-hm-actions");
    let lock = nyx_ui::widgets::labeled_button("system-lock-screen-symbolic", "Lock");
    lock.connect_clicked(|_| cmd::spawn(crate::platform::lock_command()));
    row.insert(&lock, -1);
    // Omarchy's snapshots; NyxOS rolls back to an earlier system (Updates) instead.
    if crate::platform::PLATFORM == crate::platform::Platform::Omarchy {
        let snap = nyx_ui::widgets::labeled_button("document-open-recent-symbolic", "Snapshot");
        snap.set_tooltip_text(Some("Save the system as it is now, so you can roll back"));
        snap.connect_clicked(|b| {
            b.set_sensitive(false);
            let b = b.clone();
            cmd::run_async(&["omarchy-snapshot", "create"], move |r| {
                b.set_sensitive(true);
                match r {
                    Ok(_) => window::toast("Snapshot created"),
                    Err(e) => window::toast(&format!("{e}")),
                }
            });
        });
        row.insert(&snap, -1);
    }
    let restart = widgets::confirm_button("Restart", "Click again to restart", |_| cmd::spawn(crate::platform::session_command("reboot")));
    restart.add_css_class("cn-hm-danger");
    row.insert(&restart, -1);
    let off = widgets::confirm_button("Shut down", "Click again to shut down", |_| cmd::spawn(crate::platform::session_command("shutdown")));
    off.add_css_class("cn-hm-danger");
    row.insert(&off, -1);
    let mut c = row.first_child();
    while let Some(child) = c {
        child.set_focusable(false);
        c = child.next_sibling();
    }
    g.add(&row);
    widgets::keywords("snapshot lock restart reboot shut down power off");
}

// ---------- Quick settings ----------

/// The switches people reach for most, at the top: Do Not Disturb and the power profile.
fn quick_settings(page: &Page) {
    let profiles = crate::platform::power_profiles();
    let has_dnd = cmd::present("omarchy-shell");
    if profiles.is_empty() && !has_dnd {
        return;
    }
    let g = page.group("Quick settings");
    if has_dnd {
        let (r, _) = widgets::switch_row(
            "Do not disturb",
            "Silence notifications until you turn it off.",
            crate::backend::notify::dnd(),
            |on| {
                cmd::background(
                    move || crate::backend::notify::set_dnd(on),
                    |r| {
                        if let Err(e) = r {
                            window::toast(&format!("Couldn't change Do not disturb: {e}"));
                        }
                    },
                );
            },
        );
        widgets::keywords("dnd silence notifications quiet");
        r.prepend(&widgets::tile("notifications-disabled-symbolic"));
        r.add_css_class("with-tile");
        g.add(&r);
    }
    if !profiles.is_empty() {
        let current = cmd::output(&["powerprofilesctl", "get"]).unwrap_or_default();
        let label = |p: &str| match p {
            "power-saver" => "Saver".to_string(),
            "balanced" => "Balanced".to_string(),
            "performance" => "Fast".to_string(),
            other => other.to_string(),
        };
        let options: Vec<(String, String)> = profiles.iter().map(|p| (p.clone(), label(p))).collect();
        let seg = widgets::segmented(&options, &current, |p| {
            cmd::run_async(&["powerprofilesctl", "set", &p], |r| {
                if let Err(e) = r {
                    window::toast(&format!("{e}"));
                }
            });
        });
        g.add(&widgets::icon_row(
            "power-profile-balanced-symbolic",
            "Power",
            "For now",
            Some(seg.upcast_ref()),
        ));
        widgets::keywords("power profile performance balanced power saver battery power source");
    }
}

pub fn build(page: &Page) {
    // The live picture on the right: gauges, activity and storage.
    let pv = page.split();
    pv.head("Right now", "The last minute, updated every second while Home is open.", &[]);
    let gauges = gauges(&pv);
    let disk = gauges.disk.clone();
    let act = activity(&pv);
    storage(&pv, disk);

    // What you'd do, on the left: this computer, quick settings, updates, the session.
    let hero = Rc::new(hero(page));
    quick_settings(page);
    crate::platform::home_updates(page, hero.clone());
    quick_actions(page);
    start_sampling(page, gauges, act, hero);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert_eq!(short_version("4.0.0.r6694.g821ae58"), "4.0.0");
        assert_eq!(short_version("3.1.2"), "3.1.2");
    }
}

/// `PRETTY_NAME` from /etc/os-release.
fn os_name() -> Option<String> {
    let text = std::fs::read_to_string("/etc/os-release").ok()?;
    let name = text.lines().find_map(|l| l.strip_prefix("PRETTY_NAME="))?.trim_matches('"').to_string();
    (!name.is_empty()).then_some(name)
}
