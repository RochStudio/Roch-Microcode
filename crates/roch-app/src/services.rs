//! Windows inventory: USB volumes (Storage module cmdlets) and system information.
//!
//! The USB inventory deliberately uses the same `Get-Disk` / `Get-Partition` /
//! `Get-Volume` queries as the original app, so bus type, boot/system flags,
//! partition style, partition count and the disk/volume unique IDs used for the
//! identity re-checks have exactly the same meaning. The query is read-only.

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use roch_core::usb::UsbDrive;
use roch_core::{Error, Result};

use crate::platform;

fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n =
            (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

/// Run a script with Windows PowerShell 5.1, hidden, with a 30 second limit.
fn powershell(script: &str) -> Result<String> {
    let exe = platform::system_directory().join(r"WindowsPowerShell\v1.0\powershell.exe");
    let full = format!("$ErrorActionPreference='Stop'; [Console]::OutputEncoding=[Text.Encoding]::UTF8; {script}");
    let utf16: Vec<u8> = full.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    let mut command = Command::new(exe);
    command
        .args(["-NoProfile", "-NonInteractive", "-EncodedCommand", &base64(&utf16)])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = command.spawn().map_err(|_| Error::io("Could not start Windows inventory."))?;
    let mut stdout = child.stdout.take().expect("stdout");
    let mut stderr = child.stderr.take().expect("stderr");
    let out = std::thread::spawn(move || {
        let mut v = Vec::new();
        let _ = stdout.read_to_end(&mut v);
        v
    });
    let err = std::thread::spawn(move || {
        let mut v = Vec::new();
        let _ = stderr.read_to_end(&mut v);
        v
    });
    let started = Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait()? {
            break s;
        }
        if started.elapsed() > Duration::from_secs(30) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::io("Windows drive inventory timed out. Try Refresh."));
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let output = String::from_utf8_lossy(&out.join().unwrap_or_default()).into_owned();
    if !status.success() {
        let error = String::from_utf8_lossy(&err.join().unwrap_or_default()).into_owned();
        return Err(Error::io(format!("Windows inventory failed: {error}")));
    }
    Ok(output.trim_matches(['\u{feff}', ' ', '\r', '\n']).to_string())
}

const DRIVES_SCRIPT: &str = r#"
$rows = @(foreach ($d in (Get-Disk | Where-Object BusType -eq USB)) {
  $parts = @(Get-Partition -DiskNumber $d.Number)
  foreach ($p in $parts) {
    if (!$p.DriveLetter) { continue }
    $v = $p | Get-Volume
    [pscustomobject]@{
      DiskNumber=[int]$d.Number; UniqueId=[string]$d.UniqueId; Root=([string]$p.DriveLetter + ':\');
      Label=[string]$v.FileSystemLabel; Bus=[string]$d.BusType; PartitionStyle=[string]$d.PartitionStyle;
      FileSystem=[string]$v.FileSystem; PartitionCount=$parts.Count;
      IsBoot=[bool]($d.IsBoot -or $p.IsBoot); IsSystem=[bool]($d.IsSystem -or $p.IsSystem);
      Size=[long]$d.Size; Free=[long]$v.SizeRemaining; Offset=[long]$p.Offset; VolumeId=[string]$v.UniqueId
    }
  }
})
ConvertTo-Json -InputObject $rows -Compress
"#;

/// Mounted volumes on USB disks. Read-only.
pub fn drives() -> Result<Vec<UsbDrive>> {
    let json = powershell(DRIVES_SCRIPT)?;
    if json.is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str(&json).map_err(|e| Error::io(format!("Windows inventory returned unexpected data: {e}")))
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
struct SystemRow {
    board: Option<String>,
    bios: Option<String>,
    cpu: Option<String>,
}

/// "This computer" summary for the Inspect page.
pub fn system_info() -> Result<String> {
    let data = powershell(
        "$b=Get-CimInstance Win32_BaseBoard; $f=Get-CimInstance Win32_BIOS; $c=Get-CimInstance Win32_Processor | Select-Object -First 1; [pscustomobject]@{Board=$b.Product;Bios=$f.SMBIOSBIOSVersion;Cpu=$c.Name} | ConvertTo-Json -Compress",
    )?;
    let row: SystemRow = serde_json::from_str(&data).map_err(|e| Error::io(e.to_string()))?;
    let mc = platform::windows_microcode_revision().map(|r| format!("0x{r:X}")).unwrap_or_else(|| "unavailable".into());
    let signature = platform::cpuid_signature();
    let cpuid = if signature != 0 { format!(" • CPUID {signature:X}") } else { String::new() };
    let board = row.board.unwrap_or_else(|| "Unknown board".into());
    Ok(format!(
        "{board} • BIOS {}\n{}{cpuid}\nWindows-reported microcode: {mc}\nThe build target is determined by the BIOS you supply, not by this computer.",
        row.bios.unwrap_or_default(),
        row.cpu.unwrap_or_default()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }
}
