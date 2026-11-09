use crate::backend::audio::{self, Eq};
use crate::backend::{devices, store, streams};
use crate::widgets::{self, Page};
use crate::{cmd, window};
use gtk::glib;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

fn freq_label(f: u32) -> String {
    if f >= 1000 { format!("{}k", f / 1000) } else { f.to_string() }
}

struct Fader {
    scale: gtk::Scale,
    value: gtk::Label,
}

fn fader(label: &str, range: (f64, f64), value: f64, preamp: bool) -> (gtk::Box, Fader) {
    let b = widgets::vbox(6);
    b.add_css_class("fader");
    b.set_hexpand(true);
    if preamp {
        b.add_css_class("preamp");
    }
    let v = gtk::Label::new(Some(&fmt_db(value)));
    v.add_css_class("fader-value");
    v.add_css_class("mono");
    let scale = gtk::Scale::with_range(gtk::Orientation::Vertical, range.0, range.1, 0.5);
    scale.add_css_class("fader-scale");
    scale.set_inverted(true);
    scale.set_draw_value(false);
    scale.set_value(value);
    scale.set_vexpand(true);
    scale.set_halign(gtk::Align::Center);
    // Fine steps with the keyboard, snap to 0.5 dB when dragging.
    scale.set_increments(0.5, 3.0);
    let l = gtk::Label::new(Some(label));
    l.add_css_class("fader-label");
    b.append(&v);
    b.append(&scale);
    b.append(&l);
    (b, Fader { scale, value: v })
}

fn fmt_db(v: f64) -> String {
    if v.abs() < 0.05 { "0".into() } else { format!("{v:+.1}") }
}

/// Push the equalizer to the running graph shortly after the last change, and
/// save it a little later still.
struct Applier {
    eq: Rc<RefCell<Eq>>,
    live: Cell<Option<glib::SourceId>>,
    persist: Cell<Option<glib::SourceId>>,
    busy: Cell<bool>,
    again: Cell<bool>,
}

impl Applier {
    fn changed(self: &Rc<Self>) {
        if let Some(id) = self.live.take() {
            id.remove();
        }
        let me = self.clone();
        self.live.set(Some(glib::timeout_add_local_once(std::time::Duration::from_millis(40), move || {
            me.live.set(None);
            me.push();
        })));
        if let Some(id) = self.persist.take() {
            id.remove();
        }
        let me = self.clone();
        self.persist.set(Some(glib::timeout_add_local_once(std::time::Duration::from_millis(600), move || {
            me.persist.set(None);
            let eq = me.eq.borrow().clone();
            cmd::background(
                move || {
                    audio::save(&eq)?;
                    // Keep the on-disk graph in step so a restart comes back the same.
                    if eq.enabled && audio::running() {
                        cmd::atomic_write(
                            &audio::conf_file(),
                            audio::generate_conf(&eq, &eq.target, audio::limiter_available()),
                        )?;
                    }
                    anyhow::Ok(())
                },
                |r| {
                    if let Err(e) = r {
                        window::toast(&format!("Couldn't save the equalizer: {e}"));
                    }
                },
            );
        })));
    }

    fn push(self: &Rc<Self>) {
        if self.busy.get() {
            self.again.set(true);
            return;
        }
        self.busy.set(true);
        let eq = self.eq.borrow().clone();
        let me = self.clone();
        cmd::background(
            move || audio::set_live(&eq),
            move |r| {
                me.busy.set(false);
                if let Err(e) = r {
                    eprintln!("nyx-settings: {e:#}");
                }
                if me.again.replace(false) {
                    me.push();
                }
            },
        );
    }
}

fn headroom_text(eq: &Eq) -> (String, &'static str) {
    let peak = audio::peak_boost(eq);
    let limiter = audio::limiter_available();
    if !eq.enabled {
        return ("Bypassed — sound passes through unchanged.".into(), "headroom-ok");
    }
    if peak <= 0.0 {
        ("No boost — plenty of headroom.".into(), "headroom-ok")
    } else if limiter {
        let class = if peak > 15.0 { "headroom-warn" } else { "headroom-ok" };
        (format!("Up to {peak:+.1} dB louder. The limiter keeps loud peaks from distorting."), class)
    } else if peak > 6.0 {
        (format!("Up to {peak:+.1} dB louder. Without a limiter, loud music may distort."), "headroom-hot")
    } else {
        (format!("Up to {peak:+.1} dB louder."), "headroom-warn")
    }
}

pub fn build(page: &Page) {
    let eq = Rc::new(RefCell::new(audio::load()));
    let running = audio::running();
    let default_sink = audio::default_sink();
    let routed = default_sink == audio::SINK;

    // ----- Status banner -----
    if eq.borrow().enabled && !running {
        let b = page.banner("<b>The equalizer isn't running.</b> Turn it on to use the preamp boost and the equalizer.", false);
        let start = gtk::Button::with_label("Turn on");
        start.add_css_class("suggested-action");
        start.set_valign(gtk::Align::Center);
        let eq2 = eq.clone();
        start.connect_clicked(move |btn| {
            btn.set_sensitive(false);
            let e = eq2.borrow().clone();
            cmd::background(
                move || audio::install_and_start(&e),
                |r| {
                    match r {
                        Ok(()) => window::toast("Equalizer on"),
                        Err(e) => window::toast(&format!("Couldn't start the equalizer: {e}")),
                    }
                    window::rebuild("audio");
                },
            );
        });
        b.append(&start);
    } else if running && !routed {
        let b = page.banner("<b>Sound isn't going through the equalizer.</b> Another output was chosen as the default.", false);
        let use_it = gtk::Button::with_label("Use the equalizer");
        use_it.set_valign(gtk::Align::Center);
        let eq2 = eq.clone();
        use_it.connect_clicked(move |_| {
            let mut e = eq2.borrow().clone();
            e.target = audio::default_sink();
            cmd::background(
                move || audio::install_and_start(&e),
                |r| {
                    if let Err(e) = r {
                        window::toast(&format!("{e}"));
                    }
                    window::rebuild("audio");
                },
            );
        });
        b.append(&use_it);
    }

    // ----- Output -----
    let g = page.group("Output");
    let sinks = audio::hardware_sinks();
    let current_out = if routed { eq.borrow().target.clone() } else { default_sink.clone() };
    let options: Vec<(String, String)> = sinks.iter().map(|s| (s.name.clone(), s.description.clone())).collect();
    let eq2 = eq.clone();
    let (r, _) = widgets::choice_row("Output device", "", options, &current_out, move |name| {
        let using_eq = audio::running() && audio::default_sink() == audio::SINK;
        if using_eq {
            let mut e = eq2.borrow().clone();
            e.target = name;
            *eq2.borrow_mut() = e.clone();
            cmd::background(
                move || audio::install_and_start(&e),
                |r| {
                    if let Err(e) = r {
                        window::toast(&format!("Couldn't switch output: {e}"));
                    }
                },
            );
        } else {
            cmd::spawn(&["pactl", "set-default-sink", &name]);
        }
    });
    widgets::keywords("speakers headphones hdmi output device");
    g.add(&r);
    if let Some(dev) = sinks.iter().find(|s| s.name == current_out) {
        g.add(&prefer_row(&dev.name, "output", &dev.description));
    }

    let max = store::read(|s| s.max_volume).unwrap_or(100);
    let vol_sink = audio::volume_sink();
    let vol = audio::volume_percent(&vol_sink).unwrap_or(50) as f64;
    let (vol_row, vol_slider) = widgets::slider_row("Volume", "", (0.0, max as f64, 1.0), vol.min(max as f64), 0, "%", |v| {
        let sink = audio::volume_sink();
        cmd::spawn(&["pactl", "set-sink-volume", &sink, &format!("{}%", v.round() as i64)]);
    });
    let mute = gtk::ToggleButton::new();
    mute.set_icon_name(if audio::muted(&vol_sink) { "audio-volume-muted-symbolic" } else { "audio-volume-high-symbolic" });
    mute.set_active(audio::muted(&vol_sink));
    mute.set_tooltip_text(Some("Mute"));
    mute.connect_toggled(|b| {
        let sink = audio::volume_sink();
        cmd::spawn(&["pactl", "set-sink-mute", &sink, if b.is_active() { "1" } else { "0" }]);
        b.set_icon_name(if b.is_active() { "audio-volume-muted-symbolic" } else { "audio-volume-high-symbolic" });
    });
    mute.set_valign(gtk::Align::Center);
    vol_row.append(&mute);
    g.add(&vol_row);

    let scale = vol_slider.scale.clone();
    let (r, _) = widgets::slider_row(
        "Maximum volume",
        "Let the volume go past 100%. The volume keys and the slider above follow this.",
        (100.0, 150.0, 5.0),
        max as f64,
        0,
        "%",
        move |v| {
            let m = v.round() as u32;
            scale.set_range(0.0, m as f64);
            store::update(true, |s| s.max_volume = if m > 100 { Some(m) } else { None });
        },
    );
    widgets::keywords("loud louder boost over amplify 150");
    g.add(&r);

    // ----- Equalizer -----
    // No card around it: the equalizer panel is its own frame.
    let g = page.plain_group("Equalizer & preamp");
    g.note(
        "The <b>preamp</b> makes everything louder before it reaches the speakers — use it when even full volume is too \
         quiet. The nine bands shape the tone.",
    );

    let panel = widgets::vbox(12);
    panel.add_css_class("eq-panel");
    if !eq.borrow().enabled {
        panel.add_css_class("bypassed");
    }

    // Top bar: enable, preset, actions.
    let top = widgets::hbox(10);
    let enable = gtk::Switch::new();
    enable.set_active(eq.borrow().enabled && running);
    enable.set_valign(gtk::Align::Center);
    top.append(&enable);
    let title = widgets::label("Equalizer", "settings-option-title");
    title.set_hexpand(true);
    top.append(&title);

    let builtin = audio::builtin_presets();
    let user = audio::user_presets();
    let mut preset_opts: Vec<(String, String)> = builtin.iter().map(|(id, n, _)| (id.to_string(), n.to_string())).collect();
    for p in &user.preset {
        preset_opts.push((format!("user:{}", p.name), p.name.clone()));
    }
    preset_opts.push(("custom".into(), "Custom".into()));
    let preset_dd = widgets::dropdown(&preset_opts, &eq.borrow().preset);
    top.append(&preset_dd);
    let save_btn = gtk::Button::from_icon_name("document-save-symbolic");
    save_btn.set_tooltip_text(Some("Save as a preset"));
    top.append(&save_btn);
    let flat_btn = gtk::Button::with_label("Reset");
    flat_btn.set_tooltip_text(Some("All bands to 0 dB, preamp to +6 dB"));
    top.append(&flat_btn);
    panel.append(&top);

    // Faders.
    let row = widgets::hbox(4);
    let (pre_box, pre) = fader("PREAMP", (audio::PREAMP_RANGE.0, audio::preamp_max()), eq.borrow().preamp_db, true);
    row.append(&pre_box);
    let div = gtk::Separator::new(gtk::Orientation::Vertical);
    div.add_css_class("eq-divider");
    row.append(&div);
    let mut bands = Vec::new();
    for (i, f) in audio::FREQUENCIES.iter().enumerate() {
        let (b, fd) = fader(&freq_label(*f), audio::GAIN_RANGE, eq.borrow().gains[i], false);
        row.append(&b);
        bands.push(fd);
    }
    panel.append(&row);

    let (text, class) = headroom_text(&eq.borrow());
    let headroom = widgets::label(&text, class);
    headroom.set_halign(gtk::Align::Center);
    panel.append(&headroom);
    if !audio::limiter_available() {
        let note = widgets::label(
            "Install lsp-plugins-lv2 for a limiter, which allows up to +24 dB of preamp without distortion.",
            "dim",
        );
        note.set_wrap(true);
        note.set_halign(gtk::Align::Center);
        panel.append(&note);
    }

    let applier = Rc::new(Applier {
        eq: eq.clone(),
        live: Cell::new(None),
        persist: Cell::new(None),
        busy: Cell::new(false),
        again: Cell::new(false),
    });
    let guard = Rc::new(Cell::new(false));

    let refresh_headroom = {
        let eq = eq.clone();
        let headroom = headroom.clone();
        move || {
            let (text, class) = headroom_text(&eq.borrow());
            headroom.set_text(&text);
            for c in ["headroom-ok", "headroom-warn", "headroom-hot"] {
                headroom.remove_css_class(c);
            }
            headroom.add_css_class(class);
        }
    };
    let custom_index = preset_opts.iter().position(|(id, _)| id == "custom").unwrap_or(0) as u32;

    {
        let eq = eq.clone();
        let applier = applier.clone();
        let label = pre.value.clone();
        let guard = guard.clone();
        let rh = refresh_headroom.clone();
        pre.scale.connect_value_changed(move |s| {
            let v = (s.value() * 2.0).round() / 2.0;
            label.set_text(&fmt_db(v));
            if guard.get() {
                return;
            }
            eq.borrow_mut().preamp_db = v;
            rh();
            applier.changed();
        });
    }
    for (i, band) in bands.iter().enumerate() {
        let eq = eq.clone();
        let applier = applier.clone();
        let label = band.value.clone();
        let guard = guard.clone();
        let dd = preset_dd.clone();
        let rh = refresh_headroom.clone();
        band.scale.connect_value_changed(move |s| {
            let v = (s.value() * 2.0).round() / 2.0;
            label.set_text(&fmt_db(v));
            if guard.get() {
                return;
            }
            {
                let mut e = eq.borrow_mut();
                e.gains[i] = v;
                e.preset = "custom".into();
            }
            guard.set(true);
            dd.set_selected(custom_index);
            guard.set(false);
            rh();
            applier.changed();
        });
    }

    let set_all = {
        let guard = guard.clone();
        let pre_scale = pre.scale.clone();
        let band_scales: Vec<gtk::Scale> = bands.iter().map(|b| b.scale.clone()).collect();
        move |preamp: Option<f64>, gains: [f64; 9]| {
            guard.set(true);
            if let Some(p) = preamp {
                pre_scale.set_value(p);
            }
            for (s, g) in band_scales.iter().zip(gains) {
                s.set_value(g);
            }
            guard.set(false);
        }
    };

    {
        let eq = eq.clone();
        let applier = applier.clone();
        let guard = guard.clone();
        let set_all = set_all.clone();
        let rh = refresh_headroom.clone();
        let opts2 = preset_opts.clone();
        preset_dd.connect_selected_notify(move |dd| {
            if guard.get() {
                return;
            }
            let Some((id, _)) = opts2.get(dd.selected() as usize).cloned() else { return };
            let (gains, preamp) = if let Some(name) = id.strip_prefix("user:") {
                match audio::user_presets().preset.into_iter().find(|p| p.name == name) {
                    Some(p) => (p.gains, p.preamp_db),
                    None => return,
                }
            } else if let Some((_, _, g)) = audio::builtin_presets().into_iter().find(|(pid, _, _)| *pid == id) {
                (g, None)
            } else {
                return;
            };
            {
                let mut e = eq.borrow_mut();
                e.gains = gains;
                if let Some(p) = preamp {
                    e.preamp_db = p;
                }
                e.preset = id;
            }
            set_all(preamp, gains);
            rh();
            applier.changed();
        });
    }

    {
        let eq = eq.clone();
        let applier = applier.clone();
        let set_all = set_all.clone();
        let rh = refresh_headroom.clone();
        let dd = preset_dd.clone();
        let guard = guard.clone();
        flat_btn.connect_clicked(move |_| {
            {
                let mut e = eq.borrow_mut();
                e.gains = [0.0; 9];
                e.preamp_db = Eq::default().preamp_db;
                e.preset = "flat".into();
            }
            set_all(Some(Eq::default().preamp_db), [0.0; 9]);
            guard.set(true);
            dd.set_selected(0);
            guard.set(false);
            rh();
            applier.changed();
        });
    }

    {
        let eq = eq.clone();
        save_btn.connect_clicked(move |btn| {
            let pop = gtk::Popover::new();
            let bx = widgets::hbox(6);
            let entry = gtk::Entry::new();
            entry.set_placeholder_text(Some("Preset name"));
            let ok = gtk::Button::with_label("Save");
            ok.add_css_class("suggested-action");
            bx.append(&entry);
            bx.append(&ok);
            pop.set_child(Some(&bx));
            pop.set_parent(btn);
            let eq = eq.clone();
            let p2 = pop.clone();
            let save = move |name: String| {
                let name = name.trim().to_string();
                if name.is_empty() {
                    return;
                }
                let e = eq.borrow().clone();
                let mut presets = audio::user_presets();
                presets.preset.retain(|p| p.name != name);
                presets.preset.push(audio::UserPreset { name: name.clone(), gains: e.gains, preamp_db: Some(e.preamp_db) });
                match audio::save_user_presets(&presets) {
                    Ok(()) => {
                        eq.borrow_mut().preset = format!("user:{name}");
                        let _ = audio::save(&eq.borrow());
                        window::toast(&format!("Saved preset “{name}”"));
                        p2.popdown();
                        window::rebuild("audio");
                    }
                    Err(e) => window::toast(&format!("Couldn't save preset: {e}")),
                }
            };
            let s1 = save.clone();
            let e1 = entry.clone();
            ok.connect_clicked(move |_| s1(e1.text().to_string()));
            entry.connect_activate(move |e| save(e.text().to_string()));
            pop.popup();
            entry.grab_focus();
        });
    }

    {
        let eq = eq.clone();
        let panel = panel.clone();
        let rh = refresh_headroom.clone();
        enable.connect_active_notify(move |sw| {
            let on = sw.is_active();
            eq.borrow_mut().enabled = on;
            if on {
                panel.remove_css_class("bypassed");
            } else {
                panel.add_css_class("bypassed");
            }
            rh();
            let e = eq.borrow().clone();
            cmd::background(
                move || {
                    if on { audio::install_and_start(&e) } else { audio::save(&e).and_then(|_| audio::stop(&e)) }
                },
                move |r| match r {
                    Ok(()) => window::toast(if on { "Equalizer on" } else { "Equalizer off" }),
                    Err(e) => window::toast(&format!("{e}")),
                },
            );
        });
    }

    let eq_row = widgets::stacked_row("", "", panel.upcast_ref());
    eq_row.add_css_class("bare");
    widgets::keywords("equalizer preamp boost louder bass treble eq band 31 63 125 250 500 1k 2k 4k 8k");
    g.add(&eq_row);

    // Values the graph is actually running, when it is.
    if running {
        let set_all = set_all.clone();
        cmd::background(audio::live_controls, move |live| {
            let Some(live) = live else { return };
            let get = |k: &str| live.iter().find(|(n, _)| n == k).map(|(_, v)| *v);
            if let Some(mult) = get("pre_l:Mult")
                && mult > 0.0
            {
                let db = 20.0 * mult.log10();
                let mut gains = [0.0; 9];
                for (i, g) in gains.iter_mut().enumerate() {
                    *g = get(&format!("eq_l_{i}:Gain")).unwrap_or(0.0);
                }
                // Only show it if it's not bypassed (bypass reads as flat).
                if (mult - 1.0).abs() > 1e-6 || gains.iter().any(|g| *g != 0.0) {
                    set_all(Some((db * 2.0).round() / 2.0), gains);
                }
            }
        });
    }

    // ----- Input -----
    let g = page.group("Input");
    let sources = audio::sources();
    let options: Vec<(String, String)> = sources.iter().map(|s| (s.name.clone(), s.description.clone())).collect();
    let (r, _) = widgets::choice_row("Microphone", "", options, &audio::default_source(), |name| {
        cmd::spawn(&["pactl", "set-default-source", &name]);
    });
    g.add(&r);
    let current_in = audio::default_source();
    if let Some(dev) = sources.iter().find(|s| s.name == current_in) {
        g.add(&prefer_row(&dev.name, "input", &dev.description));
    }
    let src_vol = cmd::output(&["pactl", "get-source-volume", "@DEFAULT_SOURCE@"])
        .and_then(|t| t.split_whitespace().find(|w| w.ends_with('%')).and_then(|w| w.trim_end_matches('%').parse::<f64>().ok()))
        .unwrap_or(100.0);
    let (r, _) = widgets::slider_row("Input level", "", (0.0, 150.0, 1.0), src_vol, 0, "%", |v| {
        cmd::spawn(&["pactl", "set-source-volume", "@DEFAULT_SOURCE@", &format!("{}%", v.round() as i64)]);
    });
    g.add(&r);

    apps_group(page);
    remembered(page, &sinks, &sources);

    // ----- More -----
    let g = page.collapsible("More", false);
    let buttons = widgets::hbox(8);
    let restart = gtk::Button::with_label("Restart audio");
    restart.connect_clicked(|b| {
        b.set_sensitive(false);
        let b = b.clone();
        cmd::run_async(&["systemctl", "--user", "restart", "pipewire", "pipewire-pulse", "wireplumber"], move |r| {
            b.set_sensitive(true);
            match r {
                Ok(_) => window::toast("Audio restarted"),
                Err(e) => window::toast(&format!("{e}")),
            }
            glib::timeout_add_local_once(std::time::Duration::from_millis(1500), || window::rebuild("audio"));
        });
    });
    buttons.append(&restart);
    g.add(&widgets::row(
        "Troubleshooting",
        "Restart PipeWire if sound stops or a device goes missing.",
        Some(buttons.upcast_ref()),
    ));
}

// ----- Apps -----

fn app_row(s: &streams::Stream, outputs: &[(String, String)]) -> gtk::Box {
    let index = s.index;
    let (slider, sl) = widgets::slider(0.0, 150.0, 1.0, s.volume as f64, 0, "%");
    sl.scale.set_hexpand(true);
    slider.set_hexpand(true);
    sl.scale.connect_value_changed(move |sc| streams::set_volume(index, sc.value().round() as u32));
    let r = widgets::hbox(10);
    r.append(&slider);
    if outputs.len() > 1 {
        let dd = widgets::dropdown(outputs, &s.sink);
        dd.set_valign(gtk::Align::Center);
        dd.set_tooltip_text(Some("Plays through"));
        let outputs = outputs.to_vec();
        dd.connect_selected_notify(move |d| {
            let Some((name, _)) = outputs.get(d.selected() as usize).cloned() else { return };
            cmd::background(
                move || streams::move_to(index, &name),
                |r| {
                    if let Err(e) = r {
                        window::toast(&format!("Couldn't move the app: {e}"));
                    }
                },
            );
        });
        r.append(&dd);
    }
    let mute = gtk::ToggleButton::new();
    let icon = |m: bool| if m { "audio-volume-muted-symbolic" } else { "audio-volume-high-symbolic" };
    mute.set_icon_name(icon(s.muted));
    mute.set_active(s.muted);
    mute.set_tooltip_text(Some("Mute"));
    mute.set_valign(gtk::Align::Center);
    mute.connect_toggled(move |b| {
        streams::set_mute(index, b.is_active());
        b.set_icon_name(icon(b.is_active()));
    });
    r.append(&mute);
    widgets::stacked_row(&s.app, &glib::markup_escape_text(&s.media), r.upcast_ref())
}

/// Volume, mute and output for each app playing sound. Rows are rebuilt only
/// when apps start or stop, so a slider being dragged is never replaced.
fn apps_group(page: &Page) {
    let g = page.group("Apps");
    g.note("Apps playing sound right now.");
    widgets::keywords("per app application volume mixer stream mute firefox spotify");
    let list = widgets::vbox(6);
    g.add(&list);
    let shown: Rc<RefCell<Vec<u32>>> = Rc::new(RefCell::new(vec![u32::MAX]));
    let refresh = {
        let (list, shown) = (list.clone(), shown.clone());
        move || {
            let now = streams::list();
            let ids: Vec<u32> = now.iter().map(|s| s.index).collect();
            if *shown.borrow() == ids {
                return;
            }
            *shown.borrow_mut() = ids;
            while let Some(c) = list.first_child() {
                list.remove(&c);
            }
            if now.is_empty() {
                list.append(&widgets::row("Nothing playing", "Apps show up here while they play sound.", None));
                return;
            }
            let mut outputs: Vec<(String, String)> =
                audio::hardware_sinks().into_iter().map(|s| (s.name, s.description)).collect();
            if audio::running() {
                outputs.insert(0, (audio::SINK.to_string(), "Equalizer".to_string()));
            }
            for s in &now {
                list.append(&app_row(s, &outputs));
            }
        }
    };
    refresh();
    let weak = list.downgrade();
    glib::timeout_add_seconds_local(2, move || {
        let Some(l) = weak.upgrade() else { return glib::ControlFlow::Break };
        if l.is_mapped() {
            refresh();
        }
        glib::ControlFlow::Continue
    });
}

// ----- Remembered devices -----

/// "Use whenever it's connected": WirePlumber gives it priority each time it appears.
fn prefer_row(node: &str, kind: &'static str, description: &str) -> gtk::Box {
    let on = devices::load_audio().devices.get(node).is_some_and(|d| d.prefer);
    let (n, d) = (node.to_string(), description.to_string());
    let what = if kind == "input" { "Switch to this microphone" } else { "Switch to this output" };
    let (r, _) = widgets::switch_row(
        "Use whenever it's connected",
        &format!("{what} each time it's plugged in or turned on."),
        on,
        move |on| {
            if let Err(e) = devices::set_prefer(&n, kind, &d, on) {
                window::toast(&format!("Couldn't save that: {e}"));
            }
        },
    );
    widgets::keywords("prefer priority automatic switch connect headset usb bluetooth");
    r
}

fn remembered(page: &Page, sinks: &[audio::Device], sources: &[audio::Device]) {
    let outs = devices::remembered_audio("output", &sinks.iter().map(|d| d.name.clone()).collect::<Vec<_>>());
    let ins = devices::remembered_audio("input", &sources.iter().map(|d| d.name.clone()).collect::<Vec<_>>());
    if outs.is_empty() && ins.is_empty() {
        return;
    }
    let g = page.collapsible("Remembered devices", false);
    g.note("Sound devices with their own settings that aren't connected now. Their settings apply again when they're back.");
    widgets::keywords("remembered disconnected headset speaker microphone forget");
    for (kind, list) in [("output", outs), ("input", ins)] {
        for (node, dev, profile) in list {
            let body = widgets::vbox(8);
            body.append(&prefer_row(&node, kind, &dev.name));
            let forget = gtk::Button::with_label("Forget this device");
            forget.set_halign(gtk::Align::Start);
            let n = node.clone();
            forget.connect_clicked(move |_| {
                let category =
                    if kind == "input" { nyx_devices::Category::AudioInput } else { nyx_devices::Category::AudioOutput };
                match devices::forget(&nyx_devices::DeviceId::new(category, &n)) {
                    Ok(()) => window::toast("Device forgotten"),
                    Err(e) => window::toast(&format!("Couldn't forget all of it: {e}")),
                }
                window::rebuild("audio");
            });
            body.append(&forget);
            let title = if dev.name.is_empty() { node.clone() } else { dev.name.clone() };
            g.add(&widgets::stacked_row(&title, &devices::last_seen(profile.as_ref()), body.upcast_ref()));
        }
    }
}
