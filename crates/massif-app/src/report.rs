//! A report of a problem, made in one click (Help ▸ Report a Problem…): one
//! `.zip` the user saves where they like and sends themself, with what the
//! start wrote (`startup.log`, the one before, `graphics.txt`), the
//! computer and its graphics card, the settings that change how Massif
//! draws, the Developer panel's figures over the last minute, the map's
//! size, and the map itself only when asked. Nothing is sent anywhere.
//!
//! Every text that goes in has the user's home folder written `~`, and any
//! other account's folder `<user>`, so that names do not leak; the dialog
//! shows each file as it will be written before it is saved.
//!
//! A performance recording follows every frame between Start and Stop: its
//! time, the work in it part by part (the parts `startup::frame_step`
//! names), the memory, the processor and the tasks running. It goes in the
//! report as a table and as a trace for `chrome://tracing` or Perfetto.

use std::io::Write;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Private paths

/// `text` with the home folder `home` written `~`, and the folders of any
/// other account (`/Users/name`, `/home/name`, `C:\Users\name`) written
/// with `<user>` for the name.
pub fn scrub(text: &str, home: Option<&Path>) -> String {
    let mut out = text.to_owned();
    if let Some(home) = home.and_then(|h| h.to_str()).map(|h| h.trim_end_matches(['/', '\\'])).filter(|h| h.len() > 1) {
        // As written, and with the other kind of slash (a log may have
        // either on Windows).
        for form in [home.to_owned(), home.replace('\\', "/"), home.replace('/', "\\")] {
            out = replace_ci(&out, &form, "~", cfg!(windows));
        }
    }
    for root in ["/Users/", "/home/", "\\Users\\", "/users/", "\\users\\"] {
        out = hide_names_after(&out, root);
    }
    out
}

/// `text` with every `from` replaced by `to`, ignoring case if asked (paths
/// on Windows), where the match ends a path component.
fn replace_ci(text: &str, from: &str, to: &str, ignore_case: bool) -> String {
    if from.is_empty() {
        return text.to_owned();
    }
    let (hay, needle) = if ignore_case { (text.to_ascii_lowercase(), from.to_ascii_lowercase()) } else { (text.to_owned(), from.to_owned()) };
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    while let Some(i) = hay[at..].find(&needle) {
        let start = at + i;
        let end = start + needle.len();
        // `/Users/ann` is not the start of `/Users/anne`.
        let ends_component = text[end..].chars().next().is_none_or(|c| !(c.is_alphanumeric() || c == '_' || c == '-' || c == '.'));
        out.push_str(&text[at..start]);
        out.push_str(if ends_component { to } else { &text[start..end] });
        at = end;
    }
    out.push_str(&text[at..]);
    out
}

/// `text` with the name after each `root` (`/Users/`) written `<user>`;
/// `~` and `Shared`, `Public` left as they are.
fn hide_names_after(text: &str, root: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    while let Some(i) = text[at..].find(root) {
        let start = at + i + root.len();
        out.push_str(&text[at..start]);
        let name_end = text[start..].find(|c: char| c == '/' || c == '\\' || c.is_whitespace() || c == '"' || c == '\'' || c == ')' || c == ':').map_or(text.len(), |j| start + j);
        let name = &text[start..name_end];
        if name.is_empty() || matches!(name, "<user>" | "Shared" | "Public" | "Default" | "All Users") {
            out.push_str(name);
        } else {
            out.push_str("<user>");
        }
        at = name_end;
    }
    out.push_str(&text[at..]);
    out
}

// ---------------------------------------------------------------------------
// The computer

/// The computer Massif runs on, as a report says it.
#[derive(Debug, Clone, Default)]
pub struct System {
    pub os: String,
    pub cpu: String,
    pub threads: usize,
    pub ram: Option<u64>,
    pub gpu: Option<Gpu>,
    /// The window in points, and pixels a point.
    pub window: Option<([f32; 2], f32)>,
}

#[derive(Debug, Clone, Default)]
pub struct Gpu {
    pub name: String,
    pub backend: String,
    pub driver: String,
    pub kind: String,
}

impl Gpu {
    pub fn of(a: &wgpu::AdapterInfo) -> Gpu {
        let driver = [a.driver.as_str(), a.driver_info.as_str()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(" ");
        Gpu { name: a.name.clone(), backend: format!("{:?}", a.backend), driver, kind: format!("{:?}", a.device_type) }
    }
}

/// The version of Massif and the commit it was built from.
pub fn version() -> String {
    let commit = env!("MASSIF_COMMIT");
    if commit.is_empty() { crate::update::RUNNING.to_owned() } else { format!("{} ({commit})", crate::update::RUNNING) }
}

impl System {
    /// The computer, as far as it can be read here.
    pub fn read(gpu: Option<Gpu>, window: Option<([f32; 2], f32)>) -> System {
        System {
            os: format!("{} {}", os_name(), std::env::consts::ARCH),
            cpu: cpu_name().unwrap_or_else(|| "unknown".to_owned()),
            threads: std::thread::available_parallelism().map_or(1, |n| n.get()),
            ram: ram_bytes(),
            gpu,
            window,
        }
    }

    /// Name and value, a line each.
    pub fn lines(&self) -> Vec<(String, String)> {
        let mut v = vec![
            ("Massif".to_owned(), version()),
            ("System".to_owned(), self.os.clone()),
            ("Processor".to_owned(), format!("{}, {} threads", self.cpu, self.threads)),
            ("Memory".to_owned(), self.ram.map_or_else(|| "unknown".to_owned(), gb)),
        ];
        match &self.gpu {
            Some(g) => {
                v.push(("Graphics card".to_owned(), format!("{} ({})", g.name, g.kind)));
                v.push(("Graphics API".to_owned(), g.backend.clone()));
                v.push(("Driver".to_owned(), if g.driver.is_empty() { "not said".to_owned() } else { g.driver.clone() }));
            }
            None => v.push(("Graphics card".to_owned(), "not known yet".to_owned())),
        }
        if let Some(([w, h], ppp)) = self.window {
            v.push(("Window".to_owned(), format!("{w:.0} × {h:.0} points, scale {ppp:.2}")));
        }
        v
    }
}

fn gb(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / (1u64 << 30) as f64)
}

/// The system and its version: "macOS 26.2", "Windows 11 Pro (26100)",
/// "Ubuntu 24.04 LTS".
fn os_name() -> String {
    #[cfg(target_os = "macos")]
    {
        sysctl_string(c"kern.osproductversion").map_or_else(|| "macOS".to_owned(), |v| format!("macOS {v}"))
    }
    #[cfg(target_os = "linux")]
    {
        std::fs::read_to_string("/etc/os-release")
            .ok()
            .and_then(|s| s.lines().find_map(|l| l.strip_prefix("PRETTY_NAME=").map(|v| v.trim_matches('"').to_owned())))
            .unwrap_or_else(|| "Linux".to_owned())
    }
    #[cfg(windows)]
    {
        let key = "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion";
        let name = windows_registry_string(key, "ProductName").unwrap_or_else(|| "Windows".to_owned());
        let build = windows_registry_string(key, "CurrentBuildNumber").unwrap_or_default();
        // Windows 11 still says "Windows 10" in ProductName; its builds start
        // at 22000.
        let name = if build.parse::<u32>().is_ok_and(|b| b >= 22000) { name.replacen("Windows 10", "Windows 11", 1) } else { name };
        if build.is_empty() { name } else { format!("{name} ({build})") }
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
        std::env::consts::OS.to_owned()
    }
}

fn cpu_name() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        sysctl_string(c"machdep.cpu.brand_string")
    }
    #[cfg(target_os = "linux")]
    {
        let info = std::fs::read_to_string("/proc/cpuinfo").ok()?;
        info.lines().find_map(|l| l.strip_prefix("model name").and_then(|r| r.split_once(':')).map(|(_, v)| v.trim().to_owned()))
    }
    #[cfg(windows)]
    {
        windows_registry_string("HARDWARE\\DESCRIPTION\\System\\CentralProcessor\\0", "ProcessorNameString").map(|s| s.trim().to_owned())
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
        None
    }
}

pub(crate) fn ram_bytes() -> Option<u64> {
    #[cfg(target_os = "macos")]
    {
        let mut v: u64 = 0;
        let mut len = std::mem::size_of::<u64>();
        let ok = unsafe { libc::sysctlbyname(c"hw.memsize".as_ptr(), (&mut v as *mut u64).cast(), &mut len, std::ptr::null_mut(), 0) };
        (ok == 0).then_some(v)
    }
    #[cfg(target_os = "linux")]
    {
        let info = std::fs::read_to_string("/proc/meminfo").ok()?;
        let kb: u64 = info.lines().find_map(|l| l.strip_prefix("MemTotal:"))?.split_whitespace().next()?.parse().ok()?;
        Some(kb * 1024)
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
        let mut m: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
        m.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
        (unsafe { GlobalMemoryStatusEx(&mut m) } != 0).then_some(m.ullTotalPhys)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
        None
    }
}

#[cfg(target_os = "macos")]
fn sysctl_string(name: &std::ffi::CStr) -> Option<String> {
    let mut len = 0usize;
    if unsafe { libc::sysctlbyname(name.as_ptr(), std::ptr::null_mut(), &mut len, std::ptr::null_mut(), 0) } != 0 || len == 0 {
        return None;
    }
    let mut buf = vec![0u8; len];
    if unsafe { libc::sysctlbyname(name.as_ptr(), buf.as_mut_ptr().cast(), &mut len, std::ptr::null_mut(), 0) } != 0 {
        return None;
    }
    buf.truncate(len);
    let s = String::from_utf8_lossy(&buf).trim_end_matches('\0').trim().to_owned();
    (!s.is_empty()).then_some(s)
}

#[cfg(windows)]
fn windows_registry_string(key: &str, value: &str) -> Option<String> {
    use windows_sys::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW};
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let (key, value) = (wide(key), wide(value));
    let mut buf = [0u16; 512];
    let mut len = (buf.len() * 2) as u32;
    let status = unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, key.as_ptr(), value.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut len) };
    if status != 0 {
        return None;
    }
    let n = (len as usize / 2).min(buf.len());
    Some(String::from_utf16_lossy(&buf[..n]).trim_end_matches('\0').to_owned())
}

// ---------------------------------------------------------------------------
// The last minute

/// What the Developer panel measured over the last minute, as a report
/// takes it: a frame each, and readings of the memory and the processor.
#[derive(Debug, Clone, Default)]
pub struct LastMinute {
    /// (seconds, milliseconds since the frame before, milliseconds of work)
    pub frames: Vec<(f64, f32, f32)>,
    /// (seconds, megabytes in use)
    pub memory: Vec<(f64, f32)>,
    /// (seconds, percent of one core)
    pub cpu: Vec<(f64, f32)>,
    pub peak_memory: f32,
    /// What Massif keeps: name, size, note.
    pub keeps: Vec<(String, String, String)>,
}

impl LastMinute {
    /// A line a frame, with the latest reading of the memory and the
    /// processor before it.
    pub fn csv(&self) -> String {
        let t0 = self.frames.first().map_or(0.0, |f| f.0);
        let mut out = String::from("seconds,frame_ms,work_ms,memory_mb,cpu_percent\n");
        let (mut m, mut c) = (0, 0);
        for &(t, gap, work) in &self.frames {
            while m + 1 < self.memory.len() && self.memory[m + 1].0 <= t {
                m += 1;
            }
            while c + 1 < self.cpu.len() && self.cpu[c + 1].0 <= t {
                c += 1;
            }
            let mem = self.memory.get(m).filter(|x| x.0 <= t).map_or(String::new(), |x| format!("{:.0}", x.1));
            let cpu = self.cpu.get(c).filter(|x| x.0 <= t).map_or(String::new(), |x| format!("{:.0}", x.1));
            out.push_str(&format!("{:.3},{gap:.2},{work:.2},{mem},{cpu}\n", t - t0));
        }
        out
    }

    /// The figures that matter, a line each.
    pub fn summary(&self) -> Vec<(String, String)> {
        let gaps: Vec<f32> = self.frames.iter().map(|f| f.1).collect();
        let work: Vec<f32> = self.frames.iter().map(|f| f.2).collect();
        let span = match (self.frames.first(), self.frames.last()) {
            (Some(a), Some(b)) => b.0 - a.0,
            _ => 0.0,
        };
        let mut v = vec![("Frames measured".to_owned(), format!("{} over {span:.0} s", self.frames.len()))];
        if !gaps.is_empty() {
            let total: f32 = gaps.iter().sum();
            v.push(("Frames a second".to_owned(), format!("{:.0} on average", gaps.len() as f32 * 1000.0 / total.max(1e-3))));
            v.push(("Frame time".to_owned(), format!("median {:.1} ms, 99th percentile {:.1} ms, worst {:.1} ms", percentile(&gaps, 0.5), percentile(&gaps, 0.99), percentile(&gaps, 1.0))));
            v.push(("Work a frame".to_owned(), format!("median {:.1} ms, worst {:.1} ms", percentile(&work, 0.5), percentile(&work, 1.0))));
            v.push(("Frames over 50 ms".to_owned(), format!("{}", gaps.iter().filter(|g| **g > 50.0).count())));
        }
        if let Some(m) = self.memory.last() {
            v.push(("Memory".to_owned(), format!("{:.0} MB now, highest {:.0} MB", m.1, self.peak_memory)));
        }
        if !self.cpu.is_empty() {
            let mean = self.cpu.iter().map(|c| c.1).sum::<f32>() / self.cpu.len() as f32;
            v.push(("Processor".to_owned(), format!("{mean:.0} % of one core on average, highest {:.0} %", self.cpu.iter().map(|c| c.1).fold(0.0, f32::max))));
        }
        v
    }
}

/// The value under which `q` of `v` are (0.5 the median, 1 the largest).
fn percentile(v: &[f32], q: f32) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    let mut s = v.to_vec();
    s.sort_by(f32::total_cmp);
    s[((s.len() - 1) as f32 * q).round() as usize]
}

// ---------------------------------------------------------------------------
// The report

/// What a report is made of, gathered by the app when the dialog opens.
#[derive(Debug, Clone, Default)]
pub struct Inputs {
    pub system: System,
    /// The settings that change how Massif draws: name, value.
    pub settings: Vec<(String, String)>,
    pub last_minute: LastMinute,
    /// The map: its size, height, levels. Empty with no map open.
    pub map: Vec<(String, String)>,
    /// The folder of `startup.log` and `graphics.txt`.
    pub log_dir: Option<PathBuf>,
    /// The folder written `~`.
    pub home: Option<PathBuf>,
    /// The user's own words: what went wrong.
    pub what_happened: String,
}

/// A file of the report.
#[derive(Debug, Clone)]
pub struct Part {
    /// Its name in the zip.
    pub name: String,
    /// What it is, in a few words.
    pub about: &'static str,
    pub bytes: Vec<u8>,
    /// Text the dialog can show.
    pub text: bool,
}

/// The map in a report, written as a `.massif`.
pub struct MapFile {
    pub bytes: Vec<u8>,
}

/// The name a report's map has in the zip: never the user's file name.
pub const MAP_NAME: &str = "map.massif";

/// The files of a report, every text scrubbed. The map goes in only when
/// given (ticked in the dialog).
pub fn parts(inputs: &Inputs, recording: Option<&crate::perf::Recording>, map: Option<MapFile>) -> Vec<Part> {
    let home = inputs.home.as_deref();
    let text = |name: &str, about: &'static str, s: &str| Part { name: name.to_owned(), about, bytes: scrub(s, home).into_bytes(), text: true };
    let mut out = vec![text("report.txt", tn!("Massif, the computer, the settings, the map, the last minute"), &report_text(inputs, recording))];
    if let Some(dir) = &inputs.log_dir {
        for (file, about) in [
            ("startup.log", tn!("What Massif did as it started, and any error since")),
            ("startup.previous.log", tn!("The same for the start before")),
            ("graphics.txt", tn!("Graphics APIs set aside on this computer")),
        ] {
            if let Ok(bytes) = std::fs::read(dir.join(file)) {
                out.push(text(file, about, &String::from_utf8_lossy(&bytes)));
            }
        }
    }
    if !inputs.last_minute.frames.is_empty() {
        out.push(text("last-minute.csv", tn!("Each frame of the last minute: its time, the memory, the processor"), &inputs.last_minute.csv()));
    }
    if let Some(r) = recording.filter(|r| r.frames() > 0) {
        out.extend(r.parts("recording/").into_iter().filter(|p| p.name != "recording/system.txt").map(|mut p| {
            p.bytes = scrub(&String::from_utf8_lossy(&p.bytes), home).into_bytes();
            p
        }));
    }
    out.extend(map.map(map_part));
    out
}

/// The map as a file of the report.
pub fn map_part(map: MapFile) -> Part {
    Part { name: MAP_NAME.to_owned(), about: tn!("The map itself, as it is now"), bytes: map.bytes, text: false }
}

/// `report.txt`: everything but the files, in words.
pub fn report_text(inputs: &Inputs, recording: Option<&crate::perf::Recording>) -> String {
    let mut s = String::from("Massif problem report\n");
    let section = |s: &mut String, title: &str, lines: &[(String, String)]| {
        s.push_str(&format!("\n## {title}\n"));
        for (k, v) in lines {
            s.push_str(&format!("{k}: {v}\n"));
        }
    };
    if !inputs.what_happened.trim().is_empty() {
        s.push_str(&format!("\n## What happened\n{}\n", inputs.what_happened.trim()));
    }
    section(&mut s, "Computer", &inputs.system.lines());
    section(&mut s, "Settings", &inputs.settings);
    if inputs.map.is_empty() {
        s.push_str("\n## Map\nNo map open\n");
    } else {
        section(&mut s, "Map", &inputs.map);
    }
    section(&mut s, "Last minute", &inputs.last_minute.summary());
    if !inputs.last_minute.keeps.is_empty() {
        s.push_str("\n## What Massif keeps\n");
        for (k, v, note) in &inputs.last_minute.keeps {
            s.push_str(&format!("{k}: {v}{}\n", if note.is_empty() { String::new() } else { format!(" ({note})") }));
        }
    }
    if let Some(r) = recording.filter(|r| r.frames() > 0) {
        s.push('\n');
        s.push_str(&r.summary());
    }
    s
}

/// A few lines for Discord: the version, the computer, how it ran, the
/// last error the log has.
pub fn short_summary(inputs: &Inputs, recording: Option<&crate::perf::Recording>) -> String {
    let sys = &inputs.system;
    let mut s = format!("**Massif {}** · {}\n", version(), sys.os);
    s.push_str(&format!("{} ({} threads) · {} RAM\n", sys.cpu, sys.threads, sys.ram.map_or_else(|| "?".to_owned(), gb)));
    if let Some(g) = &sys.gpu {
        s.push_str(&format!("{} · {} · {}\n", g.name, g.backend, if g.driver.is_empty() { "driver not said" } else { &g.driver }));
    }
    let lm = &inputs.last_minute;
    let gaps: Vec<f32> = lm.frames.iter().map(|f| f.1).collect();
    if !gaps.is_empty() {
        let total: f32 = gaps.iter().sum();
        s.push_str(&format!(
            "Last minute: {:.0} fps, worst frame {:.0} ms, {} frames over 50 ms · memory {:.0} MB (highest {:.0} MB)\n",
            gaps.len() as f32 * 1000.0 / total.max(1e-3),
            percentile(&gaps, 1.0),
            gaps.iter().filter(|g| **g > 50.0).count(),
            lm.memory.last().map_or(0.0, |m| m.1),
            lm.peak_memory
        ));
    }
    if let Some(r) = recording.filter(|r| r.frames() > 0) {
        let (median, worst) = r.busy_median_worst();
        s.push_str(&format!("Recording: {:.0} s, {} frames, median {median:.0} ms, worst {worst:.0} ms\n", r.seconds(), r.frames()));
    }
    if let Some((_, size)) = inputs.map.iter().find(|(k, _)| k == "Size") {
        let levels = inputs.map.iter().find(|(k, _)| k == "Levels").map_or(String::new(), |(_, v)| format!(", {v} levels"));
        s.push_str(&format!("Map: {size}{levels}\n"));
    }
    if let Some(err) = inputs.log_dir.as_deref().and_then(last_error) {
        s.push_str(&format!("Last error: {err}\n"));
    }
    if !inputs.what_happened.trim().is_empty() {
        s.push_str(&format!("> {}\n", inputs.what_happened.trim().replace('\n', "\n> ")));
    }
    scrub(&s, inputs.home.as_deref())
}

/// The first line of the last panic or failure in the latest start's log,
/// or the one before.
pub fn last_error(dir: &Path) -> Option<String> {
    ["startup.log", "startup.previous.log"].iter().find_map(|f| {
        let log = std::fs::read_to_string(dir.join(f)).ok()?;
        log.lines()
            .rev()
            .find(|l| l.contains("panic in thread") || l.contains("could not start") || l.contains("crash"))
            .map(|l| l.split_once("] ").map_or(l, |(_, rest)| rest).chars().take(200).collect())
    })
}

/// A suggested name for the zip, at the user's local time:
/// `Massif report 2026-11-16 14.05.zip`.
pub fn file_name() -> String {
    let (y, m, d, h, mi, _) = local_now();
    format!("Massif report {y:04}-{m:02}-{d:02} {h:02}.{mi:02}.zip")
}

/// The year, month, day, hour, minute and second of now, as the user's
/// clock says.
#[cfg(windows)]
pub fn local_now() -> (i64, u32, u32, u32, u32, u32) {
    let mut t: windows_sys::Win32::Foundation::SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe { windows_sys::Win32::System::SystemInformation::GetLocalTime(&mut t) };
    (t.wYear as i64, t.wMonth as u32, t.wDay as u32, t.wHour as u32, t.wMinute as u32, t.wSecond as u32)
}

/// The year, month, day, hour, minute and second of now, as the user's
/// clock says (UTC where it cannot be read).
#[cfg(not(windows))]
pub fn local_now() -> (i64, u32, u32, u32, u32, u32) {
    #[cfg(unix)]
    {
        let t = unsafe { libc::time(std::ptr::null_mut()) };
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        if !unsafe { libc::localtime_r(&t, &mut tm) }.is_null() {
            return (tm.tm_year as i64 + 1900, tm.tm_mon as u32 + 1, tm.tm_mday as u32, tm.tm_hour as u32, tm.tm_min as u32, tm.tm_sec as u32);
        }
    }
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()) as i64;
    let (y, m, d) = civil(secs.div_euclid(86400));
    let t = secs.rem_euclid(86400);
    (y, m, d, (t / 3600) as u32, (t % 3600 / 60) as u32, (t % 60) as u32)
}

/// Days since 1970 as a date (Howard Hinnant's `civil_from_days`).
fn civil(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

// ---------------------------------------------------------------------------
// The zip

/// Write `parts` as a zip at `path` (through a file beside it, renamed
/// once whole).
pub fn save(path: &Path, parts: &[Part]) -> std::io::Result<()> {
    let tmp = path.with_extension("zip.part");
    let result = (|| {
        let mut f = std::io::BufWriter::new(std::fs::File::create(&tmp)?);
        zip(&mut f, parts)?;
        f.into_inner().map_err(|e| e.into_error())?.sync_all()
    })();
    match result {
        Ok(()) => std::fs::rename(&tmp, path),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// A zip of `parts`, each deflated (or stored, when deflating does not make
/// it smaller), with the time of now.
pub fn zip<W: Write>(out: &mut W, parts: &[Part]) -> std::io::Result<()> {
    let (time, date) = dos_time(std::time::SystemTime::now());
    let mut offset: u64 = 0;
    let mut central = Vec::new();
    for p in parts {
        let crc = crc32fast::hash(&p.bytes);
        let deflated = {
            let mut e = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
            e.write_all(&p.bytes)?;
            e.finish()?
        };
        let (method, data): (u16, &[u8]) = if deflated.len() < p.bytes.len() { (8, &deflated) } else { (0, &p.bytes) };
        let name = p.name.as_bytes();
        let (size, csize) = (u32::try_from(p.bytes.len()), u32::try_from(data.len()));
        let (Ok(size), Ok(csize), Ok(at)) = (size, csize, u32::try_from(offset)) else {
            return Err(std::io::Error::other("a report over 4 GB"));
        };
        let mut local = Vec::with_capacity(30 + name.len());
        local.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        // Version 2.0; names in UTF-8 (bit 11).
        for v in [20u16, 1 << 11, method, time, date] {
            local.extend_from_slice(&v.to_le_bytes());
        }
        for v in [crc, csize, size] {
            local.extend_from_slice(&v.to_le_bytes());
        }
        local.extend_from_slice(&(name.len() as u16).to_le_bytes());
        local.extend_from_slice(&0u16.to_le_bytes());
        local.extend_from_slice(name);
        out.write_all(&local)?;
        out.write_all(data)?;

        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        for v in [20u16, 20, 1 << 11, method, time, date] {
            central.extend_from_slice(&v.to_le_bytes());
        }
        for v in [crc, csize, size] {
            central.extend_from_slice(&v.to_le_bytes());
        }
        // Name length, extra, comment, disk, internal attributes.
        for v in [name.len() as u16, 0, 0, 0, 0] {
            central.extend_from_slice(&v.to_le_bytes());
        }
        // External attributes: a file readable by all (rw-r--r--).
        central.extend_from_slice(&(0o100644u32 << 16).to_le_bytes());
        central.extend_from_slice(&at.to_le_bytes());
        central.extend_from_slice(name);
        offset += local.len() as u64 + data.len() as u64;
    }
    let Ok(cd_at) = u32::try_from(offset) else { return Err(std::io::Error::other("a report over 4 GB")) };
    out.write_all(&central)?;
    let mut end = Vec::with_capacity(22);
    end.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    for v in [0u16, 0, parts.len() as u16, parts.len() as u16] {
        end.extend_from_slice(&v.to_le_bytes());
    }
    end.extend_from_slice(&(central.len() as u32).to_le_bytes());
    end.extend_from_slice(&cd_at.to_le_bytes());
    end.extend_from_slice(&0u16.to_le_bytes());
    out.write_all(&end)
}

/// The time and date as a zip writes them (MS-DOS), in UTC.
fn dos_time(now: std::time::SystemTime) -> (u16, u16) {
    let secs = now.duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()) as i64;
    let (y, m, d) = civil(secs.div_euclid(86400));
    let t = secs.rem_euclid(86400);
    let time = ((t / 3600) << 11 | (t % 3600 / 60) << 5 | ((t % 60) / 2)) as u16;
    let date = (((y - 1980).clamp(0, 127) as u32) << 9 | m << 5 | d) as u16;
    (time, date)
}

/// The files of a zip: name and bytes (stored or deflated), for the tests
/// and to check a report.
#[cfg(test)]
pub fn unzip(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
    use std::io::Read;
    let u16_at = |i: usize| u16::from_le_bytes([bytes[i], bytes[i + 1]]) as usize;
    let u32_at = |i: usize| u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap()) as usize;
    let end = (0..=bytes.len() - 22).rev().find(|&i| u32_at(i) == 0x0605_4b50).expect("an end of central directory");
    let (count, mut at) = (u16_at(end + 10), u32_at(end + 16));
    let mut out = Vec::new();
    for _ in 0..count {
        assert_eq!(u32_at(at), 0x0201_4b50);
        let (method, crc, csize, size) = (u16_at(at + 10), u32_at(at + 16), u32_at(at + 20), u32_at(at + 24));
        let (n, x, c, local) = (u16_at(at + 28), u16_at(at + 30), u16_at(at + 32), u32_at(at + 42));
        let name = String::from_utf8(bytes[at + 46..at + 46 + n].to_vec()).unwrap();
        let data_at = local + 30 + u16_at(local + 26) + u16_at(local + 28);
        let data = &bytes[data_at..data_at + csize];
        let content = match method {
            0 => data.to_vec(),
            8 => {
                let mut v = Vec::new();
                flate2::read::DeflateDecoder::new(data).read_to_end(&mut v).unwrap();
                v
            }
            m => panic!("method {m}"),
        };
        assert_eq!(content.len(), size);
        assert_eq!(crc32fast::hash(&content) as usize, crc, "{name}: its CRC");
        out.push((name, content));
        at += 46 + n + x + c;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_and_other_accounts_are_hidden() {
        let home = Path::new("/Users/ann");
        let log = "asked to open /Users/ann/Maps/Island.massif\nread /Users/anne/x and /Users/bob/Desktop/y.massif, /Users/Shared/z, /tmp/q";
        assert_eq!(scrub(log, Some(home)), "asked to open ~/Maps/Island.massif\nread /Users/<user>/x and /Users/<user>/Desktop/y.massif, /Users/Shared/z, /tmp/q");
        let win = Path::new("C:\\Users\\Ann Lee");
        let s = scrub("C:\\Users\\Ann Lee\\AppData\\Local\\Massif and C:/Users/Ann Lee/x and D:\\Users\\zed\\m", Some(win));
        assert_eq!(s, "~\\AppData\\Local\\Massif and ~/x and D:\\Users\\<user>\\m");
        assert_eq!(scrub("/home/joe/.local/state/massif", Some(Path::new("/home/joe"))), "~/.local/state/massif");
        // No home known: the name still goes.
        assert_eq!(scrub("/home/joe/a", None), "/home/<user>/a");
    }

    #[test]
    fn a_zip_reads_back() {
        let parts = vec![
            Part { name: "a.txt".into(), about: "", bytes: b"hello hello hello hello hello".repeat(40), text: true },
            Part { name: "dir/b.bin".into(), about: "", bytes: vec![7, 1, 200], text: false },
            Part { name: "empty".into(), about: "", bytes: Vec::new(), text: true },
        ];
        let mut z = Vec::new();
        zip(&mut z, &parts).unwrap();
        let back = unzip(&z);
        assert_eq!(back.len(), 3);
        for (p, (name, bytes)) in parts.iter().zip(&back) {
            assert_eq!(&p.name, name);
            assert_eq!(&p.bytes, bytes);
        }
        assert!(z.len() < 500, "the text is deflated: {}", z.len());
    }

    fn inputs(dir: &Path, home: &Path) -> Inputs {
        Inputs {
            system: System { os: "macOS 26.2 aarch64".into(), cpu: "Apple M2".into(), threads: 8, ram: Some(16 << 30), gpu: None, window: Some(([1440.0, 900.0], 2.0)) },
            settings: vec![("Reduce motion".into(), "off".into())],
            last_minute: LastMinute { frames: vec![(10.0, 16.0, 3.0), (10.016, 70.0, 60.0)], memory: vec![(10.0, 900.0)], cpu: vec![(10.0, 40.0)], peak_memory: 950.0, keeps: vec![] },
            map: vec![("Size".into(), "1024 × 1024 blocks".into()), ("Levels".into(), "2".into())],
            log_dir: Some(dir.to_owned()),
            home: Some(home.to_owned()),
            what_happened: "It froze when I exported".into(),
        }
    }

    #[test]
    fn a_report_has_its_files_scrubbed_and_the_map_only_when_asked() {
        let tmp = std::env::temp_dir().join(format!("massif-report-test-{}", std::process::id()));
        let home = tmp.join("home").join("ann");
        let logs = home.join("Library/Logs/Massif");
        std::fs::create_dir_all(&logs).unwrap();
        std::fs::write(logs.join("startup.log"), format!("[      0 ms] Massif\n[     10 ms] asked to open {}/Maps/Secret.massif\n[   900 ms] panic in thread main: boom\n", home.display())).unwrap();
        std::fs::write(logs.join("graphics.txt"), "vulkan: stuck\n").unwrap();
        let inputs = inputs(&logs, &home);

        let names = |parts: &[Part]| parts.iter().map(|p| p.name.clone()).collect::<Vec<_>>();
        let without = parts(&inputs, None, None);
        assert_eq!(names(&without), ["report.txt", "startup.log", "graphics.txt", "last-minute.csv"]);
        let with = parts(&inputs, None, Some(MapFile { bytes: vec![1, 2, 3] }));
        assert_eq!(names(&with).last().map(String::as_str), Some(MAP_NAME));

        let zip_path = tmp.join("r.zip");
        save(&zip_path, &without).unwrap();
        let files = unzip(&std::fs::read(&zip_path).unwrap());
        assert_eq!(files.iter().map(|f| f.0.as_str()).collect::<Vec<_>>(), ["report.txt", "startup.log", "graphics.txt", "last-minute.csv"]);
        let home_text = home.display().to_string();
        for (name, bytes) in &files {
            let text = String::from_utf8_lossy(bytes);
            assert!(!text.contains(&home_text), "{name} has the home folder: {text}");
            assert!(!text.contains("/ann/"), "{name} has the user's name");
        }
        let log = String::from_utf8_lossy(&files[1].1);
        assert!(log.contains("asked to open ~/Maps/Secret.massif"), "{log}");
        let report = String::from_utf8_lossy(&files[0].1);
        assert!(report.contains("It froze when I exported") && report.contains("Size: 1024 × 1024 blocks") && report.contains("Frames over 50 ms: 1"), "{report}");
        let short = short_summary(&inputs, None);
        assert!(short.contains("Last error: panic in thread main: boom") && short.contains("Apple M2 (8 threads)"), "{short}");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn dates_from_days() {
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(1_794_837_900 / 86400), (2026, 11, 16));
        assert_eq!(civil(11016), (2000, 2, 29));
        let name = file_name();
        assert!(name.starts_with("Massif report 20") && name.ends_with(".zip"), "{name}");
    }
}
