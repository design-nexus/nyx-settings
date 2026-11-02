use crate::backend::store;
use crate::cmd;
use crate::sketch::Sketch;
use crate::widgets::{self, Page, hypr_slider, hypr_switch, sketched};

/// A switch row with a drawing of what it does.
fn pictured(key: &'static str, title: &str, desc: &str, s: Sketch) -> gtk::Box {
    let r = hypr_switch(key, title, desc);
    sketched(&r, s);
    r
}

pub fn build(page: &Page) {
    let g = page.plain_group("");
    g.add(&crate::preview::window_style());

    let g = page.group("Spacing");
    g.add(&hypr_slider("general.gaps_in", "Inner gaps", "Space between neighboring windows.", (0.0, 40.0, 1.0), 0, " px", true));
    g.add(&hypr_slider(
        "general.gaps_out",
        "Outer gaps",
        "Space between windows and the screen edge.",
        (0.0, 60.0, 1.0),
        0,
        " px",
        true,
    ));
    widgets::keywords("margin padding");

    let g = page.group("Borders & corners");
    g.add(&hypr_slider(
        "general.border_size",
        "Border width",
        "Outline drawn around every window.",
        (0.0, 10.0, 1.0),
        0,
        " px",
        true,
    ));
    g.add(&hypr_slider("decoration.rounding", "Corner rounding", "Radius of window corners.", (0.0, 24.0, 1.0), 0, " px", true));
    widgets::keywords("radius round corners");
    g.add(&pictured(
        "general.resize_on_border",
        "Resize from borders",
        "Drag a window's border to resize it.",
        Sketch::ResizeOnBorder,
    ));

    let g = page.collapsible("Transparency", false);
    g.add(&hypr_slider("decoration.active_opacity", "Focused window opacity", "", (0.5, 1.0, 0.01), 2, "", false));
    g.add(&hypr_slider("decoration.inactive_opacity", "Unfocused window opacity", "", (0.5, 1.0, 0.01), 2, "", false));
    g.add(&pictured(
        "decoration.dim_inactive",
        "Dim unfocused windows",
        "Windows you're not using are darkened, so the one you are stands out.",
        Sketch::Dim,
    ));
    g.add(&hypr_slider("decoration.dim_strength", "Dim amount", "", (0.0, 0.8, 0.01), 2, "", false));

    let g = page.collapsible("Blur", false);
    g.note("Blur shows behind transparent windows, the bar and menus. It costs some GPU time.");
    g.add(&pictured(
        "decoration.blur.enabled",
        "Blur",
        "What's behind a transparent window looks frosted, so its text stays readable.",
        Sketch::Blur,
    ));
    g.add(&hypr_slider("decoration.blur.size", "Blur size", "", (1.0, 20.0, 1.0), 0, "", true));
    g.add(&hypr_slider("decoration.blur.passes", "Blur passes", "More passes look smoother.", (1.0, 6.0, 1.0), 0, "", true));
    g.add(&hypr_slider(
        "decoration.blur.vibrancy",
        "Vibrancy",
        "Color saturation of the blurred background.",
        (0.0, 1.0, 0.01),
        2,
        "",
        false,
    ));

    let g = page.collapsible("Shadows", false);
    g.add(&pictured(
        "decoration.shadow.enabled",
        "Window shadows",
        "A soft shadow lifts each window off the wallpaper.",
        Sketch::Shadow,
    ));
    g.add(&hypr_slider("decoration.shadow.range", "Shadow size", "", (0.0, 50.0, 1.0), 0, " px", true));
    g.add(&hypr_slider(
        "decoration.shadow.render_power",
        "Shadow falloff",
        "Higher is a sharper edge.",
        (1.0, 4.0, 1.0),
        0,
        "",
        true,
    ));

    let g = page.collapsible("Animations", false);
    g.add(&pictured(
        "animations.enabled",
        "Animations",
        "Windows and workspaces slide and fade as they open, move and switch.",
        Sketch::Animations,
    ));
    g.add(&hypr_switch("misc.animate_manual_resizes", "Animate resizing", "Smooth out keyboard and mouse resizes."));

    let g = page.collapsible("Cursor", false);
    let size = store::read(|s| s.env.get("XCURSOR_SIZE").cloned())
        .or_else(|| std::env::var("XCURSOR_SIZE").ok())
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(24.0);
    let (r, _) = widgets::slider_row("Cursor size", "", (16.0, 64.0, 4.0), size, 0, " px", |v| {
        let n = (v.round() as i64).to_string();
        store::set_env("XCURSOR_SIZE", Some(n.clone()));
        store::set_env("HYPRCURSOR_SIZE", Some(n.clone()));
        let theme = std::env::var("XCURSOR_THEME").unwrap_or_else(|_| "Adwaita".into());
        cmd::spawn(&["hyprctl", "setcursor", &theme, &n]);
    });
    g.add(&r);
    g.add(&hypr_slider(
        "cursor.inactive_timeout",
        "Hide cursor after",
        "Seconds without movement. 0 never hides it.",
        (0.0, 30.0, 1.0),
        0,
        " s",
        false,
    ));
    g.add(&hypr_switch("cursor.hide_on_key_press", "Hide cursor while typing", ""));
}
