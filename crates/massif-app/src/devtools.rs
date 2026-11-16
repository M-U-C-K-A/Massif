//! The developer panel (Settings ▸ Developer): how Massif runs, live — its
//! frames, its memory as the system counts it, the processor's time, what
//! it keeps decoded, its tasks, its graphics card, and why it draws again.

use crate::theme::{self, Palette, RichTextExt};
use egui::{Color32, RichText, Sense, Stroke, Ui, pos2, vec2};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Seconds of history the charts show.
const SPAN: f64 = 15.0;
/// Seconds of history kept: a problem report takes the last minute.
const KEEP: f64 = 60.0;
/// Seconds between two readings of the memory and the processor.
const EVERY: f64 = 0.25;

/// What was measured, frame after frame.
#[derive(Default)]
pub struct DevStats {
    /// (time, milliseconds since the frame before, milliseconds of work)
    frames: VecDeque<(f64, f32, f32)>,
    /// (time, megabytes in use)
    memory: VecDeque<(f64, f32)>,
    /// (time, percent of one core)
    cpu: VecDeque<(f64, f32)>,
    started: Option<Instant>,
    last_start: Option<Instant>,
    sampled: f64,
    cpu_before: Option<(Instant, Duration)>,
    peak_memory: f32,
}

impl DevStats {
    /// A frame begins.
    pub fn begin(&mut self) {
        self.started = Some(Instant::now());
    }

    /// The frame's work is done, at `time` (seconds, egui's clock).
    pub fn end(&mut self, time: f64) {
        let Some(start) = self.started.take() else { return };
        let work = start.elapsed().as_secs_f32() * 1000.0;
        let gap = self.last_start.map_or(0.0, |l| start.duration_since(l).as_secs_f32() * 1000.0);
        self.last_start = Some(start);
        if gap > 0.0 {
            self.frames.push_back((time, gap, work));
        }
        while self.frames.front().is_some_and(|f| f.0 < time - KEEP) {
            self.frames.pop_front();
        }
        if time - self.sampled >= EVERY {
            self.sampled = time;
            if let Some(bytes) = memory_in_use() {
                let mb = bytes as f32 / (1u64 << 20) as f32;
                self.peak_memory = self.peak_memory.max(mb);
                self.memory.push_back((time, mb));
            }
            if let Some(used) = cpu_time() {
                let now = Instant::now();
                if let Some((then, before)) = self.cpu_before {
                    let wall = now.duration_since(then).as_secs_f32();
                    if wall > 0.0 {
                        self.cpu.push_back((time, used.saturating_sub(before).as_secs_f32() / wall * 100.0));
                    }
                }
                self.cpu_before = Some((now, used));
            }
            for q in [&mut self.memory, &mut self.cpu] {
                while q.front().is_some_and(|v| v.0 < time - KEEP) {
                    q.pop_front();
                }
            }
        }
    }

    /// The frame that ended last: when its work began, the milliseconds
    /// since the frame before, and those of its work.
    pub fn last_frame(&self) -> Option<(Instant, f32, f32)> {
        let (_, gap, work) = *self.frames.back()?;
        Some((self.last_start?, gap, work))
    }

    /// The latest reading of the memory (MB) and the processor (% of a core).
    pub fn latest(&self) -> (Option<f32>, Option<f32>) {
        (self.memory.back().map(|m| m.1), self.cpu.back().map(|c| c.1))
    }

    /// What was measured over the last minute, for a problem report.
    pub fn last_minute(&self, keeps: Vec<(String, String, String)>) -> crate::report::LastMinute {
        crate::report::LastMinute {
            frames: self.frames.iter().copied().collect(),
            memory: self.memory.iter().copied().collect(),
            cpu: self.cpu.iter().copied().collect(),
            peak_memory: self.peak_memory,
            keeps,
        }
    }

    /// Frames a second over the last second.
    pub fn fps(&self) -> f32 {
        let Some(&(last, ..)) = self.frames.back() else { return 0.0 };
        let recent: Vec<f32> = self.frames.iter().filter(|f| f.0 > last - 1.0).map(|f| f.1).collect();
        let total: f32 = recent.iter().sum();
        if total > 0.0 { recent.len() as f32 * 1000.0 / total } else { 0.0 }
    }
}

/// The memory of the process as the system counts it (what the Activity
/// Monitor, the Task Manager or `top` show), in bytes.
pub fn memory_in_use() -> Option<u64> {
    #[cfg(target_os = "macos")]
    {
        let mut info: libc::rusage_info_v2 = unsafe { std::mem::zeroed() };
        let ok = unsafe { libc::proc_pid_rusage(libc::getpid(), libc::RUSAGE_INFO_V2, (&mut info as *mut libc::rusage_info_v2).cast()) };
        (ok == 0).then_some(info.ri_phys_footprint)
    }
    #[cfg(target_os = "linux")]
    {
        let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
        let resident: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
        Some(resident * unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as u64)
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
        use windows_sys::Win32::System::Threading::GetCurrentProcess;
        let mut c: PROCESS_MEMORY_COUNTERS = unsafe { std::mem::zeroed() };
        c.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        let ok = unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb) };
        (ok != 0).then_some(c.WorkingSetSize as u64)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
        None
    }
}

/// The processor time the process has had, all its threads together.
pub fn cpu_time() -> Option<Duration> {
    #[cfg(unix)]
    {
        let mut u: libc::rusage = unsafe { std::mem::zeroed() };
        if unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut u) } != 0 {
            return None;
        }
        let t = |v: libc::timeval| Duration::new(v.tv_sec as u64, v.tv_usec as u32 * 1000);
        Some(t(u.ru_utime) + t(u.ru_stime))
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::FILETIME;
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
        let zero = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
        let (mut a, mut b, mut kernel, mut user) = (zero, zero, zero, zero);
        if unsafe { GetProcessTimes(GetCurrentProcess(), &mut a, &mut b, &mut kernel, &mut user) } == 0 {
            return None;
        }
        let ticks = |f: FILETIME| ((f.dwHighDateTime as u64) << 32 | f.dwLowDateTime as u64) * 100;
        Some(Duration::from_nanos(ticks(kernel) + ticks(user)))
    }
    #[cfg(not(any(unix, windows)))]
    {
        None
    }
}

/// What the rest of Massif tells the panel, each frame.
pub struct DevInfo<'a> {
    pub keeps: Vec<(&'static str, String, String)>,
    pub map: Vec<(&'static str, String)>,
    pub jobs: Vec<String>,
    pub gpu: Option<&'a egui_wgpu::RenderState>,
}

/// The panel.
pub fn panel(ui: &mut Ui, stats: &DevStats, info: &DevInfo<'_>) {
    let pal = Palette::of(ui);
    // Live: drawn again every frame while shown.
    ui.ctx().request_repaint();
    let fps = stats.fps();
    // The charts show the last seconds of the minute kept.
    let now = stats.frames.back().map_or(0.0, |f| f.0);
    let recent = |t: f64| t >= now - SPAN;
    let frame_ms: Vec<f32> = stats.frames.iter().filter(|f| recent(f.0)).map(|f| f.1).collect();
    let work_ms: Vec<f32> = stats.frames.iter().filter(|f| recent(f.0)).map(|f| f.2).collect();
    let last = |v: &VecDeque<(f64, f32)>| v.back().map_or(0.0, |x| x.1);
    let mean = |v: &[f32]| if v.is_empty() { 0.0 } else { v.iter().sum::<f32>() / v.len() as f32 };
    let worst = |v: &[f32]| v.iter().copied().fold(0.0f32, f32::max);

    // The figures that matter, big: two by two.
    let fps_color = if fps >= 55.0 { pal.success } else if fps >= 30.0 { Color32::from_rgb(234, 179, 8) } else { pal.danger };
    let tiles = [
        ("Frames a second", format!("{fps:.0}"), format!("worst frame {:.1} ms", worst(&frame_ms)), fps_color),
        ("Work a frame", format!("{:.1} ms", mean(&work_ms)), format!("worst {:.1} ms", worst(&work_ms)), pal.text),
        ("Memory", format!("{:.0} MB", last(&stats.memory)), format!("highest {:.0} MB", stats.peak_memory), pal.accent),
        ("Processor", format!("{:.0} %", last(&stats.cpu)), format!("of one core, {} cores", std::thread::available_parallelism().map_or(1, |n| n.get())), Color32::from_rgb(96, 165, 250)),
    ];
    for pair in tiles.chunks(2) {
        ui.columns(2, |cols| {
            for (ui, (name, value, note, color)) in cols.iter_mut().zip(pair) {
                crate::widgets::well(ui, |ui| {
                    ui.vertical(|ui| {
                        ui.set_min_width(ui.available_width());
                        ui.label(RichText::new(*name).size(11.0).color(pal.weak));
                        ui.label(RichText::new(value).size(22.0).strong().color(*color));
                        ui.label(RichText::new(note).size(10.5).color(pal.weak));
                    });
                });
            }
        });
        ui.add_space(6.0);
    }
    ui.add_space(4.0);

    let fps_series: Vec<f32> = frame_ms.iter().map(|ms| if *ms > 0.0 { 1000.0 / ms } else { 0.0 }).collect();
    chart(ui, "Frames a second", &[(&fps_series, pal.success)], Some(60.0), "");
    chart(ui, "Frame time and the work in it (ms)", &[(&frame_ms, pal.weak), (&work_ms, pal.accent)], Some(16.7), " ms");
    let mem: Vec<f32> = stats.memory.iter().filter(|m| recent(m.0)).map(|m| m.1).collect();
    chart(ui, "Memory (MB)", &[(&mem, pal.accent)], None, " MB");
    let cpu: Vec<f32> = stats.cpu.iter().filter(|c| recent(c.0)).map(|c| c.1).collect();
    chart(ui, "Processor (% of one core)", &[(&cpu, Color32::from_rgb(96, 165, 250))], Some(100.0), " %");

    ui.add_space(6.0);
    section(ui, "What Massif keeps");
    crate::widgets::well(ui, |ui| {
        for (name, value, note) in &info.keeps {
            line(ui, name, value, note);
        }
    });
    section(ui, "The map");
    crate::widgets::well(ui, |ui| {
        for (name, value) in &info.map {
            line(ui, name, value, "");
        }
    });
    section(ui, "Tasks");
    crate::widgets::well(ui, |ui| {
        if info.jobs.is_empty() {
            ui.label(RichText::new("None running").size(12.0).color(pal.weak));
        }
        for job in &info.jobs {
            ui.label(RichText::new(job).size(12.0));
        }
    });
    section(ui, "System");
    crate::widgets::well(ui, |ui| {
        if let Some(rs) = info.gpu {
            let a = rs.adapter.get_info();
            line(ui, "Graphics", &a.name, &format!("{:?}", a.backend));
            if !a.driver.is_empty() {
                line(ui, "Driver", &a.driver, &a.driver_info);
            }
        }
        let (size, ppp) = ui.ctx().input(|i| (i.viewport().inner_rect.map(|r| r.size()), i.pixels_per_point));
        let s = size.unwrap_or_default();
        line(ui, "Window", &format!("{:.0} × {:.0}", s.x, s.y), &format!("points, {ppp:.1} pixels a point"));
        line(ui, "Threads", &format!("{}", rayon::current_num_threads()), "working together");
        line(ui, "Massif", env!("CARGO_PKG_VERSION"), &format!("{} {}", std::env::consts::OS, std::env::consts::ARCH));
    });
    section(ui, "Why it draws again");
    crate::widgets::well(ui, |ui| {
        let causes = ui.ctx().repaint_causes();
        let mut seen: Vec<String> = causes.iter().map(|c| format!("{c}")).collect();
        seen.sort();
        seen.dedup();
        for c in seen.iter().take(8) {
            ui.label(RichText::new(c).size(11.0).monospace().color(pal.weak));
        }
    });
}

/// A line: `name` on the left, `value` on the right, `note` under it.
fn line(ui: &mut Ui, name: &str, value: &str, note: &str) {
    let pal = Palette::of(ui);
    ui.horizontal(|ui| {
        ui.label(RichText::new(name).size(12.0));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add(egui::Label::new(RichText::new(value).size(12.0).monospace()).truncate());
        });
    });
    if !note.is_empty() {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
            ui.add(egui::Label::new(RichText::new(note).size(10.5).color(pal.weak)).truncate());
        });
    }
    ui.add_space(2.0);
}

fn section(ui: &mut Ui, title: &str) {
    ui.add_space(10.0);
    ui.label(RichText::new(title).size(13.0).medium());
    ui.add_space(4.0);
}

/// A chart of `series` (each its values over the last [`SPAN`] seconds, and
/// its colour), with a line at `mark`.
fn chart(ui: &mut Ui, title: &str, series: &[(&[f32], Color32)], mark: Option<f32>, unit: &str) {
    let pal = Palette::of(ui);
    ui.label(RichText::new(title).size(11.5).color(pal.weak));
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 74.0), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, theme::GROUP_RADIUS, theme::mix(pal.panel, pal.text, 0.04));
    let top = series.iter().flat_map(|(v, _)| v.iter().copied()).fold(mark.unwrap_or(1.0), f32::max) * 1.12;
    let inner = rect.shrink2(vec2(8.0, 8.0));
    let y = |v: f32| inner.bottom() - (v / top).clamp(0.0, 1.0) * inner.height();
    // Quarter lines, and the mark.
    for k in 1..4 {
        let yy = inner.bottom() - inner.height() * k as f32 / 4.0;
        painter.hline(inner.x_range(), yy, Stroke::new(1.0, theme::mix(pal.panel, pal.text, 0.07)));
    }
    if let Some(m) = mark {
        painter.hline(inner.x_range(), y(m), Stroke::new(1.0, pal.weak.gamma_multiply(0.6)));
        painter.text(pos2(inner.left() + 2.0, y(m) - 2.0), egui::Align2::LEFT_BOTTOM, format!("{m}{unit}"), egui::FontId::proportional(10.0), pal.weak);
    }
    for (k, (values, color)) in series.iter().enumerate() {
        if values.len() < 2 {
            continue;
        }
        let n = values.len();
        let points: Vec<egui::Pos2> = values.iter().enumerate().map(|(i, v)| pos2(inner.left() + inner.width() * i as f32 / (n - 1) as f32, y(*v))).collect();
        // The area under the line, faint.
        let fill = color.gamma_multiply(0.12);
        for w in points.windows(2) {
            painter.add(egui::Shape::convex_polygon(vec![w[0], w[1], pos2(w[1].x, inner.bottom()), pos2(w[0].x, inner.bottom())], fill, Stroke::NONE));
        }
        painter.add(egui::Shape::line(points, Stroke::new(1.5, *color)));
        let last = values[n - 1];
        // Each series' last value, one under the other.
        painter.text(pos2(inner.right(), inner.top() + 13.0 * k as f32), egui::Align2::RIGHT_TOP, format!("{last:.1}{unit}"), egui::FontId::monospace(11.0), *color);
    }
    ui.add_space(6.0);
}
