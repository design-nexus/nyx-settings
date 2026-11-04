//! Reading `/etc/passwd` and `/etc/group`, and the rules for names. Std only:
//! the root helper includes this file too, so keep it free of other crates.

#![allow(dead_code)]

/// Accounts people log in with. Below this are system users; 65534 is `nobody`.
pub const FIRST_UID: u32 = 1000;
pub const LAST_UID: u32 = 60000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct User {
    pub name: String,
    pub uid: u32,
    pub gid: u32,
    pub full_name: String,
    pub home: String,
    pub shell: String,
}

impl User {
    /// A person's account: a uid in people's range and a shell to sign in with (NixOS's
    /// build users have uids there too, but no shell).
    pub fn is_human(&self) -> bool {
        (FIRST_UID..LAST_UID).contains(&self.uid) && !self.shell.ends_with("/nologin") && !self.shell.ends_with("/false")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub name: String,
    pub gid: u32,
    pub members: Vec<String>,
}

impl Group {
    pub fn is_system(&self) -> bool {
        self.gid < FIRST_UID || self.gid >= LAST_UID
    }
}

pub fn parse_passwd(text: &str) -> Vec<User> {
    text.lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split(':').collect();
            if f.len() < 7 {
                return None;
            }
            Some(User {
                name: f[0].to_string(),
                uid: f[2].parse().ok()?,
                gid: f[3].parse().ok()?,
                full_name: f[4].split(',').next().unwrap_or("").to_string(),
                home: f[5].to_string(),
                shell: f[6].to_string(),
            })
        })
        .collect()
}

pub fn parse_group(text: &str) -> Vec<Group> {
    text.lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split(':').collect();
            if f.len() < 4 {
                return None;
            }
            Some(Group {
                name: f[0].to_string(),
                gid: f[2].parse().ok()?,
                members: f[3].split(',').filter(|m| !m.is_empty()).map(str::to_string).collect(),
            })
        })
        .collect()
}

/// Login shells listed in `/etc/shells`.
pub fn parse_shells(text: &str) -> Vec<String> {
    text.lines().map(str::trim).filter(|l| l.starts_with('/')).map(str::to_string).collect()
}

pub fn users() -> Vec<User> {
    parse_passwd(&std::fs::read_to_string("/etc/passwd").unwrap_or_default())
}

pub fn groups() -> Vec<Group> {
    parse_group(&std::fs::read_to_string("/etc/group").unwrap_or_default())
}

pub fn shells() -> Vec<String> {
    parse_shells(&std::fs::read_to_string("/etc/shells").unwrap_or_default())
}

/// A name useradd and groupadd would accept: lower case, digits, `_` and `-`,
/// not starting with a digit or `-`, at most 32 characters.
pub fn valid_name(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase() || c == '_')
        && s.len() <= 32
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASSWD: &str = "root:x:0:0:root:/root:/bin/bash\nken:x:1000:1000:Ken Smith,,,:/home/ken:/bin/bash\nnobody:x:65534:65534:Kernel Overflow User:/:/usr/bin/nologin\nbroken line\n";
    const GROUP: &str = "wheel:x:998:ken,sam\ndocker:x:970:\nken:x:1000:\n";

    #[test]
    fn parses_passwd() {
        let u = parse_passwd(PASSWD);
        assert_eq!(u.len(), 3);
        assert_eq!(u[1].full_name, "Ken Smith");
        assert!(u[1].is_human());
        assert!(!u[0].is_human());
        assert!(!u[2].is_human());
    }

    #[test]
    fn parses_group() {
        let g = parse_group(GROUP);
        assert_eq!(g[0].members, ["ken", "sam"]);
        assert!(g[1].members.is_empty());
        assert!(g[0].is_system());
        assert!(!g[2].is_system());
    }

    #[test]
    fn parses_shells() {
        assert_eq!(super::parse_shells("# comment\n/bin/bash\n\n/usr/bin/zsh\n"), ["/bin/bash", "/usr/bin/zsh"]);
    }

    #[test]
    fn names() {
        for ok in ["ken", "_x", "a-b_c9", "a".repeat(32).as_str()] {
            assert!(valid_name(ok), "{ok}");
        }
        for bad in ["", "Ken", "9a", "-a", "a b", "a:b", "a/b", "a\nb", "ä", "a".repeat(33).as_str(), "root;ls"] {
            assert!(!valid_name(bad), "{bad:?}");
        }
    }
}
