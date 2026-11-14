//! Corsair keyboards for Settings.
//!
//! settings-corsair pages | describe PAGE | set PAGE KEY VALUE | probe [--no-query] [FILE]

mod access;
mod bragi;
mod descriptor;
mod extension;
mod hidraw;
mod mock;
mod models;
mod probe;
mod usb;

use anyhow::{Result, bail};

fn run(args: &[String]) -> Result<()> {
    let arg = |i: usize| args.get(i).map(String::as_str).unwrap_or("");
    match arg(0) {
        "pages" => println!("{}", extension::pages()),
        "describe" => println!("{}", extension::describe(arg(1))?),
        "set" => {
            if let Some(reply) = extension::set(arg(1), arg(2), arg(3))? {
                println!("{reply}");
            }
        }
        // Simulated lights following the theme take its colour when drawn.
        "theme-changed" if mock::on() => {}
        "theme-changed" => nyx_ext::lighting::theme_changed()?,
        "probe" => {
            let ask = !args.iter().any(|a| a == "--no-query");
            let report = probe::report(ask);
            match args.iter().skip(1).find(|a| !a.starts_with("--")) {
                Some(path) => {
                    std::fs::write(path, &report)?;
                    eprintln!("Saved {path}");
                }
                None => print!("{report}"),
            }
        }
        _ => bail!("usage: settings-corsair pages | describe PAGE | set PAGE KEY VALUE | probe [--no-query] [FILE]"),
    }
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(e) = run(&args) {
        eprintln!("{e:#}");
        std::process::exit(1);
    }
}
