//! `repositories` in bunny.toml: package repositories (overlays) besides
//! Gentoo's own.
//!
//! A repository is named the way Gentoo's list of repositories names it
//! (<https://repos.gentoo.org>, the same list `eselect repository` uses), and
//! bunny looks up where to get it there. bunny writes one repos.conf file per
//! repository (`/etc/portage/repos.conf/bunny-<name>.conf`) and syncs it
//! before installing anything, so packages from it can be checked and
//! installed in the same switch.

use anyhow::{bail, Context, Result};

use crate::apply::Runner;

/// Gentoo's list of repositories.
pub const LIST_URL: &str = "https://api.gentoo.org/overlays/repositories.xml";

/// One repository in Gentoo's list.
#[derive(Debug, Clone, PartialEq)]
pub struct Listed {
    pub name: String,
    pub description: String,
    /// Run by a Gentoo project (`status="official"`), not by someone outside
    /// Gentoo. Even official ones like GURU aren't checked the way Gentoo's
    /// own tree is.
    pub official: bool,
    /// Where to get it with git (the https address when there is one).
    pub git: Option<String>,
}

/// Reads Gentoo's `repositories.xml`.
pub fn parse_list(xml: &str) -> Result<Vec<Listed>> {
    // The real list starts with a <!DOCTYPE> line, which has to be allowed.
    let options = roxmltree::ParsingOptions {
        allow_dtd: true,
        ..Default::default()
    };
    let doc = roxmltree::Document::parse_with_options(xml, options)
        .context("Gentoo's list of repositories couldn't be read")?;
    let mut out = Vec::new();
    for repo in doc.descendants().filter(|n| n.has_tag_name("repo")) {
        let text = |tag: &str| {
            repo.children()
                .find(|c| c.has_tag_name(tag))
                .and_then(|c| c.text())
                .map(|t| t.split_whitespace().collect::<Vec<_>>().join(" "))
        };
        let Some(name) = text("name") else { continue };
        let sources: Vec<String> = repo
            .children()
            .filter(|c| c.has_tag_name("source") && c.attribute("type") == Some("git"))
            .filter_map(|c| c.text().map(|t| t.trim().to_string()))
            .collect();
        let git = sources
            .iter()
            .find(|s| s.starts_with("https://"))
            .or_else(|| sources.first())
            .cloned();
        out.push(Listed {
            name,
            description: text("description").unwrap_or_default(),
            official: repo.attribute("status") == Some("official"),
            git,
        });
    }
    Ok(out)
}

/// Downloads Gentoo's list of repositories.
pub fn fetch_list(runner: &mut dyn Runner) -> Result<Vec<Listed>> {
    let out = runner.output(&["wget".into(), "-qO-".into(), LIST_URL.into()])?;
    match out {
        Some(xml) if !xml.trim().is_empty() => parse_list(&xml),
        _ => bail!(
            "Couldn't download Gentoo's list of repositories ({LIST_URL}).\n  \
             Check that this computer is online, then try again."
        ),
    }
}

/// A package that left Gentoo's own tree (or was never in it), and where
/// it lives now, for when `bunny add` can't find it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Home {
    pub package: &'static str,
    pub repository: &'static str,
    /// The repository is run by a Gentoo project (GURU) rather than people
    /// outside Gentoo.
    pub official: bool,
    /// Packages that only have testing versions there (the package itself
    /// and what it needs from the same repository), so they need
    /// `keywords = ["~amd64"]`.
    pub testing: &'static [&'static str],
}

const KNOWN_HOMES: &[Home] = &[
    Home {
        package: "gui-wm/hyprland",
        repository: "hyproverlay",
        official: false,
        testing: &["gui-wm/hyprland"],
    },
    Home {
        package: "gui-wm/mangowm",
        repository: "guru",
        official: true,
        testing: &["gui-wm/mangowm", "gui-libs/scenefx"],
    },
    Home {
        package: "gui-libs/scenefx",
        repository: "guru",
        official: true,
        testing: &["gui-libs/scenefx"],
    },
];

/// Where a package bunny couldn't find lives, if it's a well-known one.
/// Takes a full or a short name.
pub fn known_home(name: &str) -> Option<Home> {
    KNOWN_HOMES
        .iter()
        .copied()
        .find(|h| h.package == name || h.package.rsplit('/').next() == Some(name))
}

/// The plain warning shown before a repository is added.
pub fn trust_note(official: bool) -> String {
    let who = match official {
        true => "It's run by a Gentoo project, but its packages aren't checked the way Gentoo's own are.",
        false => "It's run by people outside Gentoo, and Gentoo doesn't check its packages.",
    };
    format!("{who}\nPackages run their build steps as root, so only add repositories you trust.")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Starts the way Gentoo's real list does, with a DOCTYPE line.
    const XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE repositories SYSTEM "https://www.gentoo.org/dtd/repositories.dtd">
<repositories xmlns="" version="1.0">
  <repo quality="experimental" status="official">
    <name>guru</name>
    <description lang="en">Gentoo User Repository, a repository
      maintained collaboratively by Gentoo users</description>
    <homepage>https://wiki.gentoo.org/wiki/Project:GURU</homepage>
    <owner type="project"><email>guru@gentoo.org</email></owner>
    <source type="git">git+ssh://git@git.gentoo.org/repo/proj/guru.git</source>
    <source type="git">https://anongit.gentoo.org/git/repo/proj/guru.git</source>
  </repo>
  <repo quality="experimental" status="unofficial">
    <name>hyproverlay</name>
    <description lang="en">Hyprland and friends</description>
    <owner type="person"><email>someone@example.org</email></owner>
    <source type="git">https://codeberg.org/hyproverlay/hyproverlay.git</source>
  </repo>
  <repo quality="experimental" status="unofficial">
    <name>nosource</name>
    <description lang="en">No git</description>
    <source type="rsync">rsync://example.org/x</source>
  </repo>
</repositories>"#;

    #[test]
    fn reads_gentoos_list() {
        let list = parse_list(XML).unwrap();
        assert_eq!(list.len(), 3);
        assert_eq!(list[0].name, "guru");
        assert!(list[0].official);
        assert_eq!(
            list[0].git.as_deref(),
            Some("https://anongit.gentoo.org/git/repo/proj/guru.git")
        );
        assert!(list[0]
            .description
            .starts_with("Gentoo User Repository, a repository maintained"));
        assert!(!list[1].official);
        assert_eq!(list[2].git, None);
        assert!(parse_list("<not xml").is_err());
    }

    #[test]
    fn knows_where_some_packages_went() {
        assert_eq!(
            known_home("gui-wm/hyprland").unwrap().repository,
            "hyproverlay"
        );
        let mango = known_home("mangowm").unwrap();
        assert_eq!(
            (mango.package, mango.repository),
            ("gui-wm/mangowm", "guru")
        );
        assert!(mango.testing.contains(&"gui-libs/scenefx"));
        assert_eq!(known_home("app-misc/tmux"), None);
    }

    #[test]
    fn warns_more_for_repositories_outside_gentoo() {
        assert!(trust_note(true).contains("run by a Gentoo project"));
        assert!(trust_note(false).contains("outside Gentoo"));
    }
}
