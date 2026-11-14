use crate::backend::audio::{self, Eq};
use crate::backend::{devices, store, streams};
use crate::sketch;
use crate::widgets::{self, Page};
use crate::{cmd, window};
use gtk::glib;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

fn freq_label(f: u32) -> String {
    if f >= 1000 { format!("{}k", f / 1000) } else { f.to_string() }
}

fn fmt_db(v: f64) -> String {
    if v.abs() < 0.05 { "0 dB".into() } else { format!("{v:+.1} dB").replace(".0 dB", " dB") }
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

fn headroom_text(eq: &Eq) -> String {
    let peak = audio::peak_boost(eq);
    let limiter = audio::limiter_available();
    let text = if !eq.enabled {
        "Bypassed: sound passes through unchanged.".to_string()
    } else if peak <= 0.0 {
        "No boost, so plenty of headroom.".to_string()
    } else if limiter {
        format!("Up to {} louder. The limiter keeps loud peaks from distorting.", fmt_db(peak))
    } else if peak > 6.0 {
        format!("Up to {} louder. Without a limiter, loud music may distort.", fmt_db(peak))
    } else {
        format!("Up to {} louder.", fmt_db(peak))
    };
    if limiter {
        text
    } else {
        format!("{text} Install lsp-plugins-lv2 for a limiter, which allows up to +24 dB of preamp without distortion.")
    }
}

/// What kind of device an output or input is, from its PipeWire name: an icon, and
/// how it's connected.
fn kind(d: &audio::Device) -> (&'static str, &'static str) {
    let (n, desc) = (d.name.to_lowercase(), d.description.to_lowercase());
    if n.starts_with("bluez") {
        ("audio-headphones-symbolic", "Bluetooth")
    } else if n.contains("hdmi") || desc.contains("hdmi") || n.contains("displayport") {
        ("video-display-symbolic", "HDMI or DisplayPort")
    } else if n.contains(".usb-") || n.contains("usb") {
        ("audio-speakers-symbolic", "USB audio")
    } else if n.contains(".pci-") || n.contains("platform-") {
        ("audio-card-symbolic", "Built in")
    } else {
        ("audio-speakers-symbolic", "")
    }
}

// ---------- Chips that wrap onto more lines ----------

mod wrap_imp {
    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    const GAP: i32 = 6;

    #[derive(Default)]
    pub struct Wrap;

    #[glib::object_subclass]
    impl ObjectSubclass for Wrap {
        const NAME: &'static str = "NyxSettingsWrapLayout";
        type Type = super::WrapLayout;
        type ParentType = gtk::LayoutManager;
    }

    impl ObjectImpl for Wrap {}

    fn children(w: &gtk::Widget) -> Vec<gtk::Widget> {
        std::iter::successors(w.first_child(), |c| c.next_sibling()).filter(|c| c.should_layout()).collect()
    }

    /// Each child's place, filling lines `width` wide: (child, x, y, w, h); and the height.
    fn lines(w: &gtk::Widget, width: i32) -> (Vec<(gtk::Widget, i32, i32, i32, i32)>, i32) {
        let (mut x, mut y, mut line_h) = (0, 0, 0);
        let mut out = Vec::new();
        for c in children(w) {
            let (min_w, nat_w, _, _) = c.measure(gtk::Orientation::Horizontal, -1);
            let mut cw = nat_w.min(width.max(min_w));
            if x > 0 && x + cw > width {
                if x + min_w <= width {
                    // It can narrow (wrapping text): it takes the rest of the line.
                    cw = width - x;
                } else {
                    x = 0;
                    y += line_h + GAP;
                    line_h = 0;
                    cw = nat_w.min(width.max(min_w));
                }
            }
            let (_, ch, _, _) = c.measure(gtk::Orientation::Vertical, cw);
            out.push((c, x, y, cw, ch));
            line_h = line_h.max(ch);
            x += cw + GAP;
        }
        (out, y + line_h)
    }

    impl LayoutManagerImpl for Wrap {
        fn request_mode(&self, _widget: &gtk::Widget) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }

        fn measure(&self, widget: &gtk::Widget, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            if orientation == gtk::Orientation::Horizontal {
                let sizes: Vec<(i32, i32)> = children(widget)
                    .iter()
                    .map(|c| {
                        let (a, b, _, _) = c.measure(gtk::Orientation::Horizontal, -1);
                        (a, b)
                    })
                    .collect();
                let min = sizes.iter().map(|s| s.0).max().unwrap_or(0);
                let nat = sizes.iter().map(|s| s.1).sum::<i32>() + GAP * (sizes.len() as i32 - 1).max(0);
                return (min, nat, -1, -1);
            }
            let width = if for_size < 0 { i32::MAX / 4 } else { for_size };
            let (_, h) = lines(widget, width);
            (h, h, -1, -1)
        }

        fn allocate(&self, widget: &gtk::Widget, width: i32, _height: i32, _baseline: i32) {
            for (c, x, y, w, h) in lines(widget, width).0 {
                c.size_allocate(&gtk::Allocation::new(x, y, w, h), -1);
            }
        }
    }
}

glib::wrapper! {
    /// Lays its children out in a line, wrapping onto the next when they don't fit.
    pub struct WrapLayout(ObjectSubclass<wrap_imp::Wrap>) @extends gtk::LayoutManager;
}

/// A box whose children wrap onto more lines, each as wide as it needs.
pub(super) fn wrap_box() -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    b.set_layout_manager(Some(glib::Object::new::<WrapLayout>()));
    b
}

// ---------- The equalizer, drawn ----------

/// The curve through the nine bands, with a handle on each to drag.
struct Curve {
    area: gtk::DrawingArea,
    gains: Rc<RefCell<[f64; 9]>>,
    on: Rc<Cell<bool>>,
}

impl Curve {
    fn set(&self, gains: [f64; 9]) {
        *self.gains.borrow_mut() = gains;
        self.area.queue_draw();
    }
}

const GRID: [f64; 5] = [12.0, 6.0, 0.0, -6.0, -12.0];
/// Room at the left for the dB labels and at the foot for the frequencies.
const LEFT: f64 = 38.0;
const FOOT: f64 = 28.0;
const TOP: f64 = 16.0;

fn band_x(i: usize, w: f64) -> f64 {
    let (x0, x1) = (LEFT + 16.0, w - 18.0);
    x0 + (x1 - x0) * i as f64 / 8.0
}

fn gain_y(g: f64, h: f64) -> f64 {
    let (y0, y1) = (TOP, h - FOOT);
    y0 + (y1 - y0) * (12.0 - g.clamp(-12.0, 12.0)) / 24.0
}

fn y_gain(y: f64, h: f64) -> f64 {
    let (y0, y1) = (TOP, h - FOOT);
    let g = 12.0 - (y - y0) / (y1 - y0).max(1.0) * 24.0;
    ((g * 2.0).round() / 2.0).clamp(audio::GAIN_RANGE.0, audio::GAIN_RANGE.1)
}

#[allow(clippy::too_many_arguments)]
fn draw_curve(cr: &gtk::cairo::Context, p: &sketch::Pal, accent: sketch::Rgb, w: f64, h: f64, gains: &[f64; 9], on: bool, focus: Option<usize>) {
    let colour = if on { accent } else { p.dim };
    // The grid, with dB at the left.
    cr.select_font_face("sans-serif", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Normal);
    cr.set_font_size(11.0);
    for g in GRID {
        let y = gain_y(g, h).round() + 0.5;
        sketch::set(cr, p.dim, if g == 0.0 { 0.35 } else { 0.16 });
        cr.set_line_width(1.0);
        if g == 0.0 {
            cr.set_dash(&[4.0, 4.0], 0.0);
        }
        cr.move_to(LEFT, y);
        cr.line_to(w - 4.0, y);
        let _ = cr.stroke();
        cr.set_dash(&[], 0.0);
        let label = if g > 0.0 { format!("+{g}") } else if g < 0.0 { format!("−{}", -g) } else { "0".into() };
        sketch::set(cr, p.dim, 0.9);
        if let Ok(ext) = cr.text_extents(&label) {
            cr.move_to(LEFT - 8.0 - ext.x_advance(), y + 4.0);
            let _ = cr.show_text(&label);
        }
    }
    // The bands' frequencies along the foot.
    for (i, f) in audio::FREQUENCIES.iter().enumerate() {
        let label = freq_label(*f);
        let x = band_x(i, w);
        sketch::set(cr, p.dim, if focus == Some(i) { 1.0 } else { 0.8 });
        if let Ok(ext) = cr.text_extents(&label) {
            cr.move_to(x - ext.x_advance() / 2.0, h - 8.0);
            let _ = cr.show_text(&label);
        }
    }
    // The curve: smooth through every band, filled down to the foot.
    let pts: Vec<(f64, f64)> = gains.iter().enumerate().map(|(i, g)| (band_x(i, w), gain_y(*g, h))).collect();
    let path = |cr: &gtk::cairo::Context| {
        cr.move_to(pts[0].0, pts[0].1);
        for k in 1..pts.len() {
            let ((x0, y0), (x1, y1)) = (pts[k - 1], pts[k]);
            let cx = (x0 + x1) / 2.0;
            cr.curve_to(cx, y0, cx, y1, x1, y1);
        }
    };
    path(cr);
    let bottom = h - FOOT;
    cr.line_to(pts[8].0, bottom);
    cr.line_to(pts[0].0, bottom);
    cr.close_path();
    let g = gtk::cairo::LinearGradient::new(0.0, TOP, 0.0, bottom);
    g.add_color_stop_rgba(0.0, colour.0, colour.1, colour.2, if on { 0.28 } else { 0.12 });
    g.add_color_stop_rgba(1.0, colour.0, colour.1, colour.2, 0.04);
    let _ = cr.set_source(&g);
    let _ = cr.fill();
    path(cr);
    sketch::set(cr, colour, 1.0);
    cr.set_line_width(2.5);
    cr.set_line_join(gtk::cairo::LineJoin::Round);
    let _ = cr.stroke();
    for (i, (x, y)) in pts.iter().enumerate() {
        let r = if focus == Some(i) { 7.5 } else { 6.0 };
        cr.arc(*x, *y, r, 0.0, std::f64::consts::TAU);
        sketch::set(cr, sketch::mix(p.text, (1.0, 1.0, 1.0), 0.6), 1.0);
        let _ = cr.fill_preserve();
        sketch::set(cr, colour, 1.0);
        cr.set_line_width(2.5);
        let _ = cr.stroke();
    }
    // A band being changed shows its value.
    if let Some(i) = focus {
        let (x, y) = pts[i];
        let text = fmt_db(gains[i]);
        cr.select_font_face("sans-serif", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Bold);
        cr.set_font_size(11.0);
        if let Ok(ext) = cr.text_extents(&text) {
            let (bw, bh) = (ext.x_advance() + 14.0, 20.0);
            let bx = (x - bw / 2.0).min(w - bw - 2.0).max(LEFT);
            let by = if y - 34.0 > 2.0 { y - 34.0 } else { y + 14.0 };
            sketch::rounded(cr, bx, by, bw, bh, 6.0);
            sketch::set(cr, p.bg, 0.92);
            let _ = cr.fill();
            sketch::set(cr, p.text, 1.0);
            cr.move_to(bx + 7.0, by + 14.0);
            let _ = cr.show_text(&text);
        }
    }
}

/// The curve, and what to call when a band is changed on it (band, new gain).
fn curve(gains: [f64; 9], on: bool, on_change: impl Fn(usize, f64) + 'static) -> Curve {
    let gains = Rc::new(RefCell::new(gains));
    let on = Rc::new(Cell::new(on));
    let focus: Rc<Cell<Option<usize>>> = Rc::default();
    let area = {
        let (g, o, f) = (gains.clone(), on.clone(), focus.clone());
        widgets::drawing(200, move |cr, p, accent, w, h| draw_curve(cr, p, accent, w, h, &g.borrow(), o.get(), f.get()))
    };
    area.add_css_class("snd-curve");
    area.set_vexpand(true);
    area.set_focusable(true);
    area.set_cursor_from_name(Some("ns-resize"));
    area.update_property(&[gtk::accessible::Property::Label("Equalizer bands")]);
    let on_change = Rc::new(on_change);

    let nearest = |x: f64, w: f64| (0..9).min_by(|a, b| (band_x(*a, w) - x).abs().total_cmp(&(band_x(*b, w) - x).abs())).unwrap_or(0);
    let drag = gtk::GestureDrag::new();
    {
        let (a, f, g, cb) = (area.clone(), focus.clone(), gains.clone(), on_change.clone());
        drag.connect_drag_begin(move |_, x, y| {
            let (w, h) = (a.width() as f64, a.height() as f64);
            let i = nearest(x, w);
            f.set(Some(i));
            a.grab_focus();
            // A press on the band's line moves its handle there.
            if (gain_y(g.borrow()[i], h) - y).abs() > 10.0 {
                let v = y_gain(y, h);
                g.borrow_mut()[i] = v;
                cb(i, v);
            }
            a.queue_draw();
        });
    }
    {
        let (a, f, g, cb) = (area.clone(), focus.clone(), gains.clone(), on_change.clone());
        drag.connect_drag_update(move |d, _, dy| {
            let (Some(i), Some((_, sy))) = (f.get(), d.start_point()) else { return };
            let v = y_gain(sy + dy, a.height() as f64);
            if (g.borrow()[i] - v).abs() > 0.01 {
                g.borrow_mut()[i] = v;
                cb(i, v);
                a.queue_draw();
            }
        });
    }
    {
        let (a, f) = (area.clone(), focus.clone());
        drag.connect_drag_end(move |_, _, _| {
            if !a.has_focus() {
                f.set(None);
            }
            a.queue_draw();
        });
    }
    area.add_controller(drag);

    // Left and Right pick a band, Up and Down change it by half a decibel.
    let keys = gtk::EventControllerKey::new();
    {
        let (a, f, g, cb) = (area.clone(), focus.clone(), gains.clone(), on_change.clone());
        keys.connect_key_pressed(move |_, key, _, _| {
            use gtk::gdk::Key;
            let i = f.get().unwrap_or(0);
            match key {
                Key::Left => f.set(Some(i.saturating_sub(1))),
                Key::Right => f.set(Some((i + 1).min(8))),
                Key::Up | Key::Down => {
                    let step = if key == Key::Up { 0.5 } else { -0.5 };
                    let v = (g.borrow()[i] + step).clamp(audio::GAIN_RANGE.0, audio::GAIN_RANGE.1);
                    g.borrow_mut()[i] = v;
                    f.set(Some(i));
                    cb(i, v);
                }
                _ => return glib::Propagation::Proceed,
            }
            a.queue_draw();
            glib::Propagation::Stop
        });
    }
    area.add_controller(keys);
    let focus_ctl = gtk::EventControllerFocus::new();
    {
        let (a, f) = (area.clone(), focus.clone());
        focus_ctl.connect_leave(move |_| {
            f.set(None);
            a.queue_draw();
        });
    }
    area.add_controller(focus_ctl);
    Curve { area, gains, on }
}

// ---------- The page ----------

pub fn build(page: &Page) {
    let eq = Rc::new(RefCell::new(audio::load()));
    let running = audio::running();
    let default_sink = audio::default_sink();
    let routed = default_sink == audio::SINK;
    let sinks = audio::hardware_sinks();
    let current_out = if routed { eq.borrow().target.clone() } else { default_sink.clone() };
    let out_name = sinks.iter().find(|s| s.name == current_out).map(|s| s.description.clone()).unwrap_or_default();

    let pv = page.split();
    equalizer(&pv, &eq, running, routed, &out_name);

    // ----- Plays through -----
    let g = page.group("Plays through");
    widgets::keywords("speakers headphones hdmi output device");
    let mut first: Option<gtk::CheckButton> = None;
    for dev in &sinks {
        let radio = gtk::CheckButton::new();
        if let Some(f) = &first {
            radio.set_group(Some(f));
        } else {
            first = Some(radio.clone());
        }
        radio.set_active(dev.name == current_out);
        radio.set_valign(gtk::Align::Center);
        let (icon, how) = kind(dev);
        let r = widgets::row(&dev.description, how, None);
        r.prepend(&widgets::tile(icon));
        r.prepend(&radio);
        r.add_css_class("with-tile");
        r.add_css_class("snd-device");
        widgets::keywords("output device speakers headphones");
        // The whole row picks it.
        let click = gtk::GestureClick::new();
        let rb = radio.clone();
        click.connect_released(move |_, _, _, _| rb.set_active(true));
        r.add_controller(click);
        let (eq2, name) = (eq.clone(), dev.name.clone());
        radio.connect_toggled(move |b| {
            if !b.is_active() {
                return;
            }
            let using_eq = audio::running() && audio::default_sink() == audio::SINK;
            if using_eq {
                let mut e = eq2.borrow().clone();
                e.target = name.clone();
                *eq2.borrow_mut() = e.clone();
                cmd::background(
                    move || audio::install_and_start(&e),
                    |r| {
                        if let Err(e) = r {
                            window::toast(&format!("Couldn't switch output: {e}"));
                        }
                        window::rebuild("audio");
                    },
                );
            } else {
                cmd::spawn(&["pactl", "set-default-sink", &name]);
                glib::timeout_add_local_once(std::time::Duration::from_millis(500), || window::rebuild("audio"));
            }
        });
        g.add(&r);
    }
    if sinks.is_empty() {
        g.add(&widgets::row("No outputs", "PipeWire doesn't list any speakers or headphones.", None));
    }
    if let Some(dev) = sinks.iter().find(|s| s.name == current_out) {
        g.add(&prefer_row(&dev.name, "output", &dev.description));
    }

    // ----- Volume -----
    let g = page.group("Volume");
    let max = store::read(|s| s.max_volume).unwrap_or(100);
    let vol_sink = audio::volume_sink();
    let vol = audio::volume_percent(&vol_sink).unwrap_or(50) as f64;
    let (slider_box, vol_slider) = widgets::slider(0.0, max as f64, 1.0, vol.min(max as f64), 0, "%");
    vol_slider.scale.set_hexpand(true);
    slider_box.set_hexpand(true);
    vol_slider.scale.connect_value_changed(|v| {
        let sink = audio::volume_sink();
        cmd::spawn(&["pactl", "set-sink-volume", &sink, &format!("{}%", v.value().round() as i64)]);
    });
    let mute = gtk::ToggleButton::new();
    mute.add_css_class("flat");
    mute.set_icon_name(if audio::muted(&vol_sink) { "audio-volume-muted-symbolic" } else { "audio-volume-high-symbolic" });
    mute.set_active(audio::muted(&vol_sink));
    mute.set_tooltip_text(Some("Mute"));
    mute.connect_toggled(|b| {
        let sink = audio::volume_sink();
        cmd::spawn(&["pactl", "set-sink-mute", &sink, if b.is_active() { "1" } else { "0" }]);
        b.set_icon_name(if b.is_active() { "audio-volume-muted-symbolic" } else { "audio-volume-high-symbolic" });
    });
    mute.set_valign(gtk::Align::Center);
    // A row of its own: the mute button, then the slider across the card.
    let vol_row = widgets::row("Volume", "", Some(slider_box.upcast_ref()));
    vol_row.add_css_class("snd-volume");
    if let Some(text) = vol_row.first_child() {
        text.set_visible(false);
        text.set_hexpand(false);
    }
    vol_row.prepend(&mute);
    widgets::keywords("volume loudness sound level mute");
    g.add(&vol_row);

    let scale = vol_slider.scale.clone();
    let mut max_opts: Vec<(String, String)> = (100..=150).step_by(10).map(|v| (v.to_string(), format!("{v}%"))).collect();
    if !max_opts.iter().any(|(id, _)| *id == max.to_string()) {
        max_opts.push((max.to_string(), format!("{max}%")));
        max_opts.sort_by_key(|(id, _)| id.parse::<u32>().unwrap_or(0));
    }
    let (r, _) = widgets::choice_row("Maximum volume", "Above 100% can distort.", max_opts, &max.to_string(), move |v| {
        let m: u32 = v.parse().unwrap_or(100);
        scale.set_range(0.0, m as f64);
        store::update(true, |s| s.max_volume = if m > 100 { Some(m) } else { None });
    });
    widgets::keywords("loud louder boost over amplify 150 past 100 volume keys");
    g.add(&r);

    // ----- Microphone -----
    let g = page.group("Microphone");
    let sources = audio::sources();
    let current_in = audio::default_source();
    let options: Vec<(String, String)> = sources.iter().map(|s| (s.name.clone(), s.description.clone())).collect();
    let (r, _) = widgets::choice_row("Microphone", "", options, &current_in, |name| {
        cmd::spawn(&["pactl", "set-default-source", &name]);
    });
    r.prepend(&widgets::tile("audio-input-microphone-symbolic"));
    r.add_css_class("with-tile");
    widgets::keywords("input microphone mic headset");
    g.add(&r);
    if let Some(dev) = sources.iter().find(|s| s.name == current_in) {
        g.add(&prefer_row(&dev.name, "input", &dev.description));
    }
    let src_vol = cmd::output(&["pactl", "get-source-volume", "@DEFAULT_SOURCE@"])
        .and_then(|t| t.split_whitespace().find(|w| w.ends_with('%')).and_then(|w| w.trim_end_matches('%').parse::<f64>().ok()))
        .unwrap_or(100.0);
    let (r, input) = widgets::slider_row("Input level", "", (0.0, 150.0, 1.0), src_vol, 0, "%", |v| {
        cmd::spawn(&["pactl", "set-source-volume", "@DEFAULT_SOURCE@", &format!("{}%", v.round() as i64)]);
    });
    input.scale.set_width_request(130);
    widgets::keywords("microphone volume gain quiet loud");
    g.add(&r);

    // ----- More, one level down -----
    let g = page.group("");
    let playing: Vec<String> = {
        let mut names: Vec<String> = Vec::new();
        for s in streams::list() {
            if !names.contains(&s.app) {
                names.push(s.app);
            }
        }
        names
    };
    let value = match playing.len() {
        0 => "None".to_string(),
        1 | 2 => playing.join(", "),
        n => format!("{}, {} and {} more", playing[0], playing[1], n - 2),
    };
    let (r, _) = widgets::nav_row(Some("view-app-grid-symbolic"), "Apps playing sound", "", &value, || {
        widgets::subpage("Apps playing sound", 620, apps_group);
    });
    widgets::keywords("per app application volume mixer stream mute firefox spotify");
    g.add(&r);
    let (outs, ins) = remembered_lists(&sinks, &sources);
    if !outs.is_empty() || !ins.is_empty() {
        let n = outs.len() + ins.len();
        let (sinks, sources) = (sinks.clone(), sources.clone());
        let (r, _) = widgets::nav_row(Some("document-open-recent-symbolic"), "Remembered devices", "", &n.to_string(), move || {
            let (sinks, sources) = (sinks.clone(), sources.clone());
            widgets::subpage("Remembered devices", 560, move |p| remembered(p, &sinks, &sources));
        });
        widgets::keywords("remembered disconnected headset speaker microphone forget");
        g.add(&r);
    }
    let (r, _) = widgets::nav_row(Some("applications-engineering-symbolic"), "Troubleshooting", "", "", || {
        widgets::subpage("Troubleshooting", 520, troubleshooting);
    });
    widgets::keywords("restart pipewire wireplumber audio sound stops missing device fix");
    g.add(&r);
}

/// The preview: the equalizer as a curve, the preamp beside it, presets under it.
fn equalizer(pv: &widgets::Preview, eq: &Rc<RefCell<Eq>>, running: bool, routed: bool, out_name: &str) {
    let enable = gtk::Switch::new();
    enable.set_active(eq.borrow().enabled && running);
    enable.set_tooltip_text(Some("Use the equalizer"));
    let builtin = audio::builtin_presets();
    let user = audio::user_presets();
    let mut preset_opts: Vec<(String, String)> = builtin.iter().map(|(id, n, _)| (id.to_string(), n.to_string())).collect();
    for p in &user.preset {
        preset_opts.push((format!("user:{}", p.name), p.name.clone()));
    }
    let line = if out_name.is_empty() {
        "Every app passes through it on its way to the speakers.".to_string()
    } else {
        format!("Every app passes through it on its way to {out_name}.")
    };
    pv.head("Equalizer", &line, &[enable.upcast_ref()]);
    widgets::keywords("equalizer preamp boost louder bass treble eq band 31 63 125 250 500 1k 2k 4k 8k");

    // Why it isn't shaping the sound, when it isn't.
    if eq.borrow().enabled && !running {
        let b = widgets::banner("<b>The equalizer isn't running.</b> Turn it on to use the preamp and the bands.", false);
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
        pv.add(&b);
    } else if running && !routed {
        let b = widgets::banner("<b>Sound isn't going through the equalizer.</b> Another output was chosen as the default.", false);
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
        pv.add(&b);
    }

    let applier = Rc::new(Applier {
        eq: eq.clone(),
        live: Cell::new(None),
        persist: Cell::new(None),
        busy: Cell::new(false),
        again: Cell::new(false),
    });
    // Set while the page itself moves the controls, so they don't count as changes.
    let guard = Rc::new(Cell::new(false));
    let chips: Rc<RefCell<Vec<(String, gtk::ToggleButton)>>> = Rc::default();
    let mark_chip = {
        let chips = chips.clone();
        move |id: &str| {
            for (cid, b) in chips.borrow().iter() {
                b.set_active(cid == id);
            }
        }
    };

    // The card: the curve, and the preamp at its right.
    let card = pv.card();
    card.add_css_class("snd-eq");
    card.set_vexpand(true);
    if !eq.borrow().enabled {
        card.add_css_class("snd-bypassed");
    }
    let row = widgets::hbox(10);
    row.set_vexpand(true);
    card.append(&row);
    let foot_text: Rc<RefCell<Option<gtk::Label>>> = Rc::default();
    let refresh_headroom = {
        let (eq, foot_text) = (eq.clone(), foot_text.clone());
        move || {
            if let Some(l) = foot_text.borrow().as_ref() {
                l.set_text(&headroom_text(&eq.borrow()));
            }
        }
    };
    let curve = {
        let (gains, on) = (eq.borrow().gains, eq.borrow().enabled);
        let (eq, applier, guard, rh, mark) = (eq.clone(), applier.clone(), guard.clone(), refresh_headroom.clone(), mark_chip.clone());
        curve(gains, on, move |i, v| {
            {
                let mut e = eq.borrow_mut();
                e.gains[i] = v;
                e.preset = "custom".into();
            }
            guard.set(true);
            mark("custom");
            guard.set(false);
            rh();
            applier.changed();
        })
    };
    curve.area.set_hexpand(true);
    row.append(&curve.area);
    let sep = gtk::Separator::new(gtk::Orientation::Vertical);
    sep.add_css_class("snd-sep");
    row.append(&sep);
    let pre_col = widgets::vbox(6);
    pre_col.add_css_class("snd-preamp");
    pre_col.append(&widgets::label("Preamp", "snd-preamp-label"));
    let pre = gtk::Scale::with_range(gtk::Orientation::Vertical, audio::PREAMP_RANGE.0, audio::preamp_max(), 0.5);
    pre.set_inverted(true);
    pre.set_draw_value(false);
    pre.set_increments(0.5, 3.0);
    pre.set_value(eq.borrow().preamp_db);
    pre.set_vexpand(true);
    pre.set_halign(gtk::Align::Center);
    pre.set_tooltip_text(Some("Makes everything louder before it reaches the speakers, for when even full volume is too quiet."));
    pre.update_property(&[gtk::accessible::Property::Label("Preamp")]);
    pre_col.append(&pre);
    let pre_value = widgets::label(&fmt_db(eq.borrow().preamp_db), "snd-preamp-value");
    pre_col.append(&pre_value);
    row.append(&pre_col);
    {
        let (eq, applier, guard, rh, label) = (eq.clone(), applier.clone(), guard.clone(), refresh_headroom.clone(), pre_value.clone());
        pre.connect_value_changed(move |s| {
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
    let curve = Rc::new(curve);
    let set_all = {
        let (guard, pre, curve) = (guard.clone(), pre.clone(), curve.clone());
        move |preamp: Option<f64>, gains: [f64; 9]| {
            guard.set(true);
            if let Some(p) = preamp {
                pre.set_value(p);
            }
            curve.set(gains);
            guard.set(false);
        }
    };

    // Picking a preset's chip.
    let apply_preset = {
        let (eq, applier, set_all, rh, guard, mark) =
            (eq.clone(), applier.clone(), set_all.clone(), refresh_headroom.clone(), guard.clone(), mark_chip.clone());
        Rc::new(move |id: &str| {
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
                e.preset = id.to_string();
            }
            set_all(preamp, gains);
            guard.set(true);
            mark(id);
            guard.set(false);
            rh();
            applier.changed();
        })
    };

    // The presets as chips, then Save and Reset.
    let chip_row = wrap_box();
    chip_row.set_hexpand(true);
    chip_row.add_css_class("snd-chips");
    for (id, name) in preset_opts.iter() {
        let b = gtk::ToggleButton::with_label(name);
        b.add_css_class("snd-chip");
        super::displays::tint(&b);
        b.set_active(*id == eq.borrow().preset);
        let (apply, guard, id2, mark) = (apply_preset.clone(), guard.clone(), id.clone(), mark_chip.clone());
        b.connect_clicked(move |b| {
            if guard.get() {
                return;
            }
            if b.is_active() {
                apply(&id2);
            } else {
                // A chip can't be un-picked by clicking it again; it stays the one in use.
                guard.set(true);
                mark(&id2);
                guard.set(false);
            }
        });
        chip_row.append(&b);
        chips.borrow_mut().push((id.clone(), b));
    }
    let save_btn = gtk::Button::new();
    let inner = widgets::hbox(6);
    inner.append(&gtk::Image::from_icon_name("list-add-symbolic"));
    inner.append(&gtk::Label::new(Some("Save as a preset…")));
    save_btn.set_child(Some(&inner));
    save_btn.add_css_class("snd-chip");
    chip_row.append(&save_btn);
    let bottom = widgets::hbox(8);
    bottom.append(&chip_row);
    let reset = gtk::Button::new();
    let inner = widgets::hbox(6);
    inner.append(&gtk::Image::from_icon_name("edit-undo-symbolic"));
    inner.append(&gtk::Label::new(Some("Reset")));
    reset.set_child(Some(&inner));
    reset.add_css_class("flat");
    reset.set_valign(gtk::Align::End);
    reset.set_tooltip_text(Some("All bands to 0 dB, preamp to +6 dB"));
    bottom.append(&reset);
    pv.add(&bottom);
    {
        let (eq, applier, set_all, rh, guard, mark) =
            (eq.clone(), applier.clone(), set_all.clone(), refresh_headroom.clone(), guard.clone(), mark_chip.clone());
        reset.connect_clicked(move |_| {
            {
                let mut e = eq.borrow_mut();
                e.gains = [0.0; 9];
                e.preamp_db = Eq::default().preamp_db;
                e.preset = "flat".into();
            }
            set_all(Some(Eq::default().preamp_db), [0.0; 9]);
            guard.set(true);
            mark("flat");
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
        let (eq, card, rh, curve) = (eq.clone(), card.clone(), refresh_headroom.clone(), curve.clone());
        enable.connect_active_notify(move |sw| {
            let on = sw.is_active();
            eq.borrow_mut().enabled = on;
            if on {
                card.remove_css_class("snd-bypassed");
            } else {
                card.add_css_class("snd-bypassed");
            }
            curve.on.set(on);
            curve.area.queue_draw();
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

    let foot = pv.foot("emblem-ok-symbolic", &headroom_text(&eq.borrow()));
    *foot_text.borrow_mut() = Some(foot);
}

fn troubleshooting(page: &Page) {
    let g = page.group("If sound stops");
    let restart = gtk::Button::with_label("Restart sound");
    restart.connect_clicked(|b| {
        b.set_sensitive(false);
        let b = b.clone();
        cmd::run_async(&["systemctl", "--user", "restart", "pipewire", "pipewire-pulse", "wireplumber"], move |r| {
            b.set_sensitive(true);
            match r {
                Ok(_) => window::toast("Sound restarted"),
                Err(e) => window::toast(&format!("{e}")),
            }
            glib::timeout_add_local_once(std::time::Duration::from_millis(1500), || window::rebuild("audio"));
        });
    });
    let r = widgets::icon_row(
        "view-refresh-symbolic",
        "Restart sound",
        "Restarts PipeWire, for when sound stops or a device goes missing.",
        Some(restart.upcast_ref()),
    );
    widgets::keywords("restart pipewire wireplumber audio troubleshooting");
    g.add(&r);
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
    let g = page.group("Playing now");
    g.note("Each app's own volume, and where it plays.");
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

type Remembered = Vec<(String, devices::AudioDevice, Option<nyx_devices::Profile>)>;

fn remembered_lists(sinks: &[audio::Device], sources: &[audio::Device]) -> (Remembered, Remembered) {
    (
        devices::remembered_audio("output", &sinks.iter().map(|d| d.name.clone()).collect::<Vec<_>>()),
        devices::remembered_audio("input", &sources.iter().map(|d| d.name.clone()).collect::<Vec<_>>()),
    )
}

fn remembered(page: &Page, sinks: &[audio::Device], sources: &[audio::Device]) {
    let (outs, ins) = remembered_lists(sinks, sources);
    let g = page.group("Not connected");
    g.note("Sound devices with their own settings that aren't connected now. Their settings apply again when they're back.");
    widgets::keywords("remembered disconnected headset speaker microphone forget");
    if outs.is_empty() && ins.is_empty() {
        g.add(&widgets::row("None", "Every sound device you've set up is connected.", None));
    }
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
