//! Pinned official ASUS reference downloads (used only by the online regression tests).

use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::builder::write_new_verified;
use crate::firmware::{self, read_image, recipe::DownloadSpec};
use crate::format::guid_n;
use crate::{report, CancelToken, Error, Progress, Result};

const CAP_SIZE: u64 = 33_558_528;

struct Cleanup(Vec<PathBuf>);
impl Drop for Cleanup {
    fn drop(&mut self) {
        for p in &self.0 {
            if p.exists() {
                let _ = std::fs::remove_file(p);
            }
        }
    }
}

/// Return a verified cached CAP, or download the pinned ZIP, verify its
/// SHA-256, extract the CAP, verify that too and replace the cache atomically.
pub fn fetch(
    spec: &DownloadSpec,
    cache: impl AsRef<Path>,
    progress: Progress<'_>,
    ct: &CancelToken,
) -> Result<PathBuf> {
    ct.check()?;
    let cache = cache.as_ref();
    std::fs::create_dir_all(cache)?;
    let cap = cache.join(spec.cap_name());
    if cap.exists() {
        match read_image(&cap) {
            Ok(bytes) if firmware::hash(&bytes) == spec.cap_hash => {
                report(progress, &format!("BIOS {}: verified cached file", spec.version));
                return Ok(cap);
            }
            // A truncated cache is replaced only after a fresh download verifies.
            Ok(_) | Err(Error::InvalidData(_)) => {}
            Err(e) => return Err(e),
        }
        report(progress, &format!("BIOS {}: cached file needs replacement", spec.version));
    }
    let zip_path = cache.join(format!("asus-{}-{}.download", spec.version, guid_n()));
    let cap_temp = cache.join(format!("{}.cap.tmp", guid_n()));
    let _cleanup = Cleanup(vec![zip_path.clone(), cap_temp.clone()]);

    let agent: ureq::Agent = ureq::Agent::config_builder()
        .max_redirects(0)
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(240)))
        .build()
        .into();
    let mut response = agent.get(&spec.url()).call().map_err(|e| Error::io(format!("Download failed: {e}")))?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(Error::io(format!("Download failed: HTTP {status}")));
    }
    {
        let mut input = response.body_mut().with_config().limit(u64::MAX).reader();
        let mut output = OpenOptions::new().write(true).create_new(true).open(&zip_path)?;
        let mut buffer = vec![0u8; 65536];
        let mut total: u64 = 0;
        loop {
            ct.check()?;
            let n = input.read(&mut buffer).map_err(|e| Error::io(format!("Download failed: {e}")))?;
            if n == 0 {
                break;
            }
            total += n as u64;
            if total > 64 * 1024 * 1024 {
                return Err(Error::invalid("Download exceeded the expected size limit."));
            }
            output.write_all(&buffer[..n])?;
            report(progress, &format!("Downloading BIOS {} • {:.1} MB", spec.version, total as f64 / 1_048_576.0));
        }
    }
    ct.check()?;
    if firmware::hash(&std::fs::read(&zip_path)?) != spec.zip_hash {
        return Err(Error::invalid(format!(
            "BIOS {} ZIP checksum does not match the pinned ASUS package. No file was accepted.",
            spec.version
        )));
    }
    let mut zip = zip::ZipArchive::new(std::fs::File::open(&zip_path)?)
        .map_err(|e| Error::invalid(format!("The ASUS package could not be read: {e}")))?;
    let name = spec.cap_name();
    let matches: Vec<usize> = (0..zip.len())
        .filter(|&i| zip.by_index_raw(i).map(|e| e.name().eq_ignore_ascii_case(&name)).unwrap_or(false))
        .collect();
    if matches.len() != 1 {
        return Err(Error::invalid("Expected CAP not found in the ASUS package."));
    }
    let entry = zip.by_index(matches[0]).map_err(|e| Error::invalid(e.to_string()))?;
    if entry.size() != CAP_SIZE {
        return Err(Error::invalid("Unexpected CAP size."));
    }
    let mut bytes = Vec::with_capacity(CAP_SIZE as usize);
    let mut limited = entry.take(CAP_SIZE + 1);
    let mut chunk = vec![0u8; 65536];
    loop {
        ct.check()?;
        let n = limited.read(&mut chunk).map_err(|e| Error::invalid(e.to_string()))?;
        if n == 0 {
            break;
        }
        if bytes.len() as u64 + n as u64 > CAP_SIZE {
            return Err(Error::invalid("CAP exceeds expected size."));
        }
        bytes.extend_from_slice(&chunk[..n]);
    }
    if firmware::hash(&bytes) != spec.cap_hash {
        return Err(Error::invalid("Extracted CAP hash is incorrect."));
    }
    ct.check()?;
    write_new_verified(&cap_temp, &bytes, &spec.cap_hash)?;
    std::fs::rename(&cap_temp, &cap)?;
    report(progress, &format!("BIOS {} downloaded and SHA-256 verified", spec.version));
    Ok(cap)
}
