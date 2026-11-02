//! What this computer is and how busy it is, read from `/proc` and `/sys`.
//! No GTK. Parsers take text so they can be tested without the files.

use std::path::Path;

fn read(path: impl AsRef<Path>) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

fn read_trim(path: impl AsRef<Path>) -> String {
    read(path).trim().to_string()
}

// ---------- Identity ----------

pub fn hostname() -> String {
    read_trim("/proc/sys/kernel/hostname")
}

pub fn cpu_model() -> String {
    read("/proc/cpuinfo")
        .lines()
        .find(|l| l.starts_with("model name"))
        .and_then(|l| l.split_once(':'))
        .map(|(_, v)| tidy_cpu(v))
        .unwrap_or_default()
}

/// "Intel(R) Core(TM) Ultra 9 285H CPU @ 2.90GHz" → "Intel Core Ultra 9 285H".
pub fn tidy_cpu(name: &str) -> String {
    let name = name.split(" @ ").next().unwrap_or(name);
    let name = name.replace("(R)", "").replace("(TM)", "").replace("(tm)", "");
    let words: Vec<&str> = name
        .split_whitespace()
        .filter(|w| !matches!(*w, "CPU" | "Processor"))
        // AMD's "8-Core Processor" says nothing the core count doesn't.
        .filter(|w| !w.ends_with("-Core"))
        .collect();
    words.join(" ")
}

pub fn model() -> String {
    let dmi = |f: &str| read_trim(format!("/sys/class/dmi/id/{f}"));
    tidy_model(&dmi("sys_vendor"), &dmi("product_name"))
}

/// A readable maker and model: short maker names, no repeated model codes.
/// ("ASUSTeK COMPUTER INC.", "ROG Zephyrus G16 GU605CW_GU605CW") → "ASUS ROG Zephyrus G16 GU605CW".
pub fn tidy_model(vendor: &str, product: &str) -> String {
    let short = [
        ("asustek", "ASUS"),
        ("hewlett", "HP"),
        ("lenovo", "Lenovo"),
        ("dell", "Dell"),
        ("micro-star", "MSI"),
        ("gigabyte", "Gigabyte"),
        ("acer", "Acer"),
        ("framework", "Framework"),
        ("apple", "Apple"),
        ("samsung", "Samsung"),
        ("microsoft", "Microsoft"),
        ("system76", "System76"),
        ("tuxedo", "TUXEDO"),
    ];
    let lower = vendor.to_lowercase();
    let vendor = short.iter().find(|(k, _)| lower.starts_with(k)).map(|(_, v)| v.to_string()).unwrap_or_else(|| {
        // Drop company suffixes.
        vendor
            .split_whitespace()
            .filter(|w| {
                !matches!(
                    w.trim_end_matches(['.', ',']).to_lowercase().as_str(),
                    "inc" | "corp" | "corporation" | "co" | "ltd" | "computer" | "llc" | "gmbh"
                )
            })
            .collect::<Vec<_>>()
            .join(" ")
    });
    // "GU605CW_GU605CW": a code repeated after an underscore.
    let product: Vec<String> = product
        .split_whitespace()
        .map(|w| match w.split_once('_') {
            Some((a, b)) if a == b => a.to_string(),
            _ => w.to_string(),
        })
        .collect();
    let mut product = product.join(" ");
    // Some makers repeat their name in the product.
    if product.to_lowercase().starts_with(&vendor.to_lowercase()) {
        product = product[vendor.len()..].trim().to_string();
    }
    // Placeholder strings some boards ship with.
    let junk = |s: &str| {
        s.is_empty() || ["to be filled by o.e.m.", "system product name", "default string"].contains(&s.to_lowercase().as_str())
    };
    match (junk(&vendor), junk(&product)) {
        (true, true) => String::new(),
        (true, false) => product,
        (false, true) => vendor,
        (false, false) => format!("{vendor} {product}"),
    }
}

pub fn uptime_secs() -> u64 {
    read("/proc/uptime").split_whitespace().next().and_then(|s| s.parse::<f64>().ok()).map(|s| s as u64).unwrap_or(0)
}

/// "45 min", "3 h 12 min", "2 days 4 h".
pub fn duration_text(secs: u64) -> String {
    let (d, h, m) = (secs / 86_400, secs % 86_400 / 3600, secs % 3600 / 60);
    match (d, h) {
        (0, 0) => format!("{m} min"),
        (0, _) => format!("{h} h {m} min"),
        (1, _) => format!("1 day {h} h"),
        _ => format!("{d} days {h} h"),
    }
}

/// "1.4 GB", "512 MB", "8.0 TB".
pub fn bytes_text(b: u64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1000.0 && i < units.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i >= 3 { format!("{v:.1} {}", units[i]) } else { format!("{v:.0} {}", units[i]) }
}

/// Throughput, e.g. "1.2 MB/s".
pub fn rate_text(bytes_per_sec: f64) -> String {
    format!("{}/s", bytes_text(bytes_per_sec.max(0.0) as u64))
}

// ---------- CPU ----------

/// Busy and total jiffies for one `cpu` line of `/proc/stat`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CpuTimes {
    pub busy: u64,
    pub total: u64,
}

impl CpuTimes {
    /// Fraction busy between `prev` and `self`.
    pub fn usage_since(&self, prev: &CpuTimes) -> f64 {
        let total = self.total.saturating_sub(prev.total);
        if total == 0 { 0.0 } else { self.busy.saturating_sub(prev.busy) as f64 / total as f64 }
    }
}

/// The overall line first, then one per core.
pub fn parse_stat(text: &str) -> Vec<CpuTimes> {
    text.lines()
        .filter(|l| l.starts_with("cpu"))
        .map(|l| {
            let f: Vec<u64> = l.split_whitespace().skip(1).take(8).filter_map(|v| v.parse().ok()).collect();
            let total: u64 = f.iter().sum();
            let idle = f.get(3).copied().unwrap_or(0) + f.get(4).copied().unwrap_or(0);
            CpuTimes { busy: total.saturating_sub(idle), total }
        })
        .collect()
}

pub fn cpu_times() -> Vec<CpuTimes> {
    parse_stat(&read("/proc/stat"))
}

// ---------- Memory ----------

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Memory {
    pub total: u64,
    pub used: u64,
    pub swap_total: u64,
    pub swap_used: u64,
}

pub fn parse_meminfo(text: &str) -> Memory {
    let kb = |key: &str| -> u64 {
        text.lines()
            .find(|l| l.split(':').next() == Some(key))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0)
            * 1024
    };
    let total = kb("MemTotal");
    let swap_total = kb("SwapTotal");
    Memory {
        total,
        used: total.saturating_sub(kb("MemAvailable")),
        swap_total,
        swap_used: swap_total.saturating_sub(kb("SwapFree")),
    }
}

pub fn memory() -> Memory {
    parse_meminfo(&read("/proc/meminfo"))
}

// ---------- Network ----------

/// Bytes received and sent by real interfaces (not loopback, containers or bridges).
pub fn parse_net_dev(text: &str) -> (u64, u64) {
    let virtual_if = |n: &str| n == "lo" || ["veth", "docker", "br-", "virbr", "vnet"].iter().any(|p| n.starts_with(p));
    text.lines()
        .filter_map(|l| l.split_once(':'))
        .filter(|(name, _)| !virtual_if(name.trim()))
        .filter_map(|(_, rest)| {
            let f: Vec<u64> = rest.split_whitespace().filter_map(|v| v.parse().ok()).collect();
            Some((*f.first()?, *f.get(8)?))
        })
        .fold((0, 0), |(r, t), (a, b)| (r + a, t + b))
}

pub fn net_bytes() -> (u64, u64) {
    parse_net_dev(&read("/proc/net/dev"))
}

// ---------- Sensors ----------

/// The CPU package temperature in °C, from the CPU's own sensor if there is one.
pub fn cpu_temp() -> Option<f64> {
    let mut found: Vec<(String, std::path::PathBuf)> =
        std::fs::read_dir("/sys/class/hwmon").ok()?.flatten().map(|e| (read_trim(e.path().join("name")), e.path())).collect();
    let rank = |n: &str| ["coretemp", "k10temp", "zenpower", "cpu_thermal", "acpitz"].iter().position(|c| *c == n);
    found.retain(|(n, _)| rank(n).is_some());
    found.sort_by_key(|(n, _)| rank(n));
    let (_, dir) = found.first()?;
    let milli: f64 = read_trim(dir.join("temp1_input")).parse().ok()?;
    Some(milli / 1000.0)
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Battery {
    pub percent: u32,
    pub status: String,
    /// Full charge now against when new, as a percentage (100 = like new).
    pub health: Option<u32>,
    pub cycles: Option<u32>,
}

/// A battery's `uevent` file.
pub fn parse_battery(uevent: &str) -> Option<Battery> {
    let get = |k: &str| uevent.lines().find_map(|l| l.strip_prefix(&format!("POWER_SUPPLY_{k}="))).map(str::trim);
    let num = |k: &str| get(k).and_then(|v| v.parse::<u64>().ok());
    if get("TYPE")? != "Battery" || get("PRESENT") == Some("0") {
        return None;
    }
    let health = match (num("ENERGY_FULL").or(num("CHARGE_FULL")), num("ENERGY_FULL_DESIGN").or(num("CHARGE_FULL_DESIGN"))) {
        (Some(full), Some(design)) if design > 0 => Some(((full * 100 / design) as u32).min(100)),
        _ => None,
    };
    Some(Battery {
        percent: num("CAPACITY")? as u32,
        status: get("STATUS").unwrap_or_default().to_string(),
        health,
        cycles: num("CYCLE_COUNT").filter(|c| *c > 0).map(|c| c as u32),
    })
}

pub fn battery() -> Option<Battery> {
    std::fs::read_dir("/sys/class/power_supply").ok()?.flatten().find_map(|e| parse_battery(&read(e.path().join("uevent"))))
}

// ---------- Storage ----------

#[derive(Debug, Clone, PartialEq)]
pub struct Disk {
    pub mount: String,
    pub size: u64,
    pub used: u64,
}

/// `df -l -B1 --output=source,fstype,target,size,used`, one entry per real device
/// (btrfs subvolumes of the same device are shown once, at the shortest mount).
pub fn parse_df(text: &str) -> Vec<Disk> {
    let mut out: Vec<(String, Disk)> = Vec::new();
    for line in text.lines().skip(1) {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 5 || !f[0].starts_with("/dev/") || f[1].starts_with("fuse") {
            continue;
        }
        let (Ok(size), Ok(used)) = (f[f.len() - 2].parse(), f[f.len() - 1].parse()) else { continue };
        let disk = Disk { mount: f[2..f.len() - 2].join(" "), size, used };
        match out.iter_mut().find(|(src, _)| src == f[0]) {
            Some((_, d)) if disk.mount.len() < d.mount.len() => *d = disk,
            Some(_) => {}
            None => out.push((f[0].to_string(), disk)),
        }
    }
    out.into_iter().map(|(_, d)| d).filter(|d| d.size > 0).collect()
}

pub fn disks() -> Vec<Disk> {
    crate::cmd::output(&["df", "-l", "-B1", "--output=source,fstype,target,size,used"]).map(|t| parse_df(&t)).unwrap_or_default()
}

// ---------- Packages ----------

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Packages {
    pub total: usize,
    /// Installed from the AUR or by hand (not in a repository).
    pub foreign: usize,
    /// Asked for by name (not pulled in as a dependency).
    pub explicit: usize,
}

pub fn packages() -> Packages {
    let count = |flag: &str| crate::cmd::output(&["pacman", flag]).map(|t| t.lines().count()).unwrap_or(0);
    Packages { total: count("-Qq"), foreign: count("-Qqm"), explicit: count("-Qqe") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tidy_names() {
        assert_eq!(tidy_cpu(" Intel(R) Core(TM) Ultra 9 285H"), "Intel Core Ultra 9 285H");
        assert_eq!(tidy_cpu("Intel(R) Core(TM) i7-8650U CPU @ 1.90GHz"), "Intel Core i7-8650U");
        assert_eq!(tidy_cpu("AMD Ryzen 7 7840U w/ Radeon 780M Graphics"), "AMD Ryzen 7 7840U w/ Radeon 780M Graphics");
        assert_eq!(tidy_cpu("AMD Ryzen 9 5900X 12-Core Processor"), "AMD Ryzen 9 5900X");
        assert_eq!(tidy_model("ASUSTeK COMPUTER INC.", "ROG Zephyrus G16 GU605CW_GU605CW"), "ASUS ROG Zephyrus G16 GU605CW");
        assert_eq!(tidy_model("LENOVO", "21K5"), "Lenovo 21K5");
        assert_eq!(tidy_model("Framework", "Laptop 13 (AMD Ryzen 7040Series)"), "Framework Laptop 13 (AMD Ryzen 7040Series)");
        assert_eq!(tidy_model("Example Corp.", "Example Box"), "Example Box");
        assert_eq!(tidy_model("To Be Filled By O.E.M.", "To Be Filled By O.E.M."), "");
    }

    #[test]
    fn formats_sizes_and_durations() {
        assert_eq!(bytes_text(512), "512 B");
        assert_eq!(bytes_text(300 * 1024 * 1024), "300 MB");
        assert_eq!(bytes_text(16 * 1024 * 1024 * 1024), "16.0 GB");
        assert_eq!(duration_text(59 * 60), "59 min");
        assert_eq!(duration_text(3 * 3600 + 12 * 60), "3 h 12 min");
        assert_eq!(duration_text(86_400 + 4 * 3600), "1 day 4 h");
        assert_eq!(duration_text(2 * 86_400), "2 days 0 h");
    }

    #[test]
    fn cpu_usage_from_stat() {
        let a = parse_stat("cpu  100 0 100 800 0 0 0 0 0 0\ncpu0 50 0 50 400 0 0 0 0 0 0\nintr 1 2\n");
        let b = parse_stat("cpu  200 0 200 900 100 0 0 0 0 0\ncpu0 50 0 50 500 0 0 0 0 0 0\n");
        assert_eq!(a.len(), 2);
        assert_eq!(a[0], CpuTimes { busy: 200, total: 1000 });
        assert!((b[0].usage_since(&a[0]) - 0.5).abs() < 1e-9);
        assert_eq!(b[1].usage_since(&a[1]), 0.0);
        assert_eq!(a[0].usage_since(&a[0]), 0.0);
    }

    #[test]
    fn memory_from_meminfo() {
        let m = parse_meminfo("MemTotal: 1000 kB\nMemFree: 100 kB\nMemAvailable: 400 kB\nSwapTotal: 200 kB\nSwapFree: 150 kB\n");
        assert_eq!(m, Memory { total: 1000 * 1024, used: 600 * 1024, swap_total: 200 * 1024, swap_used: 50 * 1024 });
    }

    #[test]
    fn network_skips_virtual_interfaces() {
        let t = "Inter-|   Receive |  Transmit\n face |bytes packets|bytes\n    lo: 9 1 0 0 0 0 0 0 9 1 0 0 0 0 0 0\n  wlo1: 100 5 0 0 0 0 0 0 40 2 0 0 0 0 0 0\ndocker0: 7 1 0 0 0 0 0 0 7 1 0 0 0 0 0 0\n  eth0: 1 1 0 0 0 0 0 0 2 1 0 0 0 0 0 0\n";
        assert_eq!(parse_net_dev(t), (101, 42));
    }

    #[test]
    fn battery_from_uevent() {
        let b = parse_battery(
            "POWER_SUPPLY_TYPE=Battery\nPOWER_SUPPLY_STATUS=Discharging\nPOWER_SUPPLY_PRESENT=1\nPOWER_SUPPLY_CYCLE_COUNT=0\nPOWER_SUPPLY_CHARGE_FULL_DESIGN=5000\nPOWER_SUPPLY_CHARGE_FULL=4500\nPOWER_SUPPLY_CAPACITY=81\n",
        )
        .unwrap();
        assert_eq!(b, Battery { percent: 81, status: "Discharging".into(), health: Some(90), cycles: None });
        assert_eq!(parse_battery("POWER_SUPPLY_TYPE=Mains\nPOWER_SUPPLY_ONLINE=1\n"), None);
    }

    #[test]
    fn disks_from_df() {
        let t = "Filesystem Type Mounted on 1B-blocks Used\n\
                 dev devtmpfs /dev 100 0\n\
                 /dev/mapper/root btrfs /var/log 2000 500\n\
                 /dev/mapper/root btrfs / 2000 500\n\
                 /dev/nvme0n1p1 vfat /boot 100 60\n\
                 /dev/fuse fuse.rclone /home/x/Google Drive 5000 1\n\
                 /dev/sdb1 ext4 /mnt/My Disk 300 30\n";
        assert_eq!(
            parse_df(t),
            vec![
                Disk { mount: "/".into(), size: 2000, used: 500 },
                Disk { mount: "/boot".into(), size: 100, used: 60 },
                Disk { mount: "/mnt/My Disk".into(), size: 300, used: 30 },
            ]
        );
    }
}
