//! The folders the server may read from.
//!
//! Every path a client sends is resolved against them: relative paths start
//! at the first folder, the result is canonicalised (so `..` and symbolic
//! links cannot step outside) and must lie inside one of the folders.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Sandbox {
    roots: Vec<PathBuf>,
}

impl Sandbox {
    /// Canonicalise `roots`; each must be an existing directory.
    pub fn new(roots: &[PathBuf]) -> anyhow::Result<Self> {
        anyhow::ensure!(!roots.is_empty(), "no folder to serve");
        let roots = roots
            .iter()
            .map(|r| {
                let canon = r
                    .canonicalize()
                    .map_err(|e| anyhow::anyhow!("--root {}: {e}", r.display()))?;
                anyhow::ensure!(canon.is_dir(), "--root {} is not a folder", r.display());
                Ok(canon)
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        Ok(Sandbox { roots })
    }

    pub fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    /// An existing path inside the allowed folders.
    pub fn resolve(&self, path: &str) -> Result<PathBuf, String> {
        let given = Path::new(path);
        let joined = if given.is_absolute() {
            given.to_path_buf()
        } else {
            self.roots[0].join(given)
        };
        let canon = joined
            .canonicalize()
            .map_err(|e| format!("cannot open {path}: {e}"))?;
        if self.roots.iter().any(|r| canon.starts_with(r)) {
            Ok(canon)
        } else {
            Err(format!(
                "{path} is outside the folders this server may read ({})",
                self.describe()
            ))
        }
    }

    pub fn describe(&self) -> String {
        self.roots
            .iter()
            .map(|r| r.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_inside_resolve_and_outside_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let inner = dir.path().join("in");
        std::fs::create_dir(&inner).unwrap();
        std::fs::write(inner.join("a.hwpx"), b"x").unwrap();
        std::fs::write(dir.path().join("secret.hwpx"), b"x").unwrap();
        let sandbox = Sandbox::new(std::slice::from_ref(&inner)).unwrap();

        let ok = sandbox.resolve("a.hwpx").unwrap();
        assert_eq!(ok, inner.canonicalize().unwrap().join("a.hwpx"));
        assert_eq!(sandbox.resolve(ok.to_str().unwrap()).unwrap(), ok);

        let escape = sandbox.resolve("../secret.hwpx").unwrap_err();
        assert!(escape.contains("outside"), "{escape}");
        let absolute = dir.path().join("secret.hwpx");
        assert!(sandbox.resolve(absolute.to_str().unwrap()).is_err());
        assert!(sandbox
            .resolve("missing.hwpx")
            .unwrap_err()
            .contains("cannot open"));
    }

    #[cfg(unix)]
    #[test]
    fn symbolic_links_cannot_lead_outside() {
        let dir = tempfile::tempdir().unwrap();
        let inner = dir.path().join("in");
        std::fs::create_dir(&inner).unwrap();
        std::fs::write(dir.path().join("secret.hwpx"), b"x").unwrap();
        std::os::unix::fs::symlink(dir.path().join("secret.hwpx"), inner.join("link.hwpx"))
            .unwrap();
        let sandbox = Sandbox::new(&[inner]).unwrap();
        assert!(sandbox.resolve("link.hwpx").is_err());
    }
}
