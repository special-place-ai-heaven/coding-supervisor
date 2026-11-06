//! G23: `npm install -g` with a Claude session open can rewrite the npm packages while the
//! running, locked `supervisor.exe` stays the old version, so the session runs an old gate.

use std::path::Path;

use serde_json::Value;

/// A warning when an npm package around `exe` (`<launcher>/node_modules/<platform>/bin/exe`)
/// records a version other than `running`. `None` outside an npm install.
pub fn package_mismatch(exe: &Path, running: &str) -> Option<String> {
    let platform = exe.parent()?.parent()?;
    let launcher = platform.parent()?.parent();
    [Some(platform), launcher]
        .into_iter()
        .flatten()
        .find_map(|dir| {
            let manifest: Value =
                serde_json::from_slice(&std::fs::read(dir.join("package.json")).ok()?).ok()?;
            let name = manifest["name"].as_str()?;
            let version = manifest["version"].as_str()?;
            (name.starts_with("coding-supervisor") && version != running).then(|| {
                format!(
                    "SUPERVISOR WARNING: the running supervisor binary is {running} but the installed package {name} is {version}; close every Claude session, reinstall, then check `supervisor --version`."
                )
            })
        })
}

/// [`package_mismatch`] for this process.
pub fn current_mismatch() -> Option<String> {
    package_mismatch(&std::env::current_exe().ok()?, env!("CARGO_PKG_VERSION"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn install(launcher: &str, platform: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "supervisor-version-{}-{launcher}-{platform}",
            std::process::id()
        ));
        let pkg = root.join("coding-supervisor");
        let plat = pkg.join("node_modules/coding-supervisor-windows-x64");
        std::fs::create_dir_all(plat.join("bin")).unwrap();
        std::fs::write(
            pkg.join("package.json"),
            format!(r#"{{"name":"coding-supervisor","version":"{launcher}"}}"#),
        )
        .unwrap();
        std::fs::write(
            plat.join("package.json"),
            format!(r#"{{"name":"coding-supervisor-windows-x64","version":"{platform}"}}"#),
        )
        .unwrap();
        (root, plat.join("bin/supervisor.exe"))
    }

    #[test]
    fn a_rewritten_package_around_an_old_binary_is_reported() {
        let (root, exe) = install("0.12.0", "0.11.1");
        let warning = package_mismatch(&exe, "0.11.1").unwrap();
        assert!(warning.contains("0.12.0") && warning.contains("close every Claude session"));
        let (root2, exe2) = install("0.11.1", "0.12.0");
        assert!(
            package_mismatch(&exe2, "0.11.1")
                .unwrap()
                .contains("0.12.0")
        );
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(root2);
    }

    #[test]
    fn matching_packages_or_no_npm_install_are_silent() {
        let (root, exe) = install("0.11.1", "0.11.1");
        assert_eq!(package_mismatch(&exe, "0.11.1"), None);
        assert_eq!(
            package_mismatch(&root.join("a/b/c/d/x.exe"), "0.11.1"),
            None
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
