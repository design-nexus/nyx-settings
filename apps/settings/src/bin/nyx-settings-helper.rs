//! Runs as root through pkexec for Settings pages that change the system. It does a
//! fixed set of changes and nothing else: every argument is checked here, because the
//! caller is a normal user's program.
//!
//!   nyx-settings-helper add-user NAME SHELL ADMIN(0|1)     password on stdin
//!   nyx-settings-helper delete-user NAME KEEP_HOME(0|1)
//!   nyx-settings-helper lock-user NAME | unlock-user NAME
//!   nyx-settings-helper set-password NAME                  password on stdin
//!   nyx-settings-helper set-shell NAME SHELL
//!   nyx-settings-helper add-group NAME | delete-group NAME | rename-group OLD NEW
//!   nyx-settings-helper rename-user OLD NEW MOVE_HOME(0|1) | set-fullname NAME TEXT
//!   nyx-settings-helper add-member USER GROUP | remove-member USER GROUP
//!   nyx-settings-helper firewall enable|disable|rules | allow PORT PROTO | delete N
//!   nyx-settings-helper set-timezone ZONE | set-ntp 0|1 | set-time 'YYYY-MM-DD HH:MM:SS'
//!   nyx-settings-helper enable-locale 'LOCALE CHARSET' | set-locale LOCALE | set-hostname NAME
//!   nyx-settings-helper logind KEY VALUE | sleep-delay SECONDS
//!   nyx-settings-helper service start|stop|restart|enable|disable|enable-now|disable-now UNIT
//!   nyx-settings-helper printer-remove NAME | printer-add NAME URI | printer-set-uri NAME URI
//!   nyx-settings-helper mount-add-net ID TYPE SOURCE DIR OPTIONS    SMB user, password, domain on stdin
//!   nyx-settings-helper mount-test TYPE SOURCE OPTIONS              same stdin; mounts once, then not
//!   nyx-settings-helper mount-add-drive ID UUID DIR TYPE OPTIONS
//!   nyx-settings-helper mount-remove ID | mount-now DIR | unmount DIR | mkdir-mount DIR
//!   nyx-settings-helper password-free on|off   no password for disks, mounts and cleanup
//!   nyx-settings-helper clean-pkg-cache | remove-orphans | vacuum-journal SIZE
//!   nyx-settings-helper --version              needs no root; Settings compares it

#[path = "../backend/helper_version.rs"]
mod helper_version;
#[path = "../backend/mounts.rs"]
#[allow(dead_code)]
mod mounts;
#[path = "../backend/passwd.rs"]
mod passwd;

use passwd::{Group, User};
use std::io::Write;
use std::process::{Command, Stdio};

type Result<T> = std::result::Result<T, String>;

const ADMIN_GROUP: &str = "wheel";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    match run(&refs) {
        Ok(out) => print!("{out}"),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}

/// Who asked: pkexec records the real user.
fn invoker() -> Option<u32> {
    std::env::var("PKEXEC_UID").ok()?.parse().ok()
}

fn user(name: &str) -> Result<User> {
    if !passwd::valid_name(name) {
        return Err(format!("“{name}” isn't a valid user name"));
    }
    passwd::users().into_iter().find(|u| u.name == name).ok_or_else(|| format!("There is no user “{name}”"))
}

/// A user this helper may change: a person, not a system account.
fn human(name: &str) -> Result<User> {
    let u = user(name)?;
    if !u.is_human() {
        return Err(format!("“{name}” is a system account"));
    }
    Ok(u)
}

fn group(name: &str) -> Result<Group> {
    if !passwd::valid_name(name) {
        return Err(format!("“{name}” isn't a valid group name"));
    }
    passwd::groups().into_iter().find(|g| g.name == name).ok_or_else(|| format!("There is no group “{name}”"))
}

fn shell(path: &str) -> Result<&str> {
    if passwd::shells().iter().any(|s| s == path) {
        Ok(path)
    } else {
        Err(format!("{path} isn't a login shell listed in /etc/shells"))
    }
}

fn flag(s: &str) -> Result<bool> {
    match s {
        "0" => Ok(false),
        "1" => Ok(true),
        _ => Err("expected 0 or 1".into()),
    }
}

fn exec(program: &str, args: &[&str], stdin: Option<&str>) -> Result<String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not start {program}: {e}"))?;
    if let (Some(text), Some(mut pipe)) = (stdin, child.stdin.take()) {
        let _ = pipe.write_all(text.as_bytes());
    }
    // Reads stdout and stderr together, so a chatty child can't fill one pipe and stall.
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).to_string());
    }
    let err = String::from_utf8_lossy(&output.stderr);
    let err = err.trim();
    Err(if err.is_empty() { format!("{program} failed") } else { err.to_string() })
}

/// The password, read from stdin: one line, never an argument.
fn password() -> Result<String> {
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).map_err(|e| e.to_string())?;
    let pw = line.trim_end_matches(['\n', '\r']).to_string();
    if pw.is_empty() { Err("The password is empty".into()) } else { Ok(pw) }
}

fn set_password(name: &str, pw: &str) -> Result<String> {
    exec("/usr/bin/chpasswd", &[], Some(&format!("{name}:{pw}\n")))
}

fn run(args: &[&str]) -> Result<String> {
    match args {
        ["--version"] => Ok(format!("{}\n", helper_version::HELPER_VERSION)),
        ["add-user", name, sh, admin] => {
            if !passwd::valid_name(name) {
                return Err(format!("“{name}” isn't a valid user name. Use lower-case letters, digits, - and _."));
            }
            if passwd::users().iter().any(|u| u.name == *name) || passwd::groups().iter().any(|g| g.name == *name) {
                return Err(format!("“{name}” is already taken"));
            }
            let sh = shell(sh)?;
            let pw = password()?;
            let mut a = vec!["-m", "-s", sh];
            if flag(admin)? {
                a.extend(["-G", ADMIN_GROUP]);
            }
            a.push(name);
            exec("/usr/bin/useradd", &a, None)?;
            if let Err(e) = set_password(name, &pw) {
                let _ = exec("/usr/bin/userdel", &["-r", name], None);
                return Err(e);
            }
            Ok(String::new())
        }
        ["delete-user", name, keep_home] => {
            let u = human(name)?;
            if Some(u.uid) == invoker() {
                return Err("You can't delete the account you're signed in with".into());
            }
            if flag(keep_home)? { exec("/usr/bin/userdel", &[name], None) } else { exec("/usr/bin/userdel", &["-r", name], None) }
        }
        ["lock-user", name] => {
            let u = human(name)?;
            if Some(u.uid) == invoker() {
                return Err("You can't lock the account you're signed in with".into());
            }
            exec("/usr/bin/usermod", &["-L", name], None)
        }
        ["unlock-user", name] => {
            human(name)?;
            exec("/usr/bin/usermod", &["-U", name], None)
        }
        ["set-password", name] => {
            human(name)?;
            set_password(name, &password()?)
        }
        ["set-shell", name, sh] => {
            human(name)?;
            exec("/usr/bin/usermod", &["-s", shell(sh)?, name], None)
        }
        ["rename-user", old, new, move_home] => {
            let u = human(old)?;
            if Some(u.uid) == invoker() {
                return Err("You can't rename the account you're signed in with".into());
            }
            if !passwd::valid_name(new) {
                return Err(format!("“{new}” isn't a valid user name. Use lower-case letters, digits, - and _."));
            }
            if passwd::users().iter().any(|x| x.name == *new) || passwd::groups().iter().any(|g| g.name == *new) {
                return Err(format!("“{new}” is already taken"));
            }
            let move_home = flag(move_home)? && u.home == format!("/home/{old}");
            let home = format!("/home/{new}");
            let mut a = vec!["-l", new];
            if move_home {
                a.extend(["-d", &home, "-m"]);
            }
            a.push(old);
            exec("/usr/bin/usermod", &a, None)?;
            // The private group that shares the old name follows it.
            if passwd::groups().iter().any(|g| g.name == *old && g.gid == u.gid) {
                exec("/usr/bin/groupmod", &["-n", new, old], None)?;
            }
            Ok(String::new())
        }
        ["set-fullname", name, text] => {
            human(name)?;
            if !valid_fullname(text) {
                return Err("A full name can't contain : or , or control characters".into());
            }
            exec("/usr/bin/usermod", &["-c", text, name], None)
        }
        ["rename-group", old, new] => {
            let g = group(old)?;
            if g.is_system() || passwd::users().iter().any(|u| u.gid == g.gid) {
                return Err(format!("“{old}” can't be renamed here"));
            }
            if !passwd::valid_name(new) {
                return Err(format!("“{new}” isn't a valid group name. Use lower-case letters, digits, - and _."));
            }
            if passwd::groups().iter().any(|x| x.name == *new) || passwd::users().iter().any(|u| u.name == *new) {
                return Err(format!("“{new}” is already taken"));
            }
            exec("/usr/bin/groupmod", &["-n", new, old], None)
        }
        ["add-group", name] => {
            if !passwd::valid_name(name) {
                return Err(format!("“{name}” isn't a valid group name. Use lower-case letters, digits, - and _."));
            }
            if passwd::groups().iter().any(|g| g.name == *name) {
                return Err(format!("There is already a group “{name}”"));
            }
            exec("/usr/bin/groupadd", &[name], None)
        }
        ["delete-group", name] => {
            let g = group(name)?;
            if g.is_system() {
                return Err(format!("“{name}” is a system group"));
            }
            if passwd::users().iter().any(|u| u.gid == g.gid) {
                return Err(format!("“{name}” is a user's main group"));
            }
            exec("/usr/bin/groupdel", &[name], None)
        }
        ["add-member", who, grp] => {
            human(who)?;
            group(grp)?;
            exec("/usr/bin/gpasswd", &["-a", who, grp], None)
        }
        ["remove-member", who, grp] => {
            let u = human(who)?;
            group(grp)?;
            if *grp == ADMIN_GROUP && Some(u.uid) == invoker() {
                return Err("You can't remove your own administrator rights".into());
            }
            exec("/usr/bin/gpasswd", &["-d", who, grp], None)
        }
        ["firewall", "enable"] => exec("/usr/bin/ufw", &["--force", "enable"], None),
        ["firewall", "disable"] => exec("/usr/bin/ufw", &["disable"], None),
        ["firewall", "rules"] => exec("/usr/bin/ufw", &["status", "numbered"], None),
        ["firewall", "allow", port, proto] => {
            if !valid_port(port) {
                return Err(format!("“{port}” isn't a port number (1–65535, or a range like 8000:8010)"));
            }
            let rule = match *proto {
                "tcp" | "udp" => format!("{port}/{proto}"),
                "any" => port.to_string(),
                _ => return Err("protocol must be tcp, udp or any".into()),
            };
            exec("/usr/bin/ufw", &["allow", &rule], None)
        }
        ["firewall", "delete", n] if !n.is_empty() && n.len() < 5 && n.chars().all(|c| c.is_ascii_digit()) => {
            exec("/usr/bin/ufw", &["--force", "delete", n], None)
        }
        ["set-timezone", zone] => {
            if !valid_zone(zone) {
                return Err(format!("“{zone}” isn't a time zone"));
            }
            exec("/usr/bin/timedatectl", &["set-timezone", zone], None)
        }
        ["set-ntp", on] => exec("/usr/bin/timedatectl", &["set-ntp", if flag(on)? { "true" } else { "false" }], None),
        ["set-time", when] => {
            if !valid_time(when) {
                return Err("Use the form 2026-01-31 14:05:00".into());
            }
            // Setting the clock by hand only works with network time off.
            exec("/usr/bin/timedatectl", &["set-ntp", "false"], None)?;
            exec("/usr/bin/timedatectl", &["set-time", when], None)
        }
        ["enable-locale", entry] => enable_locale(entry),
        ["set-locale", locale] => {
            if !valid_locale(locale) {
                return Err(format!("“{locale}” isn't a locale name"));
            }
            exec("/usr/bin/localectl", &["set-locale", &format!("LANG={locale}")], None)
        }
        ["set-hostname", name] => {
            if !valid_hostname(name) {
                return Err("A host name is letters, digits and -, up to 63 characters, not starting or ending with -".into());
            }
            exec("/usr/bin/hostnamectl", &["set-hostname", name], None)
        }
        ["logind", key, value] => set_logind(key, value),
        ["sleep-delay", secs] => {
            if secs.is_empty() || secs.len() > 6 || !secs.chars().all(|c| c.is_ascii_digit()) {
                return Err("expected a number of seconds".into());
            }
            std::fs::create_dir_all("/etc/systemd/sleep.conf.d").map_err(|e| e.to_string())?;
            std::fs::write("/etc/systemd/sleep.conf.d/50-settings.conf", format!("[Sleep]\nHibernateDelaySec={secs}\n"))
                .map_err(|e| e.to_string())?;
            Ok(String::new())
        }
        ["service", action, unit] => {
            if !valid_unit(unit) {
                return Err(format!("“{unit}” isn't a unit name"));
            }
            let a: &[&str] = match *action {
                "start" => &["start"],
                "stop" => &["stop"],
                "restart" => &["restart"],
                "enable" => &["enable"],
                "disable" => &["disable"],
                "enable-now" => &["enable", "--now"],
                "disable-now" => &["disable", "--now"],
                _ => return Err("unknown service action".into()),
            };
            let mut full = a.to_vec();
            full.push(unit);
            exec("/usr/bin/systemctl", &full, None)
        }
        ["printer-remove", name] => {
            if !valid_printer(name) {
                return Err("not a printer name".into());
            }
            exec("/usr/bin/lpadmin", &["-x", name], None)
        }
        ["printer-set-uri", name, uri] => {
            if !valid_printer(name) {
                return Err("not a printer name".into());
            }
            if !valid_printer_uri(uri) {
                return Err("The address should look like ipp://host/ipp/print or socket://host".into());
            }
            exec("/usr/bin/lpadmin", &["-p", name, "-v", uri], None)
        }
        ["printer-add", name, uri] => {
            if !valid_printer(name) {
                return Err("Use letters, digits, - and _ for the printer name".into());
            }
            if !valid_printer_uri(uri) {
                return Err("The address should look like ipp://host/ipp/print or socket://host".into());
            }
            exec("/usr/bin/lpadmin", &["-p", name, "-E", "-v", uri, "-m", "everywhere"], None)
        }
        ["mount-add-net", id, fstype, source, dir, options] => mount_add_net(id, fstype, source, dir, options),
        ["mount-add-drive", id, uuid, dir, fstype, options] => mount_add_drive(id, uuid, dir, fstype, options),
        ["password-free", "on"] => password_free(true),
        ["password-free", "off"] => password_free(false),
        ["mount-test", fstype, source, options] => mount_test(fstype, source, options),
        ["mount-remove", id] => mount_remove(id),
        ["mount-now", dir] => {
            fstab_dir(dir)?;
            exec("/usr/bin/mount", &[dir], None)
        }
        ["unmount", dir] => {
            fstab_dir(dir)?;
            exec("/usr/bin/umount", &[dir], None)
        }
        ["mkdir-mount", dir] => {
            if !mounts::valid_dir(dir, &invoker_home()?) {
                return Err(format!("A mount folder has to be inside /mnt, /media or your home folder, not “{dir}”"));
            }
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            Ok(String::new())
        }
        ["clean-pkg-cache"] => {
            if std::path::Path::new("/usr/bin/paccache").exists() {
                exec("/usr/bin/paccache", &["-rk1"], None)?;
                exec("/usr/bin/paccache", &["-ruk0"], None)
            } else {
                exec("/usr/bin/pacman", &["-Sc", "--noconfirm"], None)
            }
        }
        ["remove-orphans"] => {
            let list = Command::new("/usr/bin/pacman").args(["-Qdtq"]).output().map_err(|e| e.to_string())?;
            let names: Vec<String> =
                String::from_utf8_lossy(&list.stdout).lines().map(str::to_string).filter(|n| passwd_safe_pkg(n)).collect();
            if names.is_empty() {
                return Ok(String::new());
            }
            let mut a = vec!["-Rns", "--noconfirm", "--"];
            a.extend(names.iter().map(String::as_str));
            exec("/usr/bin/pacman", &a, None)
        }
        ["vacuum-journal", size] => {
            if !size.ends_with('M') || size.len() > 6 || !size[..size.len() - 1].chars().all(|c| c.is_ascii_digit()) {
                return Err("expected a size like 200M".into());
            }
            exec("/usr/bin/journalctl", &[&format!("--vacuum-size={size}")], None)
        }
        _ => Err("unknown or malformed request".into()),
    }
}

// ----- Checks for the system changes -----

fn valid_fullname(s: &str) -> bool {
    s.len() <= 100 && s.chars().all(|c| !c.is_control() && c != ':' && c != ',')
}

fn valid_port(s: &str) -> bool {
    let one = |p: &str| p.parse::<u32>().is_ok_and(|n| (1..=65535).contains(&n)) && p.chars().all(|c| c.is_ascii_digit());
    match s.split_once(':') {
        Some((a, b)) => one(a) && one(b) && a.parse::<u32>().ok() < b.parse::<u32>().ok(),
        None => one(s),
    }
}

/// An existing zone file, with only the characters zone names use.
fn valid_zone(s: &str) -> bool {
    !s.is_empty()
        && s.len() < 64
        && !s.starts_with('/')
        && !s.split('/').any(|p| p.is_empty() || p == "." || p == "..")
        && s.chars().all(|c| c.is_ascii_alphanumeric() || "/_+-".contains(c))
        && std::path::Path::new("/usr/share/zoneinfo").join(s).is_file()
}

fn valid_time(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 19
        && b.iter().enumerate().all(|(i, c)| match i {
            4 | 7 => *c == b'-',
            10 => *c == b' ',
            13 | 16 => *c == b':',
            _ => c.is_ascii_digit(),
        })
}

fn valid_locale(s: &str) -> bool {
    !s.is_empty() && s.len() < 40 && s.chars().all(|c| c.is_ascii_alphanumeric() || "_.@-".contains(c))
}

fn valid_hostname(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 63
        && !s.starts_with('-')
        && !s.ends_with('-')
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

fn valid_unit(s: &str) -> bool {
    let Some((name, kind)) = s.rsplit_once('.') else { return false };
    !name.is_empty()
        && name.len() < 100
        && ["service", "socket", "timer"].contains(&kind)
        && !name.starts_with('-')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || ":_.@\\-".contains(c))
}

fn valid_printer(s: &str) -> bool {
    !s.is_empty() && s.len() <= 127 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn valid_printer_uri(s: &str) -> bool {
    ["ipp://", "ipps://", "socket://", "lpd://", "http://", "https://", "dnssd://"].iter().any(|p| s.starts_with(p))
        && s.len() < 300
        && s.chars().all(|c| c.is_ascii_graphic() && !"\"'`$;|&<>\\".contains(c))
}

/// Settings that logind accepts in `logind.conf.d`, and the values they may take.
const LOGIND_KEYS: &[&str] = &[
    "HandleLidSwitch",
    "HandleLidSwitchExternalPower",
    "HandleLidSwitchDocked",
    "HandlePowerKey",
    "HandleSuspendKey",
    "HandleHibernateKey",
];
const LOGIND_ACTIONS: &[&str] =
    &["ignore", "poweroff", "reboot", "halt", "suspend", "hibernate", "hybrid-sleep", "suspend-then-hibernate", "lock"];
const LOGIND_FILE: &str = "/etc/systemd/logind.conf.d/50-settings.conf";

fn set_logind(key: &str, value: &str) -> Result<String> {
    if !LOGIND_KEYS.contains(&key) || !LOGIND_ACTIONS.contains(&value) {
        return Err("not a setting this can change".into());
    }
    let old = std::fs::read_to_string(LOGIND_FILE).unwrap_or_default();
    let text = merge_logind(&old, key, value);
    std::fs::create_dir_all("/etc/systemd/logind.conf.d").map_err(|e| e.to_string())?;
    std::fs::write(LOGIND_FILE, text).map_err(|e| e.to_string())?;
    // Ask logind to reread its configuration without ending any session.
    exec("/usr/bin/systemctl", &["kill", "-s", "HUP", "systemd-logind"], None)
}

/// The drop-in with `key` set to `value`, keeping the other keys.
fn merge_logind(old: &str, key: &str, value: &str) -> String {
    let mut lines: Vec<String> =
        old.lines().filter(|l| !l.starts_with('[') && !l.trim().is_empty()).map(str::to_string).collect();
    let prefix = format!("{key}=");
    lines.retain(|l| !l.starts_with(&prefix));
    lines.push(format!("{key}={value}"));
    lines.sort();
    format!("[Login]\n{}\n", lines.join("\n"))
}

/// Uncomment one line of `/etc/locale.gen` that glibc supports, then build it.
fn enable_locale(entry: &str) -> Result<String> {
    let supported = std::fs::read_to_string("/usr/share/i18n/SUPPORTED").unwrap_or_default();
    if !supported.lines().any(|l| l.trim() == entry) {
        return Err(format!("“{entry}” isn't a supported locale"));
    }
    let gen_text = std::fs::read_to_string("/etc/locale.gen").map_err(|e| e.to_string())?;
    let (text, found) = uncomment_locale(&gen_text, entry);
    let text = if found {
        text
    } else {
        format!("{}{entry}\n", if gen_text.ends_with('\n') { gen_text } else { format!("{gen_text}\n") })
    };
    std::fs::write("/etc/locale.gen", text).map_err(|e| e.to_string())?;
    exec("/usr/bin/locale-gen", &[], None)
}

fn uncomment_locale(text: &str, entry: &str) -> (String, bool) {
    let mut found = false;
    let lines: Vec<String> = text
        .lines()
        .map(|l| {
            if l.trim_start_matches('#').trim() == entry {
                found = true;
                entry.to_string()
            } else {
                l.to_string()
            }
        })
        .collect();
    (format!("{}\n", lines.join("\n")), found)
}

// ---------- Mounts ----------

/// The signed-in user's home folder, where their mounts may also go.
fn invoker_home() -> Result<std::path::PathBuf> {
    let uid = invoker().ok_or("run through pkexec")?;
    passwd::users()
        .into_iter()
        .find(|u| u.uid == uid)
        .map(|u| std::path::PathBuf::from(u.home))
        .ok_or_else(|| "unknown user".into())
}

/// A package name pacman printed.
fn passwd_safe_pkg(n: &str) -> bool {
    !n.is_empty() && n.chars().all(|c| c.is_ascii_alphanumeric() || "@._+-".contains(c)) && !n.starts_with('-')
}

/// Only folders that have a line in fstab, other than the system's own.
fn fstab_dir(dir: &str) -> Result<()> {
    match mounts::fstab().into_iter().find(|m| m.dir == dir) {
        Some(m) if !m.system => Ok(()),
        Some(_) => Err(format!("{dir} is one of the system's own mounts")),
        None => Err(format!("{dir} isn't in /etc/fstab")),
    }
}

/// SMB user, password and domain from stdin (three lines; none for a guest).
fn smb_login() -> Result<Option<String>> {
    let mut text = String::new();
    std::io::Read::read_to_string(&mut std::io::stdin(), &mut text).map_err(|e| e.to_string())?;
    if text.trim().is_empty() {
        return Ok(None);
    }
    let mut lines = text.lines();
    let user = lines.next().unwrap_or_default();
    let pass = lines.next().unwrap_or_default();
    let domain = lines.next().unwrap_or_default();
    mounts::credentials_text(user, pass, domain).map(Some)
}

fn options_list(options: &str) -> Vec<String> {
    if options == "-" { Vec::new() } else { options.split(',').map(str::to_string).collect() }
}

fn write_private(path: &str, text: &str) -> Result<()> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    if let Some(dir) = std::path::Path::new(path).parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    let mut f =
        std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path).map_err(|e| e.to_string())?;
    f.write_all(text.as_bytes()).map_err(|e| e.to_string())
}

/// fstab replaced in one step, with a copy of the first version kept.
fn write_fstab(text: &str) -> Result<()> {
    if !std::path::Path::new("/etc/fstab.nyx-bak").exists() {
        let _ = std::fs::copy("/etc/fstab", "/etc/fstab.nyx-bak");
    }
    let tmp = "/etc/.fstab.nyx-new";
    std::fs::write(tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(tmp, "/etc/fstab").map_err(|e| e.to_string())?;
    let _ = exec("/usr/bin/systemctl", &["daemon-reload"], None);
    Ok(())
}

fn mount_add_net(id: &str, fstype: &str, source: &str, dir: &str, options: &str) -> Result<String> {
    let home = invoker_home()?;
    let creds = if fstype == "cifs" { smb_login()? } else { None };
    let opts = options_list(options);
    let line = mounts::network_line(id, fstype, source, dir, &opts, creds.is_some(), &home)?;
    let text = std::fs::read_to_string("/etc/fstab").map_err(|e| e.to_string())?;
    if mounts::parse_fstab(&text).iter().any(|m| m.id == id || m.dir == dir) {
        return Err(format!("Something already mounts at {dir}, or a mount is called {id}"));
    }
    if let Some(c) = &creds {
        write_private(&format!("{}/{id}", mounts::CREDENTIALS), c)?;
    }
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let mut new = text.trim_end().to_string();
    new.push('\n');
    new.push_str(&line);
    new.push('\n');
    write_fstab(&new)?;
    // Mounted on first use from now on: start the automount, or mount now.
    if opts.iter().any(|o| o == "x-systemd.automount") {
        let _ = exec("/usr/bin/systemctl", &["restart", "remote-fs.target"], None);
        Ok(String::new())
    } else {
        exec("/usr/bin/mount", &[dir], None).map(|_| String::new())
    }
}

fn mount_add_drive(id: &str, uuid: &str, dir: &str, fstype: &str, options: &str) -> Result<String> {
    let home = invoker_home()?;
    let opts = options_list(options);
    let line = mounts::drive_line(id, uuid, dir, fstype, &opts, &home)?;
    let text = std::fs::read_to_string("/etc/fstab").map_err(|e| e.to_string())?;
    let entries = mounts::parse_fstab(&text);
    if entries.iter().any(|m| m.id == id || m.dir == dir) {
        return Err(format!("Something already mounts at {dir}, or a mount is called {id}"));
    }
    if entries.iter().any(|m| m.source == format!("UUID={uuid}")) {
        return Err("That partition already mounts at startup".into());
    }
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let mut new = text.trim_end().to_string();
    new.push('\n');
    new.push_str(&line);
    new.push('\n');
    write_fstab(&new)?;
    if opts.iter().any(|o| o == "x-systemd.automount") {
        // Its automount unit exists after the daemon-reload; start it now.
        let unit = exec("/usr/bin/systemd-escape", &["-p", "--suffix=automount", dir], None)?;
        let _ = exec("/usr/bin/systemctl", &["start", unit.trim()], None);
        Ok(String::new())
    } else {
        exec("/usr/bin/mount", &[dir], None).map(|_| String::new())
    }
}

/// Turn the password-free rule for disks, mounts and cleanup on or off, for
/// the user who asked.
fn password_free(on: bool) -> Result<String> {
    if !on {
        match std::fs::remove_file(mounts::RULE_FILE) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
        return Ok(String::new());
    }
    let uid = invoker().ok_or("run through pkexec")?;
    let user = passwd::users().into_iter().find(|u| u.uid == uid).ok_or("unknown user")?;
    if !passwd::valid_name(&user.name) {
        return Err("unusual user name".into());
    }
    let helper = std::env::current_exe().map_err(|e| e.to_string())?;
    let rule = mounts::password_free_rule(&user.name, &helper.to_string_lossy());
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(mounts::RULE_FILE, rule).map_err(|e| e.to_string())?;
    std::fs::set_permissions(mounts::RULE_FILE, std::fs::Permissions::from_mode(0o644)).map_err(|e| e.to_string())?;
    Ok(String::new())
}

fn mount_remove(id: &str) -> Result<String> {
    if !mounts::valid_id(id) {
        return Err("bad id".into());
    }
    let text = std::fs::read_to_string("/etc/fstab").map_err(|e| e.to_string())?;
    let Some(m) = mounts::parse_fstab(&text).into_iter().find(|m| m.id == id) else {
        return Err("There's no such mount".into());
    };
    let _ = exec("/usr/bin/umount", &["-l", &m.dir], None);
    write_fstab(&mounts::without(&text, id))?;
    let _ = std::fs::remove_file(format!("{}/{id}", mounts::CREDENTIALS));
    // Its folder goes too if it's empty.
    let _ = std::fs::remove_dir(&m.dir);
    Ok(String::new())
}

fn mount_test(fstype: &str, source: &str, options: &str) -> Result<String> {
    let creds = if fstype == "cifs" { smb_login()? } else { None };
    let dir = format!("/run/nyx-settings-test-{}", std::process::id());
    let mut opts: Vec<String> =
        options_list(options).into_iter().filter(|o| !o.starts_with("x-systemd") && o != "nofail" && o != "_netdev").collect();
    // Checked as a line would be, with a stand-in folder.
    mounts::network_line("test", fstype, source, "/mnt/test", &opts, false, std::path::Path::new("/nonexistent"))?;
    let cred_file = format!("/run/nyx-settings-test-{}.cred", std::process::id());
    match &creds {
        Some(c) => {
            write_private(&cred_file, c)?;
            opts.push(format!("credentials={cred_file}"));
        }
        None if fstype == "cifs" => opts.push("guest".into()),
        None => {}
    }
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut a = vec!["20", "/usr/bin/mount", "-t", fstype, source, dir.as_str()];
    let joined = opts.join(",");
    if !joined.is_empty() {
        a.extend(["-o", joined.as_str()]);
    }
    let r = exec("/usr/bin/timeout", &a, None);
    if r.is_ok() {
        let _ = exec("/usr/bin/umount", &[&dir], None);
    }
    let _ = std::fs::remove_dir(&dir);
    let _ = std::fs::remove_file(&cred_file);
    r.map(|_| String::new())
        .map_err(|e| if e.contains("timeout failed") || e.is_empty() { "It didn't answer within 20 seconds".into() } else { e })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_malformed_requests() {
        for args in [
            &[][..],
            &["rm", "-rf"],
            &["add-user", "x"],
            &["delete-user", "root", "0"],
            &["firewall", "allow", "22; ls", "tcp"],
            &["firewall", "delete", "x"],
            &["rename-user", "root", "x", "0"],
            &["rename-group", "wheel", "x"],
            &["set-fullname", "root", "a"],
            &["printer-set-uri", "a b", "ipp://x"],
            &["service", "start", "a b.service"],
            &["logind", "HandleLidSwitch", "rm"],
            &["set-timezone", "../etc/passwd"],
            &["lock-user", "a b"],
            &["add-group", "Bad"],
            &["set-shell", "root", "/bin/sh"],
        ] {
            assert!(run(args).is_err(), "{args:?}");
        }
    }

    #[test]
    fn full_names() {
        assert!(valid_fullname("Ken Smith") && valid_fullname("") && valid_fullname("Zoë O'Neil"));
        assert!(!valid_fullname("a:b") && !valid_fullname("a,b") && !valid_fullname("a\nb"));
    }

    #[test]
    fn ports() {
        for ok in ["22", "65535", "8000:8010"] {
            assert!(valid_port(ok), "{ok}");
        }
        for bad in ["", "0", "65536", "22; ls", "a", "9000:8000", "1:", "-1"] {
            assert!(!valid_port(bad), "{bad}");
        }
    }

    #[test]
    fn names_and_times() {
        assert!(valid_hostname("hades-2") && !valid_hostname("-a") && !valid_hostname("a b") && !valid_hostname(""));
        assert!(valid_time("2026-01-31 14:05:00") && !valid_time("2026-01-31T14:05:00") && !valid_time("now"));
        assert!(
            valid_unit("cups.service")
                && valid_unit("getty@tty1.service")
                && !valid_unit("a b.service")
                && !valid_unit("x.mount")
                && !valid_unit("--now.service")
        );
        assert!(valid_printer("HP_LaserJet-1") && !valid_printer("a b") && !valid_printer(""));
        assert!(
            valid_printer_uri("ipp://printer.local/ipp/print")
                && !valid_printer_uri("file:///etc/passwd")
                && !valid_printer_uri("ipp://a;b")
        );
        assert!(valid_locale("en_US.UTF-8") && !valid_locale("en US") && !valid_locale(""));
        assert!(!valid_zone("../etc/passwd") && !valid_zone("/etc/passwd") && !valid_zone("") && !valid_zone("Not/AZone"));
    }

    #[test]
    fn logind_merge() {
        let a = merge_logind("", "HandleLidSwitch", "ignore");
        assert_eq!(a, "[Login]\nHandleLidSwitch=ignore\n");
        let b = merge_logind(&a, "HandlePowerKey", "suspend");
        let c = merge_logind(&b, "HandleLidSwitch", "hibernate");
        assert_eq!(c, "[Login]\nHandleLidSwitch=hibernate\nHandlePowerKey=suspend\n");
    }

    #[test]
    fn locale_gen() {
        let (t, found) = uncomment_locale("#en_US.UTF-8 UTF-8\n#de_DE.UTF-8 UTF-8\n", "de_DE.UTF-8 UTF-8");
        assert!(found);
        assert_eq!(t, "#en_US.UTF-8 UTF-8\nde_DE.UTF-8 UTF-8\n");
        assert!(!uncomment_locale("# x\n", "fr_FR.UTF-8 UTF-8").1);
    }

    #[test]
    fn flags() {
        assert!(flag("1").unwrap());
        assert!(!flag("0").unwrap());
        assert!(flag("yes").is_err());
    }
}
