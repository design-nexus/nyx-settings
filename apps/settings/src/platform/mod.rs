//! The platform Settings is built for: Omarchy (the default) or NyxOS (the `nyxos`
//! feature). The pages are shared; what a platform does its own way (its sidebar, its
//! system pages, its updates and version) comes from here, so pages never ask which
//! platform they're on.

pub mod nyxos;
mod omarchy;

use crate::sections::Section;
use crate::sections::home::Hero;
use crate::widgets::Page;
use std::rc::Rc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Omarchy,
    NyxOS,
}

pub const PLATFORM: Platform = if cfg!(feature = "nyxos") { Platform::NyxOS } else { Platform::Omarchy };

/// The sidebar's pages: the platform's categories, names and own pages.
pub fn layout(list: &mut [Section]) {
    if PLATFORM == Platform::NyxOS {
        nyxos::layout::apply(list);
    }
}

/// Pages that open from another page instead of the sidebar.
pub fn off_sidebar(id: &str) -> bool {
    PLATFORM == Platform::NyxOS && nyxos::layout::off_sidebar(id)
}

/// The group whose colours a sidebar group takes.
pub fn colour_group(group: &str) -> &str {
    match PLATFORM {
        Platform::NyxOS => nyxos::layout::colour_group(group),
        Platform::Omarchy => group,
    }
}

/// Home's updates: Omarchy's packages, or NyxOS's generation.
pub fn home_updates(page: &Page, hero: Rc<Hero>) {
    match PLATFORM {
        Platform::NyxOS => nyxos::home_updates(page, &hero),
        Platform::Omarchy => crate::sections::home::updates_group(page, hero),
    }
}

/// About's group for the system itself.
pub fn about(page: &Page) {
    match PLATFORM {
        Platform::NyxOS => nyxos::about(page),
        Platform::Omarchy => omarchy::about(page),
    }
}
