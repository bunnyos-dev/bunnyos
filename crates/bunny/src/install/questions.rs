//! The few questions `bunny install` asks about the disk, after you pick
//! it: encrypt it, swap, and /home on a second disk (only when there is
//! one). Each has a recommended answer, so pressing Enter through them gives
//! the recommended setup. An option given on the command line answers its
//! question, which then isn't asked.

use anyhow::Result;

use super::disks::Disk;
use super::layout::{self, Choices};

/// From this much memory, no swap is the recommendation.
const NO_SWAP_FROM_MIB: u64 = 16 * 1024;

/// What the command line already answered.
#[derive(Default)]
pub struct Given {
    pub no_encrypt: bool,
    /// `--swap` or `--hibernate` was given.
    pub swap: bool,
    /// `--home-disk` was given.
    pub home_disk: bool,
}

/// Asks what `given` doesn't answer, and fills in `choices`. `others`: the
/// other disks that could hold /home. `ram_mib`: the computer's memory, for
/// the swap advice and the hibernate size.
pub fn ask(
    choices: &mut Choices,
    given: &Given,
    others: &[&Disk],
    ram_mib: Option<u64>,
    ask: &mut dyn FnMut(&str) -> Result<String>,
) -> Result<()> {
    if !given.no_encrypt {
        println!(
            "\nEncrypt the disk?  Recommended: yes.\n  \
             If the computer is lost or stolen, nobody can read\n  \
             your files without the password.\n  \
             You type it once each time the computer starts."
        );
        choices.encrypt = yes_no(ask, "Encrypt? [Y/n] ", true)?;
    }
    if !given.swap {
        ask_swap(choices, ram_mib, ask)?;
    }
    if !given.home_disk && !others.is_empty() {
        choices.home_disk = ask_home(others, ask)?;
    }
    Ok(())
}

fn ask_swap(
    choices: &mut Choices,
    ram_mib: Option<u64>,
    ask: &mut dyn FnMut(&str) -> Result<String>,
) -> Result<()> {
    let advice = match ram_mib {
        Some(ram) if ram >= NO_SWAP_FROM_MIB => {
            "Recommended: none\n  with 16 GB of memory or more; your choice otherwise."
        }
        _ => "Recommended: a little\n  with less than 16 GB of memory (4G is plenty); none works too.",
    };
    let hibernate_size = ram_mib.unwrap_or(8192);
    let where_it_is = if choices.encrypt {
        "It's a file inside the encrypted system, so it's\n  encrypted too."
    } else {
        "It's a file inside the system."
    };
    println!(
        "\nSwap (extra memory on disk). {advice}\n  \
         · Enter: none\n  \
         · a size, like 8G\n  \
         · hibernate: big enough to sleep to disk ({})\n  \
         {where_it_is}",
        size_text(hibernate_size)
    );
    loop {
        let answer = ask("Swap: ")?;
        let answer = answer.trim();
        if answer.is_empty() || answer.eq_ignore_ascii_case("none") || answer == "0" {
            choices.swap_mib = 0;
            choices.hibernate = false;
            return Ok(());
        }
        if answer.eq_ignore_ascii_case("hibernate") {
            choices.swap_mib = hibernate_size;
            choices.hibernate = true;
            return Ok(());
        }
        match layout::parse_size(answer) {
            Some(mib) if mib > 0 => {
                choices.swap_mib = mib;
                choices.hibernate = false;
                return Ok(());
            }
            _ => println!("Type a size like 8G or 512M, \"hibernate\", or press Enter for none."),
        }
    }
}

fn ask_home(
    others: &[&Disk],
    ask: &mut dyn FnMut(&str) -> Result<String>,
) -> Result<Option<String>> {
    if let [only] = others {
        println!(
            "\nKeep your files (/home) on the other disk?  Recommended: no.\n  \
             {}\n  \
             Your files stay there if you reinstall later.\n  \
             ⚠ This ERASES {} too.",
            only.describe(),
            only.name
        );
        let yes = yes_no(ask, &format!("/home on {}? [y/N] ", only.name), false)?;
        return Ok(yes.then(|| only.path.clone()));
    }
    let names: Vec<&str> = others.iter().map(|d| d.name.as_str()).collect();
    println!("\nKeep your files (/home) on another disk?  Recommended: no.");
    for d in others {
        println!("  {}", d.describe());
    }
    println!(
        "  Your files stay there if you reinstall later.\n  \
         ⚠ The disk you name is ERASED too."
    );
    loop {
        let answer = ask(&format!(
            "/home on (Enter for no, or {}): ",
            names.join(", ")
        ))?;
        let answer = answer.trim();
        if answer.is_empty()
            || answer.eq_ignore_ascii_case("no")
            || answer.eq_ignore_ascii_case("n")
        {
            return Ok(None);
        }
        if let Some(d) = others.iter().find(|d| d.name == answer || d.path == answer) {
            return Ok(Some(d.path.clone()));
        }
        println!(
            "That's not one of them: type {}, or press Enter for no.",
            names.join(", ")
        );
    }
}

/// A yes/no question; Enter takes `default`. Asks again on anything else.
fn yes_no(
    ask: &mut dyn FnMut(&str) -> Result<String>,
    question: &str,
    default: bool,
) -> Result<bool> {
    loop {
        match ask(question)?.trim().to_ascii_lowercase().as_str() {
            "" => return Ok(default),
            "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => println!(
                "Type y or n (or press Enter for {}).",
                if default { "yes" } else { "no" }
            ),
        }
    }
}

/// `8G` for a whole number of GiB, `512M` otherwise.
pub fn size_text(mib: u64) -> String {
    if mib >= 1024 && mib.is_multiple_of(1024) {
        format!("{}G", mib / 1024)
    } else {
        format!("{mib}M")
    }
}

/// The options that make the same choices without asking, for installing
/// the same way on another computer (`None` if it's the recommended setup).
/// The disk isn't included: it's named differently on each computer. With
/// /home on a second disk, `--home-disk` names it here; the other computer
/// may call it something else.
pub fn repeat_command(choices: &Choices, home_name: Option<&str>) -> Option<String> {
    let mut opts = Vec::new();
    if !choices.encrypt {
        opts.push("--no-encrypt".to_string());
    }
    if choices.hibernate {
        opts.push("--hibernate".to_string());
    } else if choices.swap_mib > 0 {
        opts.push(format!("--swap {}", size_text(choices.swap_mib)));
    }
    if let Some(name) = home_name {
        opts.push(format!("--home-disk {name}"));
    }
    (!opts.is_empty()).then(|| format!("bunny install {}", opts.join(" ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn disk(name: &str) -> Disk {
        Disk {
            name: name.into(),
            path: format!("/dev/{name}"),
            size: 2_000_000_000_000,
            model: Some("WD Blue".into()),
            transport: Some("sata".into()),
            removable: false,
            read_only: false,
            parts: Vec::new(),
            mounted: Vec::new(),
            fstype: None,
        }
    }

    /// The questions asked, in order.
    type Asked = std::rc::Rc<std::cell::RefCell<Vec<String>>>;

    /// Answers the questions in order, and records them.
    fn answers(list: &[&str]) -> (impl FnMut(&str) -> Result<String>, Asked) {
        let asked = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let seen = asked.clone();
        let mut list: Vec<String> = list.iter().map(|s| s.to_string()).collect();
        list.reverse();
        (
            move |q: &str| {
                seen.borrow_mut().push(q.to_string());
                Ok(list.pop().expect("asked more than expected"))
            },
            asked,
        )
    }

    #[test]
    fn enter_through_everything_is_the_recommended_setup() {
        let mut c = Choices::default();
        let sda = disk("sda");
        let (mut ask_fn, asked) = answers(&["", "", ""]);
        ask(&mut c, &Given::default(), &[&sda], Some(32768), &mut ask_fn).unwrap();
        assert!(c.encrypt);
        assert_eq!(c.swap_mib, 0);
        assert!(!c.hibernate);
        assert_eq!(c.home_disk, None);
        assert_eq!(
            *asked.borrow(),
            ["Encrypt? [Y/n] ", "Swap: ", "/home on sda? [y/N] "]
        );
        assert_eq!(repeat_command(&c, None), None);
    }

    #[test]
    fn the_answers_become_choices_and_a_command_to_repeat_them() {
        let mut c = Choices::default();
        let (mut ask_fn, _) = answers(&["n", "8G"]);
        ask(&mut c, &Given::default(), &[], Some(8192), &mut ask_fn).unwrap();
        assert!(!c.encrypt);
        assert_eq!(c.swap_mib, 8192);
        assert_eq!(
            repeat_command(&c, None).as_deref(),
            Some("bunny install --no-encrypt --swap 8G")
        );
    }

    #[test]
    fn hibernate_sizes_swap_from_memory() {
        let mut c = Choices::default();
        let (mut ask_fn, _) = answers(&["", "hibernate"]);
        ask(&mut c, &Given::default(), &[], Some(32768), &mut ask_fn).unwrap();
        assert!(c.hibernate);
        assert_eq!(c.swap_mib, 32768);
        assert_eq!(
            repeat_command(&c, None).as_deref(),
            Some("bunny install --hibernate")
        );
    }

    #[test]
    fn a_bad_answer_is_asked_again() {
        let mut c = Choices::default();
        let (mut ask_fn, asked) = answers(&["maybe", "y", "lots", "4G"]);
        ask(&mut c, &Given::default(), &[], None, &mut ask_fn).unwrap();
        assert!(c.encrypt);
        assert_eq!(c.swap_mib, 4096);
        assert_eq!(asked.borrow().len(), 4);
    }

    #[test]
    fn home_on_a_second_disk_is_a_deliberate_yes() {
        let sda = disk("sda");
        let mut c = Choices::default();
        let (mut ask_fn, _) = answers(&["", "", "y"]);
        ask(&mut c, &Given::default(), &[&sda], Some(16384), &mut ask_fn).unwrap();
        assert_eq!(c.home_disk.as_deref(), Some("/dev/sda"));
        assert_eq!(
            repeat_command(&c, Some("sda")).as_deref(),
            Some("bunny install --home-disk sda")
        );
        // With two disks to choose from, it's named; a wrong name asks again.
        let sdb = disk("sdb");
        let mut c = Choices::default();
        let (mut ask_fn, asked) = answers(&["", "", "sdc", "sdb"]);
        ask(
            &mut c,
            &Given::default(),
            &[&sda, &sdb],
            Some(16384),
            &mut ask_fn,
        )
        .unwrap();
        assert_eq!(c.home_disk.as_deref(), Some("/dev/sdb"));
        assert_eq!(asked.borrow()[2], "/home on (Enter for no, or sda, sdb): ");
    }

    #[test]
    fn options_on_the_command_line_skip_their_questions() {
        let sda = disk("sda");
        let mut c = Choices {
            encrypt: false,
            swap_mib: 2048,
            ..Choices::default()
        };
        let given = Given {
            no_encrypt: true,
            swap: true,
            home_disk: true,
        };
        let (mut ask_fn, asked) = answers(&[]);
        ask(&mut c, &given, &[&sda], Some(8192), &mut ask_fn).unwrap();
        assert!(asked.borrow().is_empty());
        assert_eq!(c.swap_mib, 2048);
        // No second disk: no /home question.
        let mut c = Choices::default();
        let (mut ask_fn, asked) = answers(&["", ""]);
        ask(&mut c, &Given::default(), &[], Some(8192), &mut ask_fn).unwrap();
        assert_eq!(*asked.borrow(), ["Encrypt? [Y/n] ", "Swap: "]);
    }

    #[test]
    fn sizes_read_back_the_way_they_were_typed() {
        assert_eq!(size_text(8192), "8G");
        assert_eq!(size_text(512), "512M");
        assert_eq!(size_text(1536), "1536M");
    }
}
