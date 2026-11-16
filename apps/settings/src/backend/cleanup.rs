//! What can be cleaned up, and how much each would free. Reading only: the
//! cleaning itself runs through the root helper (or in the user's own folders).

use std::path::Path;
use std::process::Command;

/// Bytes in a size like "7.75 GiB", "586.6M" or "12 KiB".
pub fn parse_size(text: &str) -> Option<u64> {
    let t = text.trim();
    let split = t.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(t.len());
    let n: f64 = t[..split].parse().ok()?;
    let unit = t[split..].trim().trim_end_matches("iB").trim_end_matches('B').to_ascii_uppercase();
    let k: f64 = match unit.as_str() {
        "" => 1.0,
        "K" => 1024.0,
        "M" => 1024.0 * 1024.0,
        "G" => 1024.0 * 1024.0 * 1024.0,
        "T" => 1024f64.powi(4),
        _ => return None,
    };
    Some((n * k) as u64)
}

/// "==> finished dry run: 239 candidates (disk space saved: 7.75 GiB)".
pub fn paccache_saved(out: &str) -> u64 {
    out.lines()
        .rev()
        .find_map(|l| l.split("disk space saved:").nth(1).and_then(|r| parse_size(r.trim().trim_end_matches(')'))))
        .unwrap_or(0)
}

fn output(program: &str, args: &[&str]) -> String {
    Command::new(program).args(args).output().map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default()
}

/// Old package versions (all but the newest) and packages no longer installed.
pub fn package_cache() -> u64 {
    if !Path::new("/usr/bin/paccache").exists() {
        return 0;
    }
    paccache_saved(&output("/usr/bin/paccache", &["-dk1"])) + paccache_saved(&output("/usr/bin/paccache", &["-duk0"]))
}

/// Packages installed as dependencies that nothing needs any more, and their size.
pub fn orphans() -> (Vec<String>, u64) {
    let names: Vec<String> =
        output("/usr/bin/pacman", &["-Qdtq"]).lines().map(str::to_string).filter(|l| !l.is_empty()).collect();
    if names.is_empty() {
        return (names, 0);
    }
    let mut args = vec!["-Qi"];
    args.extend(names.iter().map(String::as_str));
    let size = output("/usr/bin/pacman", &args)
        .lines()
        .filter_map(|l| l.strip_prefix("Installed Size").and_then(|r| r.split(':').nth(1)).and_then(parse_size))
        .sum();
    (names, size)
}

/// "Archived and active journals take up 586.6M in the file system."
pub fn journal_usage(out: &str) -> u64 {
    out.split("take up").nth(1).and_then(|r| r.split_whitespace().next()).and_then(parse_size).unwrap_or(0)
}

/// System logs over `keep` bytes.
pub fn journal_extra(keep: u64) -> u64 {
    journal_usage(&output("journalctl", &["--disk-usage"])).saturating_sub(keep)
}

/// Bytes under a folder (not following other filesystems).
pub fn folder_size(path: &Path) -> u64 {
    if !path.exists() {
        return 0;
    }
    output("du", &["-sbx", &path.to_string_lossy()]).split_whitespace().next().and_then(|n| n.parse().ok()).unwrap_or(0)
}

/// The biggest folders directly in `home`, largest first.
pub fn largest(home: &Path, count: usize) -> Vec<(String, u64)> {
    let out = Command::new("nice")
        .args(["-n", "15", "du", "-xb", "-d", "1", &home.to_string_lossy()])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();
    parse_du(&out, &home.to_string_lossy(), count)
}

pub fn parse_du(out: &str, root: &str, count: usize) -> Vec<(String, u64)> {
    let root = root.trim_end_matches('/');
    let mut v: Vec<(String, u64)> = out
        .lines()
        .filter_map(|l| {
            let (size, path) = l.split_once('\t')?;
            (path.trim_end_matches('/') != root).then(|| Some((path.to_string(), size.trim().parse().ok()?)))?
        })
        .collect();
    v.sort_by_key(|e| std::cmp::Reverse(e.1));
    v.truncate(count);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(parse_size("12"), Some(12));
        assert_eq!(parse_size("1 KiB"), Some(1024));
        assert_eq!(parse_size("586.6M"), Some((586.6 * 1048576.0) as u64));
        assert_eq!(parse_size("7.75 GiB"), Some((7.75 * 1073741824.0) as u64));
        assert_eq!(parse_size("lots"), None);
    }

    #[test]
    fn tool_output() {
        assert_eq!(paccache_saved("==> finished dry run: 239 candidates (disk space saved: 2 GiB)\n"), 2 << 30);
        assert_eq!(paccache_saved("==> no candidate packages found for pruning\n"), 0);
        assert_eq!(journal_usage("Archived and active journals take up 1.0G in the file system."), 1 << 30);
        let du = "100\t/home/k/a\n300\t/home/k/b c\n50\t/home/k/.cache\n450\t/home/k\n";
        assert_eq!(parse_du(du, "/home/k/", 2), vec![("/home/k/b c".to_string(), 300), ("/home/k/a".to_string(), 100)]);
    }
}
