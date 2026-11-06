use crate::preview;
use crate::sketch::Sketch;
use crate::widgets::{Page, hypr_slider, hypr_switch, hypr_visual_choice, sketched};

fn cards(items: &[(&str, &str, Sketch)]) -> Vec<(String, String, Sketch)> {
    items.iter().map(|(id, name, s)| (id.to_string(), name.to_string(), *s)).collect()
}

/// A switch row with a drawing of what it does.
fn pictured(key: &'static str, title: &str, desc: &str, s: Sketch) -> gtk::Box {
    let r = hypr_switch(key, title, desc);
    sketched(&r, s);
    r
}

pub fn build(page: &Page) {
    let g = page.group("Tiling");
    g.add(&preview::tiling());
    g.add(&hypr_visual_choice(
        "general.layout",
        "Layout",
        "How windows share the screen as you open more of them.",
        cards(&[
            ("dwindle", "Dwindle: each new window splits the last in half", Sketch::Layout("dwindle")),
            ("master", "Master & stack: one big window, the rest stacked beside it", Sketch::Layout("master")),
            ("scrolling", "Scrolling: side-by-side columns you scroll through", Sketch::Layout("scrolling")),
        ]),
        false,
    ));
    g.add(&pictured(
        "dwindle.smart_split",
        "Split toward the cursor",
        "Dwindle: the new window opens on the side of the window the pointer is on.",
        Sketch::SplitTowardCursor,
    ));
    g.add(&pictured(
        "dwindle.preserve_split",
        "Keep split direction",
        "Dwindle: a split stays side by side or stacked when windows move or close.",
        Sketch::PreserveSplit,
    ));
    g.add(&hypr_visual_choice(
        "master.new_status",
        "New windows in Master & stack",
        "Where a window you open goes.",
        cards(&[
            ("slave", "Join the stack", Sketch::NewWindow("slave")),
            ("master", "Become the master", Sketch::NewWindow("master")),
            ("inherit", "Next to the focused window", Sketch::NewWindow("inherit")),
        ]),
        false,
    ));
    g.add(&hypr_slider(
        "master.mfact",
        "Master width",
        "Master &amp; stack: how much of the screen the big window takes.",
        (0.2, 0.8, 0.01),
        2,
        "",
        false,
    ));
    g.add(&hypr_slider(
        "scrolling.column_width",
        "Column width",
        "Scrolling: how wide a new column is, as a share of the screen.",
        (0.2, 1.0, 0.05),
        2,
        "",
        false,
    ));

    let g = page.group("Focus");
    g.add(&hypr_visual_choice(
        "input.follow_mouse",
        "Focus and the mouse pointer",
        "Which window your typing goes to as the pointer moves.",
        cards(&[
            ("1", "Focus follows the pointer", Sketch::Focus(1)),
            ("0", "Click to focus", Sketch::Focus(0)),
            ("2", "Pointer hovers; a click moves the keys", Sketch::Focus(2)),
            ("3", "Keys never follow the pointer", Sketch::Focus(3)),
        ]),
        true,
    ));
    g.add(&pictured(
        "misc.focus_on_activate",
        "Focus apps that ask for it",
        "An app that wants attention (a link opened from elsewhere) comes to the front.",
        Sketch::FocusOnActivate,
    ));
    g.add(&pictured(
        "cursor.no_warps",
        "Keep the pointer still",
        "Moving focus with the keyboard leaves the pointer where it is instead of jumping to the window.",
        Sketch::NoWarps,
    ));

    let g = page.group("Workspaces");
    g.add(&pictured(
        "binds.workspace_back_and_forth",
        "Press again to go back",
        "Choosing the workspace you're on takes you back to the one you came from.",
        Sketch::BackAndForth,
    ));
    g.add(&pictured(
        "misc.enable_swallow",
        "Terminal swallowing",
        "A window app started from a terminal takes the terminal's place, and gives it back when it closes.",
        Sketch::Swallow,
    ));
    g.add(&pictured(
        "misc.middle_click_paste",
        "Middle-click paste",
        "Selected text pastes with the middle mouse button.",
        Sketch::MiddlePaste,
    ));
    super::workspace_rules::build(page);
    super::own_titlebars::build(page);
}
