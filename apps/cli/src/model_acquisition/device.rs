// SPDX-License-Identifier: Apache-2.0
use serde_json::{json, Value};
#[cfg(windows)]
use std::path::PathBuf;
use std::{
    process::{Command, Stdio},
    sync::OnceLock,
    thread,
    time::{Duration, Instant},
};

/// Hardware probes may call platform utilities. Only filesystem capacity changes between plans.
pub(super) fn snapshot() -> Value {
    static DEVICE: OnceLock<Value> = OnceLock::new();
    let mut value = DEVICE
        .get_or_init(|| {
            let (cpu, gpu, vram, unified) = inventory();
            json!({"cpu":cpu,"cores":std::thread::available_parallelism().ok().map(|n| n.get()),
            "memoryBytes":crate::local_models::physical_memory_bytes(),"gpu":gpu,
            "gpuMemoryBytes":vram,"unifiedMemory":unified})
        })
        .clone();
    value["freeDiskBytes"] = json!(super::root_path().ok().and_then(|path| {
        path.ancestors()
            .find(|p| p.is_dir())
            .and_then(|p| super::storage::free_space(p).ok())
    }));
    value
}
type Inventory = (Option<String>, Option<String>, Option<u64>, bool);

fn output(program: &str, args: &[&str]) -> Option<String> {
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    let mut child = cmd
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + Duration::from_secs(4);
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    String::from_utf8(child.wait_with_output().ok()?.stdout)
        .ok()
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
}

#[cfg(target_os = "macos")]
fn inventory() -> Inventory {
    let cpu = output("/usr/sbin/sysctl", &["-n", "machdep.cpu.brand_string"]);
    let gpu = output(
        "/usr/sbin/system_profiler",
        &["SPDisplaysDataType", "-json"],
    )
    .and_then(|s| {
        let v: Value = serde_json::from_str(&s).ok()?;
        v["SPDisplaysDataType"].as_array()?.first()?["_name"]
            .as_str()
            .map(str::to_owned)
    });
    (cpu, gpu, None, cfg!(target_arch = "aarch64"))
}

#[cfg(not(target_os = "macos"))]
fn nvidia() -> Option<(String, u64)> {
    #[cfg(windows)]
    let path = "nvidia-smi.exe";
    #[cfg(not(windows))]
    let path = "nvidia-smi";
    output(
        path,
        &[
            "--query-gpu=name,memory.total",
            "--format=csv,noheader,nounits",
        ],
    )
    .and_then(|v| {
        v.lines()
            .filter_map(|line| {
                let (name, mib) = line.rsplit_once(',')?;
                Some((
                    name.trim().to_string(),
                    mib.trim().parse::<u64>().ok()?.checked_mul(1024 * 1024)?,
                ))
            })
            .max_by_key(|(_, size)| *size)
    })
}

#[cfg(windows)]
fn inventory() -> Inventory {
    // AdapterRAM is a 32-bit WMI property. Never interpret it as total VRAM on modern boards.
    let powershell = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .map(|p| p.join("System32/WindowsPowerShell/v1.0/powershell.exe"));
    const INVENTORY: &str = r#"
[Console]::OutputEncoding=[Text.UTF8Encoding]::new()
$cards = @(Get-CimInstance Win32_VideoController | ForEach-Object {
    $instance = Get-ItemProperty -LiteralPath ("HKLM:\SYSTEM\CurrentControlSet\Enum\" + $_.PNPDeviceID) -ErrorAction SilentlyContinue
    $driver = Get-ItemProperty -LiteralPath ("HKLM:\SYSTEM\CurrentControlSet\Control\Class\" + $instance.Driver) -ErrorAction SilentlyContinue
    $memory = $driver.'HardwareInformation.qwMemorySize'
    [PSCustomObject]@{name=$_.Name;bytes=if($memory -is [long] -and $memory -gt 0){$memory}else{$null}}
})
$card = $cards | Sort-Object bytes -Descending | Select-Object -First 1
@{cpu=(Get-CimInstance Win32_Processor | Select-Object -First 1 -ExpandProperty Name);gpu=$card.name;vram=$card.bytes} | ConvertTo-Json -Compress
"#;
    let info = powershell
        .as_ref()
        .and_then(|p| p.to_str())
        .and_then(|p| output(p, &["-NoProfile", "-NonInteractive", "-Command", INVENTORY]))
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .unwrap_or(Value::Null);
    let cpu = info["cpu"]
        .as_str()
        .map(str::to_owned)
        .or_else(|| std::env::var("PROCESSOR_IDENTIFIER").ok());
    if let Some((gpu, bytes)) = nvidia() {
        return (cpu, Some(gpu), Some(bytes), false);
    }
    (
        cpu,
        info["gpu"].as_str().map(str::to_owned),
        info["vram"].as_u64(),
        false,
    )
}
#[cfg(not(any(windows, target_os = "macos")))]
fn inventory() -> Inventory {
    let cpu = std::fs::read_to_string("/proc/cpuinfo").ok().and_then(|s| {
        s.lines().find_map(|l| {
            let (key, v) = l.split_once(':')?;
            matches!(key.trim(), "model name" | "Hardware").then(|| v.trim().to_owned())
        })
    });
    if let Some((gpu, bytes)) = nvidia() {
        return (cpu, Some(gpu), Some(bytes), false);
    }
    (cpu, None, None, false)
}
