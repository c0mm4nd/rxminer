use std::{fs, path::Path};

// A desktop without a battery remains active. Only an explicit discharging
// battery without an online external supply causes a pause.
pub fn on_battery() -> bool {
    on_battery_at(Path::new("/sys/class/power_supply"))
}
fn on_battery_at(root: &Path) -> bool {
    let Ok(entries) = fs::read_dir(root) else {
        return false;
    };
    let mut discharging = false;
    for entry in entries.flatten() {
        let path = entry.path();
        let kind = fs::read_to_string(path.join("type")).unwrap_or_default();
        if kind.trim() == "Battery" {
            discharging |=
                fs::read_to_string(path.join("status")).is_ok_and(|s| s.trim() == "Discharging");
        } else if fs::read_to_string(path.join("online")).is_ok_and(|s| s.trim() == "1") {
            return false;
        }
    }
    discharging
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ac_connection_overrides_discharging_battery() {
        let root = std::env::temp_dir().join(format!("rxminer-power-test-{}", std::process::id()));
        fs::create_dir_all(root.join("battery")).unwrap();
        fs::create_dir_all(root.join("ac")).unwrap();
        fs::write(root.join("battery/type"), "Battery\n").unwrap();
        fs::write(root.join("battery/status"), "Discharging\n").unwrap();
        assert!(on_battery_at(&root));
        fs::write(root.join("ac/type"), "Mains\n").unwrap();
        fs::write(root.join("ac/online"), "1\n").unwrap();
        assert!(!on_battery_at(&root));
        fs::write(root.join("ac/online"), "0\n").unwrap();
        assert!(on_battery_at(&root));
        fs::write(root.join("battery/status"), "Charging\n").unwrap();
        assert!(!on_battery_at(&root));
        fs::remove_dir_all(&root).unwrap();
        assert!(!on_battery_at(&root));
    }
}
