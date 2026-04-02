use crate::DRI;
use anyhow::{anyhow, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct GpuTelemetry {
    gpu_name: String,
    hwmon_path: PathBuf,
    pub data: TelemetryData,
}

#[derive(Default)]
pub struct TelemetryData {
    // HWMON Info
    pub edge_temp_c: f64,
    pub power_avg_w: f64,
    pub sclk_mhz: u64,
    pub vddgfx_mv: u64,
    pub vddnb_mv: Option<u64>,
    pub junction_temp_c: Option<f64>,
    pub memory_temp_c: Option<f64>,
    pub fan_rpm: Option<u64>,

    // DRI Info
    pub soc_wattage: f64,
    pub gpu_load_pct: u64,
    pub vcn_load_pct: u64,

    // GEM-Sourced
    pub gem_clients: Vec<GemClient>,

    // Could be in either depending on hardware
    pub mclk_mhz: u64,

    // Error collector
    pub last_errors: Vec<String>,
}

pub struct GemClient {
    pub pid: u32,
    pub command: String,
}

impl GpuTelemetry {
    pub fn init() -> Result<Self> {
        let path = find_amd_card()?;
        let gpu_name = get_gpu_name(&path)?;
        let path = path.join("device/hwmon");

        let dir = path.read_dir()
            .with_context(|| format!("Failed to read directory {}", path.display()))?;

        let hwmon_path = dir
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with("hwmon"))
                    .unwrap_or(false)
            })
            .ok_or_else(|| anyhow!("No hwmon directory found in {}", path.display()))?;

        Ok(Self {
            gpu_name,
            hwmon_path,
            data: TelemetryData::default()
        })
    }

    pub fn collect_hwmon(&mut self) -> Result<()> {
        // Hard fails
        self.data.edge_temp_c = try_read_hwmon_u64("temp1_input", &self.hwmon_path)
            .ok_or_else(|| anyhow!("temp1_input not found"))?  as f64 / 1000.0;
        self.data.power_avg_w = try_read_hwmon_u64("power1_average", &self.hwmon_path)
            .or_else(|| try_read_hwmon_u64("power1_input", &self.hwmon_path))
            .ok_or_else(|| anyhow!("No power sensor found"))? as f64 / 1_000_000.0;
        self.data.sclk_mhz = try_read_hwmon_u64("freq1_input", &self.hwmon_path)
            .ok_or_else(|| anyhow!("freq1_input not found"))? / 1_000_000;
        self.data.vddgfx_mv = try_read_hwmon_u64("in0_input", &self.hwmon_path)
            .ok_or_else(|| anyhow!("in0_input not found"))?;

        // Soft fails
        self.data.vddnb_mv = try_read_hwmon_u64("in1_input", &self.hwmon_path);
        self.data.junction_temp_c = try_read_hwmon_u64("temp2_input", &self.hwmon_path)
            .map(|v| v as f64 / 1000.0);
        self.data.memory_temp_c = try_read_hwmon_u64("temp3_input", &self.hwmon_path)
            .map(|v| v as f64 / 1000.0);
        self.data.fan_rpm = try_read_hwmon_u64("fan1_input", &self.hwmon_path);

        Ok(())
    }

    pub fn collect_pm_info(&mut self) -> Result<()> {
        let path = format!("{}/amdgpu_pm_info", DRI);
        let content = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read {}", path))?;

        for line in content.lines() {
            let trimmed = line.trim();
            match trimmed {
                t if t.ends_with("(SCLK)") => {
                    // pm_info SCLK as cross-check
                }
                t if t.contains("current SoC including CPU") => {
                    if let Some(v) = parse_watts(t) {
                        self.data.soc_wattage = v;
                    }
                }
                t if t.starts_with("GPU Load:") => {
                    if let Some(v) = parse_pct(t.trim_start_matches("GPU Load:").trim()) {
                        self.data.gpu_load_pct = v;
                    }
                }
                t if t.starts_with("VCN Load:") => {
                    if let Some(v) = parse_pct(t.trim_start_matches("VCN Load:").trim()) {
                        self.data.vcn_load_pct = v;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    pub fn collect_mclk(&mut self) -> Result<()> {
        self.data.mclk_mhz = match try_read_hwmon_u64("freq2_input", &self.hwmon_path) {
            Some(v) => v / 1_000_000,
            None => collect_mclk_from_dri()?,
        };
        Ok(())
    }

    pub fn collect_gem_info(&mut self) -> Result<()> {
        let path = format!("{}/amdgpu_gem_info", DRI);
        let content = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read {}", path))?;

        self.data.gem_clients.clear();
        for line in content.lines() {
            let trimmed = line.trim();

            if trimmed.starts_with("pid") {
                let mut parts = trimmed.split_whitespace();

                let label   = parts.next();
                let pid_str = parts.next();
                let _skip   = parts.next();
                let cmd     = parts.next();

                if label == Some("pid") {
                    if let (Some(pid_str), Some(cmd)) = (pid_str, cmd) {
                        if let Ok(pid) = pid_str.parse::<u32>() {

                            self.data.gem_clients.push(GemClient {
                                pid,
                                command: cmd.trim_end_matches(':').to_string(),
                            });
                        }
                    }
                }
            }
        }
        Ok(())
    }

    pub fn get_gpu_name(&self) -> &String {
        &self.gpu_name
    }
}

fn find_amd_card() -> Result<PathBuf> {
    let base = Path::new("/sys/class/drm");

    let entries = base.read_dir()
        .with_context(|| format!("Failed to read directory {}", base.display()))?;

    let cards = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_str()?.to_owned();
            let suffix = name.strip_prefix("card")?;

            if suffix.chars().all(|c| c.is_ascii_digit()) {
                Some(entry.path())
            } else {
                None
            }
        });

    let mut amd_cards: Vec<(PathBuf, u64)> = Vec::new();
    // Check each card to see if it's AMD (vendor 0x1002)
    for path in cards {
        let vendor_path = path.join("device/vendor");

        let vendor = fs::read_to_string(&vendor_path)
            .with_context(|| format!("Failed to read {}", vendor_path.display()))?;

        if vendor.trim().eq("0x1002") {
            let vram_path = path.join("device/mem_info_vram_total");
            let vram = fs::read_to_string(&vram_path)
                .with_context(|| format!("Failed to get vram size {}", vram_path.display()))?;
            let vram = vram.trim().parse::<u64>()?;
            amd_cards.push((path, vram));
        }
    }

    amd_cards.into_iter()
        .max_by_key(|(_, vram)| *vram)
        .map(|(path, _)| path)
        .ok_or_else(|| anyhow!("No AMD GPU found in /sys/class/drm"))
}

fn get_gpu_name(path: &Path) -> Result<String> {
    let uevent_path = path.join("device/uevent");

    let contents = fs::read_to_string(&uevent_path)
        .with_context(|| format!("Failed to read uevent {}", uevent_path.display()))?;

    let slot = contents
        .lines()
        .find_map(|line| line.strip_prefix("PCI_SLOT_NAME="))
        .ok_or_else(|| anyhow!("PCI_SLOT_NAME not found in {}", uevent_path.display()))?;

    let short_slot = slot.splitn(2, ':').nth(1).unwrap_or(slot);

    let output = Command::new("lspci")
        .arg("-s")
        .arg(short_slot)
        .output()
        .with_context(|| format!("Failed to run lspci for slot {}", short_slot))?;

    let stdout = String::from_utf8(output.stdout)
        .with_context(|| anyhow!("lspci output was not valid UTF‑8"))?;

    let name = stdout
        .splitn(3, ':')
        .nth(2)
        .map(|s| s.trim().to_string())
        .ok_or_else(|| anyhow!("Failed to parse lspci output: {}", stdout))?;

    let short_name = name
        .rfind('[')
        .and_then(|start| name.rfind(']').map(|end| &name[start+1..end]))
        .unwrap_or(&name)
        .to_string();

    Ok(short_name)
}

fn try_read_hwmon_u64(file: &str, path: &Path) -> Option<u64> {
    let full_path = path.join(file);
    let raw = fs::read_to_string(full_path).ok()?;
    raw.trim().parse().ok()
}
fn parse_mhz(line: &str) -> Option<u64> {
    line.split_whitespace()
        .next()?
        .parse::<u64>()
        .ok()
}

fn parse_watts(line: &str) -> Option<f64> {
    let val: f64 = line.split_whitespace().next()?.parse().ok()?;
    Some(val)
}

fn parse_pct(line: &str) -> Option<u64> {
    line.split_whitespace()
        .next()?
        .parse::<u64>()
        .ok()
}

fn collect_mclk_from_dri() -> Result<u64> {
    let path = format!("{}/amdgpu_pm_info", DRI);
    let content = fs::read_to_string(&path)
        .with_context(|| format!("Failed to read {}", path))?;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.ends_with("(MCLK)") {
            return parse_mhz(trimmed)
                .ok_or_else(|| anyhow!("Failed to parse MCLK line: {}", trimmed));
        }
    }
    Err(anyhow!("MCLK not found in amdgpu_pm_info"))
}