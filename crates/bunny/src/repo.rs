//! What's available to install: the package repositories in `/var/db/repos`.
//!
//! Descriptions come from each repository's metadata cache
//! (`metadata/md5-cache`), which rsync and webrsync syncs include. Without it,
//! bunny still knows package names from the folder layout.

use std::collections::BTreeMap;
use std::path::Path;

use crate::import::strip_version;

#[derive(Debug, Clone, PartialEq)]
pub struct Package {
    /// `category/name`
    pub name: String,
    pub description: Option<String>,
}

impl Package {
    pub fn short(&self) -> &str {
        self.name.rsplit('/').next().unwrap_or(&self.name)
    }
}

/// Every package in every repository, sorted by name.
pub fn all(root: &Path) -> Vec<Package> {
    let mut found: BTreeMap<String, Option<String>> = BTreeMap::new();
    let Ok(repos) = std::fs::read_dir(root.join("var/db/repos")) else {
        return Vec::new();
    };
    for repo in repos.flatten() {
        let repo = repo.path();
        if repo.join("metadata/md5-cache").is_dir() {
            from_cache(&repo.join("metadata/md5-cache"), &mut found);
        } else {
            from_layout(&repo, &mut found);
        }
    }
    found
        .into_iter()
        .map(|(name, description)| Package { name, description })
        .collect()
}

/// `metadata/md5-cache/<category>/<name>-<version>` files have a `DESC=` line.
fn from_cache(cache: &Path, found: &mut BTreeMap<String, Option<String>>) {
    for category in std::fs::read_dir(cache).into_iter().flatten().flatten() {
        let cat = category.file_name().to_string_lossy().into_owned();
        for entry in std::fs::read_dir(category.path())
            .into_iter()
            .flatten()
            .flatten()
        {
            let pf = entry.file_name().to_string_lossy().into_owned();
            let full = format!("{cat}/{pf}");
            let Some(name) = strip_version(&full) else {
                continue;
            };
            let slot = found.entry(name.to_string()).or_default();
            if slot.is_none() {
                *slot = std::fs::read_to_string(entry.path()).ok().and_then(|text| {
                    text.lines()
                        .find_map(|l| l.strip_prefix("DESC="))
                        .map(|d| d.trim().to_string())
                });
            }
        }
    }
}

/// Without a cache: `<category>/<name>/` folders (categories contain a dash
/// or are `virtual`).
fn from_layout(repo: &Path, found: &mut BTreeMap<String, Option<String>>) {
    for category in std::fs::read_dir(repo).into_iter().flatten().flatten() {
        let cat = category.file_name().to_string_lossy().into_owned();
        if !(cat.contains('-') || cat == "virtual") || !category.path().is_dir() {
            continue;
        }
        for entry in std::fs::read_dir(category.path())
            .into_iter()
            .flatten()
            .flatten()
        {
            if entry.path().is_dir() {
                let name = format!("{cat}/{}", entry.file_name().to_string_lossy());
                found.entry(name).or_default();
            }
        }
    }
}

/// Packages matching `query`, best matches first: exact name, name starts
/// with, name contains, then description contains.
pub fn search(packages: &[Package], query: &str) -> Vec<Package> {
    let q = query.to_lowercase();
    let mut scored: Vec<(u8, &Package)> = packages
        .iter()
        .filter_map(|p| {
            let short = p.short().to_lowercase();
            let score = if short == q || p.name.to_lowercase() == q {
                0
            } else if short.starts_with(&q) {
                1
            } else if p.name.to_lowercase().contains(&q) {
                2
            } else if p
                .description
                .as_ref()
                .is_some_and(|d| d.to_lowercase().contains(&q))
            {
                3
            } else {
                return None;
            };
            Some((score, p))
        })
        .collect();
    scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.name.cmp(&b.1.name)));
    scored.into_iter().map(|(_, p)| p.clone()).collect()
}

#[derive(Debug, PartialEq)]
pub enum Resolved {
    Found(String),
    /// Several packages have that name in different categories.
    Ambiguous(Vec<String>),
    /// Nothing by that name; similar names, if any.
    NotFound(Vec<String>),
}

/// Turns what the user typed (`firefox` or `www-client/firefox`) into a full name.
pub fn resolve(packages: &[Package], input: &str) -> Resolved {
    let input = input.trim();
    if input.contains('/') {
        let plain = input.split(':').next().unwrap_or(input);
        return if packages.iter().any(|p| p.name == plain) {
            Resolved::Found(input.to_string())
        } else {
            Resolved::NotFound(similar(packages, plain.rsplit('/').next().unwrap_or(plain)))
        };
    }
    let matches: Vec<String> = packages
        .iter()
        .filter(|p| p.short() == input)
        .map(|p| p.name.clone())
        .collect();
    match matches.len() {
        0 => Resolved::NotFound(similar(packages, input)),
        1 => Resolved::Found(matches[0].clone()),
        _ => Resolved::Ambiguous(matches),
    }
}

/// Names that start with, or contain, what was typed (up to 5).
fn similar(packages: &[Package], name: &str) -> Vec<String> {
    let name = name.to_lowercase();
    let stem: String = name.chars().take(name.chars().count().max(4) - 1).collect();
    search(packages, &stem)
        .into_iter()
        .filter(|p| p.short().contains(&stem))
        .take(5)
        .map(|p| p.name)
        .collect()
}

#[cfg(test)]
pub mod tests {
    use super::*;

    pub fn fake_repos() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("var/db/repos/gentoo/metadata/md5-cache");
        for (pf, desc) in [
            ("app-misc/tmux-3.6a", "Terminal multiplexer"),
            ("app-misc/tmux-3.5", "Terminal multiplexer"),
            ("www-client/firefox-133.0", "Firefox Web Browser"),
            (
                "www-client/firefox-bin-133.0",
                "Firefox Web Browser (binary)",
            ),
            (
                "app-editors/neovim-0.11.0",
                "Vim-fork focused on extensibility and agility",
            ),
            (
                "dev-python/pip-25.0",
                "The PyPA recommended tool for installing Python packages",
            ),
            ("app-admin/pip-1.0", "Something else called pip"),
        ] {
            let path = cache.join(pf);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(
                path,
                format!("DEFINED_PHASES=install\nDESC={desc}\nEAPI=8\n"),
            )
            .unwrap();
        }
        // An overlay without a cache: names from folders.
        let guru = dir.path().join("var/db/repos/guru");
        std::fs::create_dir_all(guru.join("gui-apps/quickshell")).unwrap();
        std::fs::create_dir_all(guru.join("metadata")).unwrap();
        dir
    }

    #[test]
    fn reads_all_packages() {
        let root = fake_repos();
        let all = all(root.path());
        let names: Vec<_> = all.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "app-admin/pip",
                "app-editors/neovim",
                "app-misc/tmux",
                "dev-python/pip",
                "gui-apps/quickshell",
                "www-client/firefox",
                "www-client/firefox-bin"
            ]
        );
        assert_eq!(all[2].description.as_deref(), Some("Terminal multiplexer"));
        assert_eq!(all[4].description, None);
    }

    #[test]
    fn searches_best_first() {
        let all = all(fake_repos().path());
        let names = |q| {
            search(&all, q)
                .into_iter()
                .map(|p| p.name)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            names("firefox"),
            ["www-client/firefox", "www-client/firefox-bin"]
        );
        assert_eq!(
            names("browser"),
            ["www-client/firefox", "www-client/firefox-bin"]
        );
        assert_eq!(names("multiplexer"), ["app-misc/tmux"]);
        assert!(names("nothing-like-this").is_empty());
    }

    #[test]
    fn resolves_names() {
        let all = all(fake_repos().path());
        assert_eq!(
            resolve(&all, "tmux"),
            Resolved::Found("app-misc/tmux".into())
        );
        assert_eq!(
            resolve(&all, "www-client/firefox"),
            Resolved::Found("www-client/firefox".into())
        );
        assert_eq!(
            resolve(&all, "pip"),
            Resolved::Ambiguous(vec!["app-admin/pip".into(), "dev-python/pip".into()])
        );
        assert_eq!(
            resolve(&all, "tmuxx"),
            Resolved::NotFound(vec!["app-misc/tmux".into()])
        );
        assert_eq!(
            resolve(&all, "app-misc/tmuxx"),
            Resolved::NotFound(vec!["app-misc/tmux".into()])
        );
    }
}
