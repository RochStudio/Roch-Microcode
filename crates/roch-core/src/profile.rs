//! Flash method and USB filename profiles per vendor.

use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize, Serializer};

use crate::error::require;
use crate::input::msi_name_regex;
use crate::Result;

pub const FLASH_BACK: &str = "FlashBack / BIOS button";
pub const M_FLASH: &str = "M-FLASH (inside BIOS)";
pub const VENDORS: [&str; 4] = ["ASUS", "MSI", "ASRock", "Gigabyte"];

/// Line break used by the multi-line texts. The original raw string literals
/// were compiled from CRLF sources on Windows, so packages contain CRLF.
const NL: &str = "\r\n";

fn default_method() -> Option<String> {
    // Missing fields in existing version 2 packages retain the original button workflow.
    Some(FLASH_BACK.to_string())
}

/// Target manufacturer, board, flash method and root filename.
///
/// Serialized exactly like the original record (PascalCase, including the
/// computed `GuideUrl`, `BoardInstructions` and `Instructions` properties).
/// A missing `Method` means the FlashBack / button workflow; an explicit
/// `null` is kept and rejected by [`FlashProfile::validate`].
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct FlashProfile {
    #[serde(default)]
    pub vendor: String,
    #[serde(default)]
    pub board: String,
    #[serde(default)]
    pub file_name: String,
    #[serde(default = "default_method")]
    pub method: Option<String>,
    #[serde(default)]
    pub original_file_name: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct FlashProfileJson<'a> {
    vendor: &'a str,
    board: &'a str,
    file_name: &'a str,
    method: Option<&'a str>,
    original_file_name: Option<&'a str>,
    guide_url: &'static str,
    board_instructions: &'static str,
    instructions: String,
}

impl Serialize for FlashProfile {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        FlashProfileJson {
            vendor: &self.vendor,
            board: &self.board,
            file_name: &self.file_name,
            method: self.method.as_deref(),
            original_file_name: self.original_file_name.as_deref(),
            guide_url: self.guide_url(),
            board_instructions: self.board_instructions(),
            instructions: self.instructions(),
        }
        .serialize(serializer)
    }
}

fn root_name_regex() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"^[A-Za-z0-9_-]{1,24}\.[A-Za-z0-9]{1,4}$").expect("valid regex"))
}

fn reserved_name_regex() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"(?i)^(CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])$").expect("valid regex"))
}

/// `Path.GetFileNameWithoutExtension` for a validated root name.
fn stem(name: &str) -> &str {
    match name.rfind('.') {
        Some(i) => &name[..i],
        None => name,
    }
}

impl FlashProfile {
    /// A FlashBack / button profile (the record's default method).
    pub fn new(vendor: impl Into<String>, board: impl Into<String>, file_name: impl Into<String>) -> Self {
        Self {
            vendor: vendor.into(),
            board: board.into(),
            file_name: file_name.into(),
            method: default_method(),
            original_file_name: None,
        }
    }

    pub fn with_method(mut self, method: &str) -> Self {
        self.method = Some(method.to_string());
        self
    }

    pub fn with_original(mut self, original: Option<&str>) -> Self {
        self.original_file_name = original.map(str::to_string);
        self
    }

    pub fn method_str(&self) -> &str {
        self.method.as_deref().unwrap_or("")
    }

    pub fn is_m_flash(&self) -> bool {
        self.method.as_deref() == Some(M_FLASH)
    }

    /// Supported flash methods; the first is the default.
    pub fn methods(vendor: &str) -> &'static [&'static str] {
        if vendor == "MSI" {
            &[M_FLASH, FLASH_BACK]
        } else {
            &[FLASH_BACK]
        }
    }

    pub fn suggested_name(vendor: &str, method: &str, original_name: Option<&str>, asus_name: Option<&str>) -> String {
        if method == M_FLASH {
            original_name.unwrap_or("").to_string()
        } else if vendor == "ASUS" {
            asus_name.unwrap_or("").to_string()
        } else {
            Self::default_name(vendor).to_string()
        }
    }

    pub fn default_name(vendor: &str) -> &'static str {
        match vendor {
            "MSI" => "MSI.ROM",
            "ASRock" => "CREATIVE.ROM",
            "Gigabyte" => "GIGABYTE.bin",
            _ => "",
        }
    }

    pub fn validate(&self) -> Result<()> {
        require(VENDORS.contains(&self.vendor.as_str()), "Choose ASUS, MSI, ASRock or Gigabyte.")?;
        require(
            self.method.as_deref().is_some_and(|m| Self::methods(&self.vendor).contains(&m)),
            "Choose a supported flash method for this manufacturer.",
        )?;
        require(
            !self.board.trim().is_empty()
                && self.board.encode_utf16().count() <= 120
                && !self.board.chars().any(char::is_control),
            "Enter the target motherboard model and revision.",
        )?;
        require(
            root_name_regex().is_match(&self.file_name) && !reserved_name_regex().is_match(stem(&self.file_name)),
            "Enter a valid root BIOS filename, without folders.",
        )?;
        if self.is_m_flash() {
            require(
                msi_name_regex().is_match(&self.file_name),
                "M-FLASH needs the original MSI BIOS filename. Load the original vendor BIOS or ZIP, not a renamed MSI.ROM.",
            )?;
            require(
                self.original_file_name.as_deref().is_some_and(|o| o.eq_ignore_ascii_case(&self.file_name)),
                "M-FLASH must preserve the NEW target BIOS filename, including its version extension.",
            )?;
        } else if self.vendor == "ASUS" {
            require(
                self.file_name.to_ascii_uppercase().ends_with(".CAP"),
                "ASUS FlashBack requires the model-specific .CAP name.",
            )?;
        } else {
            require(
                self.file_name.eq_ignore_ascii_case(Self::default_name(&self.vendor)),
                "The filename does not match this vendor's FlashBack naming rule.",
            )?;
        }
        Ok(())
    }

    pub fn guide_url(&self) -> &'static str {
        match self.vendor.as_str() {
            "MSI" => {
                if self.is_m_flash() {
                    "https://www.msi.com/support/technical_details/mb_bios_update"
                } else {
                    "https://www.msi.com/support/technical_details/MB_Flash_BIOS_Button"
                }
            }
            "ASRock" => "https://www.asrock.com/microsite/BIOSFlashback2026/",
            "Gigabyte" => "https://www.gigabyte.com/FileUpload/Global/KeyFeature/3798/index.html",
            _ => "https://www.asus.com/us/support/faq/1038568/",
        }
    }

    pub fn board_instructions(&self) -> &'static str {
        if self.is_m_flash() {
            "Use M-FLASH inside the BIOS with the original target filename. A visible file does not prove acceptance; M-FLASH may reject modified firmware. Keep the untouched BIOS separately. Do not interrupt power while flashing."
        } else {
            "Confirm FlashBack support, filename, USB port and power connections in the board manual. Keep a separate recovery drive. Do not interrupt power while flashing."
        }
    }

    /// Package `READ-ME.txt` instructions for this profile.
    pub fn instructions(&self) -> String {
        let m_flash = self.is_m_flash();
        let step2 = if m_flash {
            "Use M-FLASH inside the MSI BIOS. Preserve the original target BIOS filename; MSI.ROM is for the Flash BIOS Button."
        } else {
            "Your model must have a dedicated BIOS FlashBack / Flash BIOS Button / Q-Flash Plus feature. Confirm support in its manual."
        };
        let step6 = if m_flash {
            "Enter BIOS, open M-FLASH, and select the BIOS file on the USB drive. If rejected, stop; renaming cannot bypass signature checks."
        } else {
            "Follow the EXACT board manual for power connectors, dedicated USB port, button and LED behavior. Usually the PC is off with PSU power connected."
        };
        let lines = [
            format!("ROCH MICROCODE — {} {}", self.vendor, self.board),
            format!("Flash method: {}", self.method_str()),
            format!("USB root filename: {}", self.file_name),
            "Modified and recovery files use the same name. Keep them on separate labelled drives.".into(),
            String::new(),
            "1. Confirm the NEW BIOS belongs to the exact target motherboard and hardware revision.".into(),
            "   The donor may be from another board; it contributes only its compatible Intel microcode.".into(),
            "   Modified and recovery outputs both use the target board's newer firmware as their base.".into(),
            format!("2. {step2}"),
            "3. Use a FAT32 / MBR USB drive with one partition. Roch Microcode never formats it.".into(),
            format!(
                "4. Copy the chosen {} to the USB root. Keep the untouched new BIOS on a recovery drive.",
                self.file_name
            ),
            "5. Save settings; have the BitLocker recovery key and suspend protection if enabled.".into(),
            format!("6. {step6}"),
            "   Do not interrupt power during flashing.".into(),
            "7. After boot, confirm BIOS version and active microcode, then check stability before tuning.".into(),
            "   Resume BitLocker protection when complete.".into(),
            String::new(),
            format!("Vendor guide: {}", self.guide_url()),
            "ASUS: the FlashBack filename is model-specific. Check BIOSRenamer or the board manual.".into(),
            "Other vendor defaults can also vary by generation; the exact board manual takes precedence.".into(),
            String::new(),
            "Verification checks file structure and integrity, not boot or long-term stability.".into(),
            "Changing microcode can invalidate capsule/firmware signatures or measured-boot expectations.".into(),
            "Signature enforcement or rollback protection may reject this image. Roch Microcode does not bypass them."
                .into(),
            "Earlier microcode can omit later fixes, mitigations and extended CPU signatures.".into(),
            "Windows may load a different revision. Recovery is not guaranteed.".into(),
            "The app never flashes firmware, installs a driver or changes system settings.".into(),
        ];
        lines.join(NL)
    }
}

#[cfg(test)]
mod tests {
    //! Port of `tests/RochBios.ProfileTests` (fixture-free part).
    use super::*;

    fn mpower() -> FlashProfile {
        FlashProfile::new("MSI", "Z790 MPOWER", "E7E01IMS.PA0").with_method(M_FLASH).with_original(Some("E7E01IMS.PA0"))
    }

    fn rejected(p: &FlashProfile) -> bool {
        p.validate().is_err_and(|e| e.is_invalid_data())
    }

    #[test]
    fn m_flash_preserves_pa_extension() {
        mpower().validate().unwrap();
    }
    #[test]
    fn msi_defaults_to_m_flash() {
        assert_eq!(FlashProfile::methods("MSI")[0], M_FLASH);
        assert_eq!(FlashProfile::methods("ASUS"), &[FLASH_BACK]);
    }
    #[test]
    fn suggested_names() {
        assert_eq!(FlashProfile::suggested_name("MSI", M_FLASH, Some("E7E01IMS.PA0"), None), "E7E01IMS.PA0");
        assert_eq!(FlashProfile::suggested_name("MSI", FLASH_BACK, Some("E7E01IMS.PA0"), None), "MSI.ROM");
        assert_eq!(
            FlashProfile::suggested_name("ASUS", FLASH_BACK, Some("x.CAP"), Some("SZ790AD4.CAP")),
            "SZ790AD4.CAP"
        );
        assert_eq!(FlashProfile::suggested_name("ASUS", FLASH_BACK, None, None), "");
        assert_eq!(FlashProfile::suggested_name("ASRock", FLASH_BACK, None, None), "CREATIVE.ROM");
        assert_eq!(FlashProfile::suggested_name("Gigabyte", FLASH_BACK, None, None), "GIGABYTE.bin");
    }
    #[test]
    fn m_flash_rejections() {
        let p = mpower();
        assert!(
            rejected(&FlashProfile {
                file_name: "MSI.ROM".into(),
                original_file_name: Some("MSI.ROM".into()),
                ..p.clone()
            }),
            "button name"
        );
        assert!(rejected(&FlashProfile { file_name: "E7E07IMS.190".into(), ..p.clone() }), "donor filename");
        assert!(rejected(&FlashProfile { file_name: "E7E01IMS.P90".into(), ..p.clone() }), "wrong version");
        assert!(rejected(&FlashProfile { original_file_name: None, ..p.clone() }), "original required");
        assert!(rejected(&FlashProfile { file_name: "../E7E01IMS.PA0".into(), ..p.clone() }), "traversal");
        assert!(
            rejected(&FlashProfile { method: Some(FLASH_BACK.into()), ..p.clone() }),
            "button rejects M-FLASH name"
        );
        assert!(rejected(&FlashProfile { vendor: "ASUS".into(), ..p.clone() }), "other vendors reject M-FLASH");
        assert!(rejected(&FlashProfile { method: Some("unknown".into()), ..p.clone() }), "unknown method");
        assert!(rejected(&FlashProfile { method: None, ..p.clone() }), "null method");
    }
    #[test]
    fn m_flash_guide_and_instructions() {
        let p = mpower();
        assert!(p.guide_url().ends_with("mb_bios_update"));
        assert!(p.instructions().contains("Enter BIOS, open M-FLASH"));
        assert!(!p.instructions().contains("Usually the PC is off"));
    }
    #[test]
    fn json_preserves_method_and_original_filename() {
        let p = mpower();
        let text = crate::json::to_indented(&p).unwrap();
        let restored: FlashProfile = serde_json::from_str(&text).unwrap();
        restored.validate().unwrap();
        assert_eq!(restored, p);
        assert!(text.contains("\"GuideUrl\""));
        assert!(text.contains("\"Instructions\""));
    }
    #[test]
    fn legacy_json_keeps_button_semantics() {
        let restored: FlashProfile =
            serde_json::from_str(r#"{"Vendor":"MSI","Board":"Z790 MPOWER","FileName":"MSI.ROM"}"#).unwrap();
        restored.validate().unwrap();
        assert_eq!(restored.method.as_deref(), Some(FLASH_BACK));
        assert!(restored.guide_url().ends_with("MB_Flash_BIOS_Button"));
    }
    #[test]
    fn explicit_null_method_is_rejected() {
        let restored: FlashProfile =
            serde_json::from_str(r#"{"Vendor":"MSI","Board":"Z790 MPOWER","FileName":"MSI.ROM","Method":null}"#)
                .unwrap();
        assert!(rejected(&restored));
    }
    #[test]
    fn other_vendor_naming_unchanged() {
        for (vendor, name) in [("ASUS", "SZ790AD4.CAP"), ("ASRock", "CREATIVE.ROM"), ("Gigabyte", "GIGABYTE.bin")] {
            FlashProfile::new(vendor, "Test board", name).validate().unwrap();
        }
        FlashProfile::new("Gigabyte", "Test board", "gigabyte.BIN").validate().unwrap();
        assert!(rejected(&FlashProfile::new("ASRock", "Test board", "MSI.ROM")));
        assert!(rejected(&FlashProfile::new("ASUS", "Test board", "SZ790AD4.ROM")));
    }
    #[test]
    fn reserved_and_invalid_names_rejected() {
        assert!(rejected(&FlashProfile::new("ASUS", "Test", "COM9.CAP")));
        assert!(rejected(&FlashProfile::new("ASUS", "Test", "nul.CAP")));
        assert!(rejected(&FlashProfile::new("ASUS", "Test", "../other.CAP")));
        assert!(rejected(&FlashProfile::new("ASUS", "Test", "A.B.CAP")));
        assert!(rejected(&FlashProfile::new("ASUS", "Test", "ABCDEFGHIJKLMNOPQRSTUVWXY.CAP")));
        assert!(rejected(&FlashProfile::new("ASUS", "", "SZ790AD4.CAP")));
        assert!(rejected(&FlashProfile::new("ASUS", "   ", "SZ790AD4.CAP")));
        assert!(rejected(&FlashProfile::new("ASUS", "Bad\u{7}board", "SZ790AD4.CAP")));
        assert!(rejected(&FlashProfile::new("ASUS", "x".repeat(121), "SZ790AD4.CAP")));
        assert!(rejected(&FlashProfile::new("Unknown", "Test", "SZ790AD4.CAP")));
    }
}
