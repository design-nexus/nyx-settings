//! Printers on NyxOS, through cups-pk-helper: CUPS's polkit-authorized D-Bus service,
//! the one GNOME uses. Printers are CUPS's own state, so they change right away.

use crate::window;
use gtk::gio;
use gtk::glib::{self, variant::ToVariant};

const NAME: &str = "org.opensuse.CupsPkHelper.Mechanism";

/// Call `method`; cups-pk-helper answers with an error string, empty when it worked.
fn call(method: &'static str, params: glib::Variant, done: impl FnOnce(Result<(), String>) + 'static) {
    let conn = match gio::bus_get_sync(gio::BusType::System, None::<&gio::Cancellable>) {
        Ok(c) => c,
        Err(e) => return done(Err(e.message().to_string())),
    };
    conn.call(
        Some(NAME),
        "/",
        NAME,
        method,
        Some(&params),
        Some(glib::VariantTy::new("(s)").expect("a valid type")),
        gio::DBusCallFlags::ALLOW_INTERACTIVE_AUTHORIZATION,
        120_000,
        None::<&gio::Cancellable>,
        move |r| {
            done(match r {
                Ok(v) => match v.get::<(String,)>() {
                    Some((e,)) if !e.is_empty() => Err(e),
                    _ => Ok(()),
                },
                Err(e) => Err(e.message().to_string()),
            })
        },
    );
}

fn finish(done: &'static str, section: &'static str) -> impl FnOnce(Result<(), String>) + 'static {
    move |r| {
        match r {
            Ok(()) => window::toast(done),
            Err(e) => window::toast(&e),
        }
        window::rebuild(section);
    }
}

/// A driverless (IPP Everywhere) printer, turned on and taking jobs.
pub fn add(name: &str, uri: &str, section: &'static str) {
    let name = name.to_string();
    call("PrinterAdd", (name.as_str(), uri, "everywhere", "", "").to_variant(), move |r| {
        if let Err(e) = r {
            return finish("", section)(Err(e));
        }
        call("PrinterSetEnabled", (name.as_str(), true).to_variant(), move |_| {
            call("PrinterSetAcceptJobs", (name.as_str(), true, "").to_variant(), finish("Printer added", section));
        });
    });
}

pub fn remove(name: &str, section: &'static str) {
    call("PrinterDelete", (name,).to_variant(), finish("Printer removed", section));
}

pub fn set_uri(name: &str, uri: &str, section: &'static str) {
    call("PrinterSetDevice", (name, uri).to_variant(), finish("Printer updated", section));
}
