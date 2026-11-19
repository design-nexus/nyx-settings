//! Mounts: what's in `/etc/fstab`, what's mounted, and SSH folders run as user
//! services. Plain Rust with no GTK: the root helper includes this file to
//! check and build fstab lines exactly as Settings does.

use std::path::{Path, PathBuf};

/// Lines Settings adds end with this, then the mount's id.
pub const MARK: &str = "# nyx-settings:";
/// SMB passwords, one root-only file per mount.
pub const CREDENTIALS: &str = "/etc/nyx-settings/credentials";

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    Drive,
    Smb,
    Nfs,
    Ssh,
    Other,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Mount {
    /// Settings' id for mounts it added ("" for the user's own lines).
    pub id: String,
    pub kind: Kind,
    /// `UUID=…`, `//server/share`, `server:/path`, `user@host:path`.
    pub source: String,
    pub dir: String,
    pub fstype: String,
    pub options: String,
    /// The system's own mounts: `/`, `/boot`, `/home`, swap.
    pub system: bool,
}

impl Mount {
    pub fn ours(&self) -> bool {
        !self.id.is_empty()
    }
    pub fn on_demand(&self) -> bool {
        self.options.split(',').any(|o| o == "x-systemd.automount")
    }
}

const SYSTEM_DIRS: &[&str] = &[
    "/",
    "/boot",
    "/boot/efi",
    "/efi",
    "/home",
    "/usr",
    "/var",
    "/var/log",
    "/var/cache/pacman/pkg",
    "/.snapshots",
    "none",
    "swap",
];

/// fstab writes spaces and a few others as octal escapes.
pub fn unescape(s: &str) -> String {
    let mut out = String::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 4 <= b.len() && b[i + 1..i + 4].iter().all(|c| (b'0'..=b'7').contains(c)) {
            let n = u8::from_str_radix(&s[i + 1..i + 4], 8).unwrap_or(b'?');
            out.push(n as char);
            i += 4;
        } else {
            out.push(b[i] as char);
            i += 1;
        }
    }
    out
}

pub fn escape(s: &str) -> String {
    s.replace('\\', "\\134").replace(' ', "\\040").replace('\t', "\\011")
}

/// The entries in an fstab text.
pub fn parse_fstab(text: &str) -> Vec<Mount> {
    let mut out = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let (body, id) = match trimmed.find(MARK) {
            Some(i) => (&trimmed[..i], trimmed[i + MARK.len()..].trim().to_string()),
            None => (trimmed.split('#').next().unwrap_or(trimmed), String::new()),
        };
        let f: Vec<&str> = body.split_whitespace().collect();
        if f.len() < 3 {
            continue;
        }
        let (source, dir, fstype) = (unescape(f[0]), unescape(f[1]), f[2].to_string());
        let options = f.get(3).unwrap_or(&"defaults").to_string();
        let kind = match fstype.as_str() {
            "cifs" | "smb3" => Kind::Smb,
            "nfs" | "nfs4" => Kind::Nfs,
            "fuse.sshfs" => Kind::Ssh,
            _ if source.starts_with("UUID=")
                || source.starts_with("/dev/")
                || source.starts_with("LABEL=")
                || source.starts_with("PARTUUID=") =>
            {
                Kind::Drive
            }
            _ => Kind::Other,
        };
        let system = SYSTEM_DIRS.contains(&dir.as_str())
            || fstype == "swap"
            || options.split(',').any(|o| o.starts_with("subvol=")) && dir.starts_with("/var");
        out.push(Mount { id, kind, source, dir, fstype, options, system });
    }
    out
}

pub fn fstab() -> Vec<Mount> {
    parse_fstab(&std::fs::read_to_string("/etc/fstab").unwrap_or_default())
}

/// What's mounted now: (folder, filesystem type). A folder waiting to be
/// mounted on first use shows as `autofs`, with the real mount over it once used.
pub fn mounted() -> Vec<(String, String)> {
    std::fs::read_to_string("/proc/self/mounts")
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            let (_, dir, kind) = (f.next()?, f.next()?, f.next()?);
            Some((unescape(dir), kind.to_string()))
        })
        .collect()
}

// ---------- Checking what the caller asks for ----------

/// A mount id Settings makes: letters, digits and dashes.
pub fn valid_id(s: &str) -> bool {
    !s.is_empty() && s.len() <= 40 && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// A host name or address (IPv6 in brackets is fine).
pub fn valid_host(s: &str) -> bool {
    !s.is_empty() && s.len() <= 253 && s.chars().all(|c| c.is_ascii_alphanumeric() || ".-:[]_".contains(c)) && !s.starts_with('-')
}

/// A share or exported path: printable, no quotes, no control characters.
pub fn valid_path_text(s: &str) -> bool {
    !s.is_empty() && s.len() <= 255 && !s.contains("..") && s.chars().all(|c| !c.is_control() && !"\"'`$\\#,;".contains(c))
}

pub fn valid_user(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.chars().all(|c| c.is_ascii_alphanumeric() || "._-@".contains(c)) && !s.starts_with('-')
}

/// Where a mount may go: under /mnt, /media, or the home folder `home`.
pub fn valid_dir(dir: &str, home: &Path) -> bool {
    let p = Path::new(dir);
    if !p.is_absolute() || dir.contains("..") || !valid_path_text(dir.trim_start_matches('/')) || dir.ends_with('/') {
        return false;
    }
    let allowed = [PathBuf::from("/mnt"), PathBuf::from("/media"), home.to_path_buf()];
    allowed.iter().any(|root| p.starts_with(root) && p != root)
}

/// Options a network mount may have. `credentials=` is the helper's own.
pub fn valid_option(o: &str) -> bool {
    const FLAGS: &[&str] = &[
        "ro",
        "rw",
        "guest",
        "nofail",
        "_netdev",
        "noauto",
        "x-systemd.automount",
        "soft",
        "hard",
        "noatime",
        "user",
        "users",
        "x-gvfs-show",
        "iocharset=utf8",
    ];
    if FLAGS.contains(&o) {
        return true;
    }
    let Some((k, v)) = o.split_once('=') else { return false };
    let digits = |v: &str| !v.is_empty() && v.chars().all(|c| c.is_ascii_digit());
    match k {
        "vers" | "nfsvers" => !v.is_empty() && v.chars().all(|c| c.is_ascii_digit() || c == '.'),
        "uid" | "gid" | "timeo" | "retrans" | "x-systemd.mount-timeout" => digits(v),
        "x-systemd.idle-timeout" => digits(v.trim_end_matches("min").trim_end_matches('s')),
        "file_mode" | "dir_mode" | "umask" => v.len() <= 4 && v.chars().all(|c| ('0'..='7').contains(&c)),
        _ => false,
    }
}

/// Filesystems a drive at startup may have (as fstab names them).
pub const DRIVE_TYPES: &[&str] = &["ext4", "ext3", "ext2", "btrfs", "xfs", "f2fs", "vfat", "exfat", "ntfs3"];

/// A filesystem UUID as blkid prints it.
pub fn valid_uuid(s: &str) -> bool {
    (4..=64).contains(&s.len()) && s.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

/// The fstab line for a drive mounted at startup.
pub fn drive_line(id: &str, uuid: &str, dir: &str, fstype: &str, options: &[String], home: &Path) -> Result<String, String> {
    if !valid_id(id) {
        return Err("bad id".into());
    }
    if !valid_uuid(uuid) {
        return Err(format!("“{uuid}” isn't a filesystem id"));
    }
    if !DRIVE_TYPES.contains(&fstype) {
        return Err(format!("“{fstype}” isn't a filesystem Settings mounts"));
    }
    if !valid_dir(dir, home) {
        return Err(format!("A mount folder has to be inside /mnt, /media or your home folder, not “{dir}”"));
    }
    if let Some(bad) = options.iter().find(|o| !valid_option(o) && o.as_str() != "x-udisks-auth") {
        return Err(format!("“{bad}” isn't an option Settings sets"));
    }
    let opts = if options.is_empty() { "defaults".to_string() } else { options.join(",") };
    let pass = if fstype.starts_with("ext") { 2 } else { 0 };
    Ok(format!("UUID={uuid} {} {fstype} {opts} 0 {pass} {MARK}{id}", escape(dir)))
}

/// Helper commands the password-free rule covers: disks, mounts and cleanup.
/// Users, passwords, the firewall and the rest still ask.
pub const PASSWORD_FREE: &[&str] = &[
    "mount-add-net",
    "mount-add-drive",
    "mount-test",
    "mount-remove",
    "mount-now",
    "unmount",
    "mkdir-mount",
    "clean-pkg-cache",
    "remove-orphans",
    "vacuum-journal",
];

pub const RULE_FILE: &str = "/etc/polkit-1/rules.d/49-nyx-settings.rules";

/// The polkit rule that lets `user` make disk, mount and cleanup changes
/// without a password while signed in at the computer. Settings' helper is
/// allowed only for the commands in [`PASSWORD_FREE`].
pub fn password_free_rule(user: &str, helper: &str) -> String {
    let commands = PASSWORD_FREE.iter().map(|c| format!("\"{c}\"")).collect::<Vec<_>>().join(", ");
    format!(
        "// Written by Settings (Users → Passwords for system changes). Turn the switch off, or delete this file, to undo.\n\
         polkit.addRule(function(action, subject) {{\n\
         \x20   if (subject.user != \"{user}\" || !subject.local || !subject.active) return;\n\
         \x20   if (action.id.indexOf(\"org.freedesktop.udisks2.\") == 0) return polkit.Result.YES;\n\
         \x20   if (action.id == \"io.github.design_nexus.nyx.settings.admin\") {{\n\
         \x20       var line = (action.lookup(\"command_line\") || \"\").split(\" \");\n\
         \x20       if (line[0] == \"{helper}\" && [{commands}].indexOf(line[1]) >= 0) return polkit.Result.YES;\n\
         \x20   }}\n\
         }});\n"
    )
}

/// A network share's source: `//server/share` (SMB) or `server:/path` (NFS).
pub fn network_source(fstype: &str, server: &str, share: &str) -> Option<String> {
    if !valid_host(server) || !valid_path_text(share) {
        return None;
    }
    let share = share.trim_start_matches('/');
    match fstype {
        "cifs" => Some(format!("//{server}/{share}")),
        "nfs" | "nfs4" => Some(format!("{server}:/{share}")),
        _ => None,
    }
}

/// The fstab line for a network mount Settings adds, or why it can't be made.
pub fn network_line(
    id: &str,
    fstype: &str,
    source: &str,
    dir: &str,
    options: &[String],
    credentials: bool,
    home: &Path,
) -> Result<String, String> {
    if !valid_id(id) {
        return Err("bad id".into());
    }
    if !["cifs", "nfs", "nfs4"].contains(&fstype) {
        return Err(format!("“{fstype}” isn't a network share type"));
    }
    let ok_source = match fstype {
        "cifs" => {
            source.strip_prefix("//").and_then(|r| r.split_once('/')).is_some_and(|(h, s)| valid_host(h) && valid_path_text(s))
        }
        _ => source.split_once(":/").is_some_and(|(h, p)| valid_host(h) && (p.is_empty() || valid_path_text(p))),
    };
    if !ok_source {
        return Err(format!("“{source}” isn't a share Settings can add"));
    }
    if !valid_dir(dir, home) {
        return Err(format!("A mount folder has to be inside /mnt, /media or your home folder, not “{dir}”"));
    }
    if let Some(bad) = options.iter().find(|o| !valid_option(o)) {
        return Err(format!("“{bad}” isn't an option Settings sets"));
    }
    let mut opts: Vec<String> = options.to_vec();
    if credentials {
        opts.push(format!("credentials={CREDENTIALS}/{id}"));
    }
    if opts.is_empty() {
        opts.push("defaults".into());
    }
    Ok(format!("{} {} {} {} 0 0 {MARK}{id}", escape(source), escape(dir), fstype, opts.join(",")))
}

/// The SMB credentials file: user and password (no newlines in either).
pub fn credentials_text(user: &str, password: &str, domain: &str) -> Result<String, String> {
    if [user, password, domain].iter().any(|s| s.contains('\n') || s.contains('\r')) {
        return Err("A user name or password can't contain a line break".into());
    }
    let mut t = format!("username={user}\npassword={password}\n");
    if !domain.is_empty() {
        t.push_str(&format!("domain={domain}\n"));
    }
    Ok(t)
}

/// fstab without Settings' line for `id`.
pub fn without(text: &str, id: &str) -> String {
    let mark = format!("{MARK}{id}");
    let mut out: String = text.lines().filter(|l| !l.trim_end().ends_with(&mark)).collect::<Vec<_>>().join("\n");
    out.push('\n');
    out
}

/// A short id from a name: "Photos on NAS" → "photos-on-nas".
pub fn slug(name: &str) -> String {
    let mut s: String = name.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    while s.contains("--") {
        s = s.replace("--", "-");
    }
    let s = s.trim_matches('-').chars().take(32).collect::<String>();
    if s.is_empty() { "share".into() } else { s }
}

// ---------- SSH folders: user services ----------

pub fn ssh_unit_dir(home: &Path) -> PathBuf {
    home.join(".config/systemd/user")
}

pub fn ssh_unit_name(id: &str) -> String {
    format!("nyx-sshfs-{id}.service")
}

/// Quote one argument for a systemd `ExecStart=` line.
fn sd_quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"").replace('%', "%%"))
}

/// The user service that keeps an SSH folder mounted.
pub fn ssh_unit(name: &str, user: &str, host: &str, remote: &str, dir: &str) -> Result<String, String> {
    if !valid_user(user) || !valid_host(host) {
        return Err("Check the user and computer name.".into());
    }
    if remote.chars().any(|c| c.is_control()) || dir.chars().any(|c| c.is_control()) {
        return Err("A folder name can't contain control characters.".into());
    }
    let target = format!("{user}@{host}:{remote}");
    Ok(format!(
        "[Unit]\n\
         Description=SSH folder {} (Settings)\n\
         After=network-online.target\n\
         Wants=network-online.target\n\n\
         [Service]\n\
         ExecStartPre=/usr/bin/mkdir -p {}\n\
         ExecStart=/usr/bin/sshfs -f -o reconnect,ServerAliveInterval=15,ServerAliveCountMax=3,BatchMode=yes,idmap=user {} {}\n\
         ExecStop=/usr/bin/fusermount3 -u {}\n\
         Restart=on-failure\n\
         RestartSec=15\n\n\
         [Install]\n\
         WantedBy=default.target\n",
        name.replace('\n', " "),
        sd_quote(dir),
        sd_quote(&target),
        sd_quote(dir),
        sd_quote(dir),
    ))
}

/// SSH folders Settings set up: (id, user@host:path, local folder).
pub fn ssh_folders(home: &Path) -> Vec<(String, String, String)> {
    let Ok(rd) = std::fs::read_dir(ssh_unit_dir(home)) else { return Vec::new() };
    let mut out: Vec<(String, String, String)> = rd
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let id = name.strip_prefix("nyx-sshfs-")?.strip_suffix(".service")?.to_string();
            let text = std::fs::read_to_string(e.path()).ok()?;
            let exec = text.lines().find(|l| l.starts_with("ExecStart="))?;
            let (_, args) = exec.split_once(",idmap=user ")?;
            let args: Vec<String> = args
                .split('"')
                .skip(1)
                .step_by(2)
                .map(|a| a.replace("\\\"", "\"").replace("\\\\", "\\").replace("%%", "%"))
                .collect();
            Some((id, args.first()?.clone(), args.get(1)?.clone()))
        })
        .collect();
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME: &str = "/home/ken";

    #[test]
    fn reads_fstab() {
        let t = "# comment\nUUID=abc / btrfs rw,subvol=/@ 0 0\nUUID=b /boot vfat defaults 0 2\n\
                 //nas/Photos\\040Old /mnt/photos cifs credentials=/x,nofail 0 0 # nyx-settings:photos\n\
                 nas:/export /mnt/nfs nfs defaults 0 0\n";
        let m = parse_fstab(t);
        assert_eq!(m.len(), 4);
        assert!(m[0].system && m[1].system);
        assert_eq!(m[2].source, "//nas/Photos Old");
        assert_eq!(m[2].id, "photos");
        assert_eq!(m[2].kind, Kind::Smb);
        assert!(m[2].ours() && !m[3].ours());
        assert_eq!(m[3].kind, Kind::Nfs);
    }

    #[test]
    fn builds_network_lines() {
        let home = Path::new(HOME);
        let src = network_source("cifs", "nas.local", "Photos Old").unwrap();
        let opts: Vec<String> =
            ["nofail", "_netdev", "x-systemd.automount", "x-systemd.idle-timeout=10min", "vers=3.0", "uid=1000"]
                .iter()
                .map(|s| s.to_string())
                .collect();
        let line = network_line("photos", "cifs", &src, "/mnt/photos old", &opts, true, home).unwrap();
        assert_eq!(
            line,
            "//nas.local/Photos\\040Old /mnt/photos\\040old cifs nofail,_netdev,x-systemd.automount,x-systemd.idle-timeout=10min,vers=3.0,uid=1000,credentials=/etc/nyx-settings/credentials/photos 0 0 # nyx-settings:photos"
        );
        let parsed = &parse_fstab(&line)[0];
        assert_eq!(parsed.dir, "/mnt/photos old");
        assert_eq!(parsed.id, "photos");
        let nfs = network_source("nfs", "10.0.0.2", "/srv/media").unwrap();
        assert_eq!(nfs, "10.0.0.2:/srv/media");
        assert!(network_line("m", "nfs", &nfs, "/home/ken/Media", &[], false, home).is_ok());
    }

    #[test]
    fn refuses_what_it_shouldnt_write() {
        let home = Path::new(HOME);
        let src = "//nas/x".to_string();
        assert!(network_line("x", "cifs", &src, "/etc/cron.d", &[], false, home).is_err());
        assert!(network_line("x", "cifs", &src, "/mnt", &[], false, home).is_err());
        assert!(network_line("x", "cifs", &src, "/mnt/../etc", &[], false, home).is_err());
        assert!(network_line("x", "cifs", &src, "/home/other/x", &[], false, home).is_err());
        assert!(network_line("x", "cifs", &src, "/mnt/a\nb", &[], false, home).is_err());
        assert!(network_line("x", "ext4", &src, "/mnt/a", &[], false, home).is_err());
        assert!(network_line("x y", "cifs", &src, "/mnt/a", &[], false, home).is_err());
        assert!(network_line("x", "cifs", &src, "/mnt/a", &["credentials=/etc/shadow".into()], false, home).is_err());
        assert!(network_line("x", "cifs", &src, "/mnt/a", &["exec,suid".into()], false, home).is_err());
        assert!(network_line("x", "cifs", "//-oevil/x", "/mnt/a", &[], false, home).is_err());
        assert!(network_source("cifs", "nas", "a\"b").is_none());
        assert!(network_source("cifs", "nas;rm", "x").is_none());
        assert!(credentials_text("me", "pa\nss", "").is_err());
        assert_eq!(credentials_text("me", "p w", "").unwrap(), "username=me\npassword=p w\n");
    }

    #[test]
    fn removes_only_its_own_line() {
        let t = "a b c d 0 0\nx y cifs o 0 0 # nyx-settings:one\nq r nfs o 0 0 # nyx-settings:onetwo\n";
        assert_eq!(without(t, "one"), "a b c d 0 0\nq r nfs o 0 0 # nyx-settings:onetwo\n");
    }

    #[test]
    fn ssh_units_round_trip() {
        let u = ssh_unit("Server", "ken", "box.lan", "/srv/files", "/home/ken/Remote/Server box").unwrap();
        assert!(u.contains("ExecStart=/usr/bin/sshfs -f -o reconnect,ServerAliveInterval=15,ServerAliveCountMax=3,BatchMode=yes,idmap=user \"ken@box.lan:/srv/files\" \"/home/ken/Remote/Server box\""));
        assert!(ssh_unit("x", "-oProxyCommand=evil", "h", "/", "/home/ken/x").is_err());
        let dir = std::env::temp_dir().join(format!("nyx-ssh-test-{}", std::process::id()));
        let units = ssh_unit_dir(&dir);
        std::fs::create_dir_all(&units).unwrap();
        let path = units.join(ssh_unit_name("server"));
        let expected = vec![("server".into(), "ken@box.lan:/srv/files".into(), "/home/ken/Remote/Server box".into())];
        std::fs::write(&path, &u).unwrap();
        assert_eq!(ssh_folders(&dir), expected);
        let migrated = u.replace("ExecStart=/usr/bin/sshfs ", "ExecStart=\"/home/ken/.nix-profile/bin/sshfs\" ");
        std::fs::write(&path, migrated).unwrap();
        assert_eq!(ssh_folders(&dir), expected);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn builds_drive_lines() {
        let home = Path::new(HOME);
        let opts: Vec<String> =
            ["nofail", "x-gvfs-show", "x-udisks-auth", "uid=1000", "umask=022"].iter().map(|s| s.to_string()).collect();
        assert_eq!(
            drive_line("photos", "B2C4-1F00", "/mnt/photos", "exfat", &opts, home).unwrap(),
            "UUID=B2C4-1F00 /mnt/photos exfat nofail,x-gvfs-show,x-udisks-auth,uid=1000,umask=022 0 0 # nyx-settings:photos"
        );
        assert!(drive_line("x", "abc;rm", "/mnt/x", "ext4", &[], home).is_err());
        assert!(drive_line("x", "abcd", "/mnt/x", "ext4 --x", &[], home).is_err());
        assert!(drive_line("x", "abcd", "/etc", "ext4", &[], home).is_err());
        assert!(drive_line("x", "abcd", "/mnt/x", "ext4", &["exec".into()], home).is_err());
        assert!(drive_line("x", "abcd", "/mnt/x", "ext4", &[], home).unwrap().ends_with(" 0 2 # nyx-settings:x"));
    }

    #[test]
    fn password_free_rule_covers_only_disks_and_mounts() {
        let r = password_free_rule("ken", "/usr/local/lib/nyx-settings/nyx-settings-helper");
        assert!(r.contains("subject.user != \"ken\""));
        assert!(r.contains("\"mount-add-net\", \"mount-add-drive\""));
        assert!(!r.contains("add-user") && !r.contains("set-password") && !r.contains("firewall"));
        assert!(r.contains("subject.local") && r.contains("subject.active"));
    }

    #[test]
    fn slugs() {
        assert_eq!(slug("Photos on NAS!"), "photos-on-nas");
        assert_eq!(slug("   "), "share");
    }
}
