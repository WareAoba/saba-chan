//! `monitor` — 시스템 모니터 (GET /api/system/metrics)
//!
//! 값은 데몬이 잰다. 여기서는 막대(인스턴스 - 나머지 프로세스 - 유휴)로 그리기만 한다.

use crate::cli::MonitorArgs;
use crate::ctx::Ctx;
use crate::error::Result;
use crate::output;
use console::{style, Color};
use serde_json::Value;
use std::time::Duration;

const BAR_WIDTH: usize = 30;
/// 디스크 IO·네트워크 막대의 최소 눈금 (1 MB/s)
const MIN_RATE_SCALE: f64 = 1024.0 * 1024.0;
/// 인스턴스 색. 넘치면 "기타 인스턴스"로 묶는다 (GUI의 계열색 8개와 같은 수)
const COLORS: [Color; 8] = [
    Color::Blue,
    Color::Color256(208),
    Color::Cyan,
    Color::Yellow,
    Color::Magenta,
    Color::Green,
    Color::Color256(99),
    Color::Red,
];

pub async fn run(ctx: &Ctx, args: MonitorArgs) -> Result<()> {
    if !args.watch {
        let v = ctx.api.system_metrics().await?;
        ctx.out.data(&v, |v| print(v, &Peaks::default()));
        return Ok(());
    }
    let every = Duration::from_secs(args.interval.max(1));
    let term = console::Term::stdout();
    let mut peaks = Peaks::default();
    loop {
        let v = ctx.api.system_metrics().await?;
        peaks.add(&v);
        if ctx.out.json {
            ctx.out.data(&v, |_| {});
        } else {
            let _ = term.clear_screen();
            print(&v, &peaks);
            println!();
            println!("{}", output::dim(&t!("monitor.watch_hint")));
        }
        tokio::select! {
            _ = tokio::time::sleep(every) => {}
            _ = tokio::signal::ctrl_c() => return Ok(()),
        }
    }
}

/// `--watch` 동안 본 최대값 (비율 막대의 눈금)
#[derive(Default)]
struct Peaks {
    disk_io: f64,
    network: f64,
}

impl Peaks {
    fn add(&mut self, v: &Value) {
        self.disk_io = self.disk_io.max(num(v, &["disk_io", "read_bps"]) + num(v, &["disk_io", "write_bps"]));
        self.network = self.network.max(num(v, &["network", "rx_bps"]) + num(v, &["network", "tx_bps"]));
    }
}

fn num(v: &Value, path: &[&str]) -> f64 {
    path.iter().try_fold(v, |cur, k| cur.get(*k)).and_then(Value::as_f64).unwrap_or(0.0)
}

/// 막대 한 줄의 조각: 인스턴스 번호(색), 나머지, 유휴
#[derive(Debug, Clone, Copy, PartialEq)]
enum Part {
    Instance(usize),
    Others,
    Idle,
}

/// 값들을 `width`칸에 나눈다 (가장 큰 나머지 반올림: 칸 합이 정확히 width)
fn split_cells(values: &[f64], total: f64, width: usize) -> Vec<usize> {
    if total <= 0.0 || values.is_empty() {
        return vec![0; values.len()];
    }
    let exact: Vec<f64> = values.iter().map(|v| v.max(0.0) / total * width as f64).collect();
    let mut cells: Vec<usize> = exact.iter().map(|e| e.floor() as usize).collect();
    let used: usize = cells.iter().sum();
    let mut order: Vec<usize> = (0..values.len()).collect();
    order.sort_by(|&a, &b| (exact[b] - exact[b].floor()).total_cmp(&(exact[a] - exact[a].floor())));
    for &i in order.iter().take(width.saturating_sub(used)) {
        cells[i] += 1;
    }
    cells
}

fn bar(parts: &[(Part, f64)], total: f64) -> String {
    let values: Vec<f64> = parts.iter().map(|(_, v)| *v).collect();
    let cells = split_cells(&values, total, BAR_WIDTH);
    let mut out = String::new();
    for ((part, _), n) in parts.iter().zip(cells) {
        let run = |c: char| c.to_string().repeat(n);
        out.push_str(&match part {
            Part::Instance(i) if *i < COLORS.len() => style(run('█')).fg(COLORS[*i]).to_string(),
            Part::Instance(_) => style(run('█')).white().to_string(),
            Part::Others => style(run('▓')).dim().to_string(),
            Part::Idle => style(run('░')).dim().to_string(),
        });
    }
    out
}

fn bytes(n: f64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"];
    let (mut v, mut i) = (n.max(0.0), 0);
    while v >= 1024.0 && i < units.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{} {}", v.round(), units[i])
    } else {
        format!("{:.*} {}", if v < 10.0 { 1 } else { 0 }, v, units[i])
    }
}

fn rate(n: f64) -> String {
    format!("{}/s", bytes(n))
}

fn print(v: &Value, peaks: &Peaks) {
    let insts: Vec<&Value> = v.get("instances").and_then(Value::as_array).map(|a| a.iter().collect()).unwrap_or_default();
    // 색 번호는 목록 순서: 꺼진 인스턴스가 있어도 색이 바뀌지 않는다
    let inst_parts = |field: &dyn Fn(&Value) -> f64| -> Vec<(Part, f64)> {
        insts
            .iter()
            .enumerate()
            .map(|(i, x)| (Part::Instance(i), field(x)))
            .filter(|(_, val)| *val > 0.0)
            .collect()
    };
    let mut rows: Vec<(String, String, String)> = Vec::new();

    let cpu_total = num(v, &["cpu", "total_percent"]);
    let mut parts = inst_parts(&|x| num(x, &["cpu_percent"]));
    parts.push((Part::Others, num(v, &["cpu", "others_percent"])));
    parts.push((Part::Idle, 100.0 - cpu_total));
    rows.push((t!("monitor.cpu"), bar(&parts, 100.0), format!("{:.0}%", cpu_total)));

    let (mem_total, mem_used) = (num(v, &["memory", "total_bytes"]), num(v, &["memory", "used_bytes"]));
    let mut parts = inst_parts(&|x| num(x, &["memory_bytes"]));
    parts.push((Part::Others, num(v, &["memory", "others_bytes"])));
    parts.push((Part::Idle, mem_total - mem_used));
    rows.push((t!("monitor.memory"), bar(&parts, mem_total), format!("{} / {}", bytes(mem_used), bytes(mem_total))));

    let io_total = num(v, &["disk_io", "read_bps"]) + num(v, &["disk_io", "write_bps"]);
    let io_scale = peaks.disk_io.max(io_total).max(MIN_RATE_SCALE);
    let mut parts = inst_parts(&|x| num(x, &["disk_read_bps"]) + num(x, &["disk_write_bps"]));
    parts.push((Part::Others, num(v, &["disk_io", "others_read_bps"]) + num(v, &["disk_io", "others_write_bps"])));
    parts.push((Part::Idle, io_scale - io_total));
    rows.push((
        t!("monitor.disk_io"),
        bar(&parts, io_scale),
        t!(
            "monitor.read_write",
            read = rate(num(v, &["disk_io", "read_bps"])),
            write = rate(num(v, &["disk_io", "write_bps"]))
        ),
    ));

    for disk in v.get("disks").and_then(Value::as_array).into_iter().flatten() {
        let total = num(disk, &["total_bytes"]);
        let mut parts: Vec<(Part, f64)> = Vec::new();
        for share in disk.get("instances").and_then(Value::as_array).into_iter().flatten() {
            let id = output::s(share, "id");
            if let Some(i) = insts.iter().position(|x| output::s(x, "id") == id) {
                parts.push((Part::Instance(i), num(share, &["bytes"])));
            }
        }
        parts.push((Part::Others, num(disk, &["others_bytes"])));
        parts.push((Part::Idle, num(disk, &["available_bytes"])));
        rows.push((
            format!("{} {}", t!("monitor.disk"), output::s(disk, "mount")),
            bar(&parts, total),
            t!("monitor.disk_free", free = bytes(num(disk, &["available_bytes"])), total = bytes(total)),
        ));
    }

    let (rx, tx) = (num(v, &["network", "rx_bps"]), num(v, &["network", "tx_bps"]));
    let net_scale = peaks.network.max(rx + tx).max(MIN_RATE_SCALE);
    // 네트워크는 인스턴스별로 못 잰다: 전체 한 조각
    let parts = [(Part::Others, rx + tx), (Part::Idle, net_scale - rx - tx)];
    rows.push((t!("monitor.network"), bar(&parts, net_scale), t!("monitor.rx_tx", rx = rate(rx), tx = rate(tx))));

    let mut table = output::table(&[String::new(), String::new(), String::new()]);
    for (name, b, value) in rows {
        table.add_row(vec![output::bold(&name), b, value]);
    }
    output::print_table(table);

    // 인스턴스 표: 색 견본이 막대 색의 범례다
    let running: Vec<(usize, &&Value)> = insts
        .iter()
        .enumerate()
        .filter(|(_, x)| x.get("running").and_then(Value::as_bool).unwrap_or(false))
        .collect();
    println!();
    if running.is_empty() {
        println!("{}", output::dim(&t!("monitor.none_running")));
    } else {
        let mut table = output::table(&[t!("col.name"), t!("monitor.cpu"), t!("monitor.memory"), t!("monitor.disk_io")]);
        for (i, x) in running {
            let swatch = match COLORS.get(i) {
                Some(c) => style("█").fg(*c).to_string(),
                None => style("█").white().to_string(),
            };
            table.add_row(vec![
                format!("{} {}", swatch, output::s(x, "name")),
                format!("{:.1}%", num(x, &["cpu_percent"])),
                bytes(num(x, &["memory_bytes"])),
                rate(num(x, &["disk_read_bps"]) + num(x, &["disk_write_bps"])),
            ]);
        }
        output::print_table(table);
    }
    println!(
        "{}",
        output::dim(&format!("{} ▓ {}  ░ {}", t!("monitor.legend"), t!("monitor.others"), t!("monitor.idle")))
    );
    if v.get("network").and_then(|n| n.get("per_instance")).and_then(Value::as_bool) != Some(true) {
        println!("{}", output::dim(&t!("monitor.network_note")));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cells_always_fill_the_bar_exactly() {
        assert_eq!(split_cells(&[30.0, 20.0, 50.0], 100.0, 30), vec![9, 6, 15]);
        // 반올림이 흩어져도 합은 폭과 같다
        let cells = split_cells(&[1.0, 1.0, 1.0], 3.0, 10);
        assert_eq!(cells.iter().sum::<usize>(), 10);
        assert_eq!(split_cells(&[5.0], 0.0, 30), vec![0]);
        // 음수(어긋난 표본)는 0으로 본다
        assert_eq!(split_cells(&[-1.0, 10.0], 10.0, 30), vec![0, 30]);
    }

    #[test]
    fn byte_units_read_like_the_gui() {
        assert_eq!(bytes(512.0), "512 B");
        assert_eq!(bytes(1536.0), "1.5 KB");
        assert_eq!(bytes(20.0 * 1024.0 * 1024.0 * 1024.0), "20 GB");
    }
}
