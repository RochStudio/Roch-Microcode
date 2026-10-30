//! Port of `tests/RochBios.ProfileTests`: flash method and filename rules.
//!
//! Usage: `roch-profile-tests [existing-MSI-button-package-folder]`

use roch_core::package::TransferPackage;
use roch_core::profile::{FlashProfile, FLASH_BACK, M_FLASH};
use roch_fixture_tests::{ensure, ok, reject_invalid, Suite};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut s = Suite::default();
    let profile = FlashProfile::new("MSI", "Z790 MPOWER", "E7E01IMS.PA0")
        .with_method(M_FLASH)
        .with_original(Some("E7E01IMS.PA0"));
    let with = |f: &dyn Fn(&mut FlashProfile)| {
        let mut p = profile.clone();
        f(&mut p);
        p
    };
    s.check("M-FLASH preserves PA extension", || ok(profile.validate()));
    s.check("MSI defaults to M-FLASH", || ensure(FlashProfile::methods("MSI")[0] == M_FLASH));
    s.check("target filename suggested", || {
        ensure(FlashProfile::suggested_name("MSI", M_FLASH, Some("E7E01IMS.PA0"), None) == "E7E01IMS.PA0")
    });
    s.check("button retains MSI.ROM", || {
        ensure(FlashProfile::suggested_name("MSI", FLASH_BACK, Some("E7E01IMS.PA0"), None) == "MSI.ROM")
    });
    s.check("M-FLASH rejects button name", || {
        reject_invalid(
            with(&|p| {
                p.file_name = "MSI.ROM".into();
                p.original_file_name = Some("MSI.ROM".into());
            })
            .validate(),
        )
    });
    s.check("M-FLASH rejects donor filename", || {
        reject_invalid(with(&|p| p.file_name = "E7E07IMS.190".into()).validate())
    });
    s.check("M-FLASH rejects wrong version", || {
        reject_invalid(with(&|p| p.file_name = "E7E01IMS.P90".into()).validate())
    });
    s.check("M-FLASH requires original name", || reject_invalid(with(&|p| p.original_file_name = None).validate()));
    s.check("M-FLASH rejects traversal", || {
        reject_invalid(with(&|p| p.file_name = "../E7E01IMS.PA0".into()).validate())
    });
    s.check("button rejects M-FLASH name", || reject_invalid(with(&|p| p.method = Some(FLASH_BACK.into())).validate()));
    s.check("other vendors reject M-FLASH", || reject_invalid(with(&|p| p.vendor = "ASUS".into()).validate()));
    s.check("unknown method rejected", || reject_invalid(with(&|p| p.method = Some("unknown".into())).validate()));
    s.check("null method rejected", || reject_invalid(with(&|p| p.method = None).validate()));
    s.check("M-FLASH guide and instructions", || {
        ensure(
            profile.guide_url().ends_with("mb_bios_update")
                && profile.instructions().contains("Enter BIOS, open M-FLASH")
                && !profile.instructions().contains("Usually the PC is off"),
        )
    });
    s.check("JSON preserves method and original filename", || {
        let text = ok(roch_core::json::to_indented(&profile))?;
        let restored: FlashProfile = serde_json_from_str(&text)?;
        ok(restored.validate())?;
        ensure(restored == profile)
    });
    s.check("legacy JSON keeps button semantics", || {
        let restored: FlashProfile =
            serde_json_from_str(r#"{"Vendor":"MSI","Board":"Z790 MPOWER","FileName":"MSI.ROM"}"#)?;
        ok(restored.validate())?;
        ensure(restored.method.as_deref() == Some(FLASH_BACK) && restored.guide_url().ends_with("MB_Flash_BIOS_Button"))
    });
    for (vendor, name) in [("ASUS", "SZ790AD4.CAP"), ("ASRock", "CREATIVE.ROM"), ("Gigabyte", "GIGABYTE.bin")] {
        s.check(format!("{vendor} naming unchanged"), || ok(FlashProfile::new(vendor, "Test board", name).validate()));
    }
    if let Some(path) = args.first() {
        s.check("existing MSI package still revalidates", || {
            let package = ok(TransferPackage::open(path))?;
            ensure(
                package.manifest.profile.method.as_deref() == Some(FLASH_BACK)
                    && package.candidate_path().file_name().is_some_and(|n| n == "MSI.ROM"),
            )
        });
    }
    println!("{} profile checks passed.", s.passed);
}

fn serde_json_from_str<T: serde::de::DeserializeOwned>(text: &str) -> Result<T, String> {
    serde_json::from_str(text).map_err(|e| e.to_string())
}
