//! Gentoo security advisories (GLSAs) that affect this system, from
//! `glsa-check` (in app-portage/gentoolkit). The advisories themselves come
//! with the package tree, so a recent sync matters too.

/// One advisory that affects the system.
#[derive(Debug, PartialEq, Clone)]
pub struct Advisory {
    /// Like "202501-03".
    pub id: String,
    /// What it's about, like "OpenSSL: Multiple vulnerabilities".
    pub title: String,
    /// The packages it's about.
    pub packages: Vec<String>,
}

/// What the check found.
#[derive(Debug, PartialEq, Clone)]
pub enum Check {
    /// Checked: these advisories affect the system (none = all clear).
    Done(Vec<Advisory>),
    /// `glsa-check` isn't installed.
    NotInstalled,
    /// It ran but failed; what it said.
    Failed(String),
}

fn is_id(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 9
        && b[6] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 6 || c.is_ascii_digit())
}

/// Reads `glsa-check --nocolor --list affected`, whose lines look like
/// `202501-03 [N] OpenSSL: Multiple vulnerabilities ( dev-libs/openssl )`.
/// `[N]` means the system may be affected; `[A]` (applied) and `[U]`
/// (unaffected) are skipped, in case they're listed too.
pub fn parse_list(output: &str) -> Vec<Advisory> {
    output
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let (id, rest) = line.split_once(' ')?;
            if !is_id(id) {
                return None;
            }
            let rest = rest.trim_start();
            let rest = match rest.strip_prefix('[') {
                Some(r) => {
                    let (mark, after) = r.split_once(']')?;
                    if matches!(mark.trim(), "A" | "U") {
                        return None;
                    }
                    after.trim_start()
                }
                None => rest,
            };
            let (title, packages) = match rest.rfind(" ( ") {
                Some(at) if rest.ends_with(')') => (
                    &rest[..at],
                    rest[at + 3..rest.len() - 1]
                        .split_whitespace()
                        .map(String::from)
                        .collect(),
                ),
                _ => (rest, Vec::new()),
            };
            Some(Advisory {
                id: id.to_string(),
                title: title.trim().to_string(),
                packages,
            })
        })
        .collect()
}

/// Runs the check on the real system.
pub fn check() -> Check {
    match std::process::Command::new("glsa-check")
        .args(["--nocolor", "--list", "affected"])
        .output()
    {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Check::NotInstalled,
        Err(e) => Check::Failed(e.to_string()),
        Ok(out) if !out.status.success() => Check::Failed(
            String::from_utf8_lossy(&out.stderr)
                .lines()
                .last()
                .unwrap_or("it stopped with an error")
                .to_string(),
        ),
        Ok(out) => Check::Done(parse_list(&String::from_utf8_lossy(&out.stdout))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_affected_advisories() {
        let out = "\
[A] means this GLSA was marked as applied (injected),
[U] means the system is not affected and
[N] indicates that the system might be affected.

202501-03 [N] OpenSSL: Multiple vulnerabilities ( dev-libs/openssl )
202412-10 [N] GnuTLS: Denial of service ( net-libs/gnutls )
202401-01 [U] Boost: Buffer overflow ( dev-libs/boost )
202312-02 [A] Old thing ( app-misc/old )
";
        let found = parse_list(out);
        assert_eq!(found.len(), 2);
        assert_eq!(
            found[0],
            Advisory {
                id: "202501-03".into(),
                title: "OpenSSL: Multiple vulnerabilities".into(),
                packages: vec!["dev-libs/openssl".into()],
            }
        );
        assert_eq!(found[1].id, "202412-10");
    }

    #[test]
    fn copes_with_several_packages_and_no_marker() {
        let found = parse_list("202502-07 Mozilla products: Many problems ( www-client/firefox mail-client/thunderbird )\n");
        assert_eq!(
            found[0].packages,
            ["www-client/firefox", "mail-client/thunderbird"]
        );
        assert_eq!(found[0].title, "Mozilla products: Many problems");
    }

    #[test]
    fn nothing_affected_means_an_empty_list() {
        assert!(parse_list("").is_empty());
        assert!(parse_list("Nothing to do.\n2025-01 not an id\n").is_empty());
    }
}
