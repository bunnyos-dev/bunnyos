//! The few questions `bunny install` asks about the disk, after you pick
//! it. First: use the recommended setup? Enter says yes and skips the rest.
//! Otherwise, one at a time, each with its recommended answer on Enter:
//! encrypt the disk, and swap. An option given on the command line answers
//! its question, which then isn't asked.
//!
//! The recommended setup: encrypted, with compressed swap in memory (zram,
//! like Fedora: as big as memory, up to 8 GB) and no swap file on disk.

use anyhow::Result;

use super::layout::{self, Choices};

/// What the command line already answered.
#[derive(Default)]
pub struct Given {
    pub no_encrypt: bool,
    /// `--swap`, `--hibernate` or `--no-zram` was given.
    pub swap: bool,
}

/// Asks what `given` doesn't answer, and fills in `choices`. `ram_mib`: the
/// computer's memory, for the hibernate size.
pub fn ask(
    choices: &mut Choices,
    given: &Given,
    ram_mib: Option<u64>,
    ask: &mut dyn FnMut(&str) -> Result<String>,
) -> Result<()> {
    if given.no_encrypt && given.swap {
        return Ok(());
    }
    if !given.no_encrypt && !given.swap {
        println!(
            "\nThe recommended setup:\n  \
             · the disk encrypted: if the computer is lost or stolen, nobody\n    \
             can read your files without the password\n  \
             · swap compressed in memory (zram), so the computer slows down\n    \
             instead of closing apps when memory runs out"
        );
        if yes_no(ask, "Use the recommended setup? [Y/n] ", true)? {
            return Ok(());
        }
    }
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
    Ok(())
}

/// Sets the three swap choices together.
fn set_swap(choices: &mut Choices, zram: bool, swap_mib: u64, hibernate: bool) {
    choices.zram = zram;
    choices.swap_mib = swap_mib;
    choices.hibernate = hibernate;
}

fn ask_swap(
    choices: &mut Choices,
    ram_mib: Option<u64>,
    ask: &mut dyn FnMut(&str) -> Result<String>,
) -> Result<()> {
    let hibernate_size = ram_mib.unwrap_or(8192);
    let encrypted = if choices.encrypt {
        ", encrypted with the system"
    } else {
        ""
    };
    println!(
        "\nSwap (extra memory when memory runs out).\n  \
         · Enter: compressed, in memory (zram). Recommended.\n  \
         · a size, like 8G: zram, plus a swap file on disk{encrypted}\n  \
         · hibernate: a swap file big enough to sleep to disk ({})\n  \
         · none",
        size_text(hibernate_size)
    );
    loop {
        let answer = ask("Swap: ")?;
        let answer = answer.trim();
        if answer.is_empty() || answer.eq_ignore_ascii_case("zram") {
            set_swap(choices, true, 0, false);
            return Ok(());
        }
        if answer.eq_ignore_ascii_case("none") || answer == "0" {
            set_swap(choices, false, 0, false);
            return Ok(());
        }
        if answer.eq_ignore_ascii_case("hibernate") {
            set_swap(choices, true, hibernate_size, true);
            return Ok(());
        }
        match layout::parse_size(answer) {
            Some(mib) if mib > 0 => {
                set_swap(choices, true, mib, false);
                return Ok(());
            }
            _ => {
                println!("Press Enter for zram, or type a size like 8G, \"hibernate\" or \"none\".")
            }
        }
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
/// The disk isn't included: it's named differently on each computer.
pub fn repeat_command(choices: &Choices, home_name: Option<&str>) -> Option<String> {
    let mut opts = Vec::new();
    if !choices.encrypt {
        opts.push("--no-encrypt".to_string());
    }
    if !choices.zram {
        opts.push("--no-zram".to_string());
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

    /// The questions asked, in order.
    type Asked = std::rc::Rc<std::cell::RefCell<Vec<String>>>;

    /// Answers the questions in order, and records them.
    fn answers(list: &[&str]) -> (impl FnMut(&str) -> Result<String>, Asked) {
        let asked = Asked::default();
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
    fn enter_once_is_the_recommended_setup() {
        let mut c = Choices::default();
        let (mut ask_fn, asked) = answers(&[""]);
        ask(&mut c, &Given::default(), Some(32768), &mut ask_fn).unwrap();
        assert!(c.encrypt);
        assert!(c.zram);
        assert_eq!(c.swap_mib, 0);
        assert_eq!(*asked.borrow(), ["Use the recommended setup? [Y/n] "]);
        assert_eq!(repeat_command(&c, None), None);
    }

    #[test]
    fn no_asks_each_question_with_its_recommended_answer_on_enter() {
        let mut c = Choices::default();
        let (mut ask_fn, asked) = answers(&["n", "", ""]);
        ask(&mut c, &Given::default(), Some(8192), &mut ask_fn).unwrap();
        assert!(c.encrypt && c.zram && c.swap_mib == 0);
        assert_eq!(
            *asked.borrow(),
            [
                "Use the recommended setup? [Y/n] ",
                "Encrypt? [Y/n] ",
                "Swap: "
            ]
        );
    }

    #[test]
    fn the_answers_become_choices_and_a_command_to_repeat_them() {
        let mut c = Choices::default();
        let (mut ask_fn, _) = answers(&["n", "n", "8G"]);
        ask(&mut c, &Given::default(), Some(8192), &mut ask_fn).unwrap();
        assert!(!c.encrypt);
        assert!(c.zram);
        assert_eq!(c.swap_mib, 8192);
        assert_eq!(
            repeat_command(&c, None).as_deref(),
            Some("bunny install --no-encrypt --swap 8G")
        );
        let mut c = Choices::default();
        let (mut ask_fn, _) = answers(&["n", "", "none"]);
        ask(&mut c, &Given::default(), Some(8192), &mut ask_fn).unwrap();
        assert!(!c.zram);
        assert_eq!(
            repeat_command(&c, None).as_deref(),
            Some("bunny install --no-zram")
        );
    }

    #[test]
    fn hibernate_sizes_the_swap_file_from_memory() {
        let mut c = Choices::default();
        let (mut ask_fn, _) = answers(&["n", "", "hibernate"]);
        ask(&mut c, &Given::default(), Some(32768), &mut ask_fn).unwrap();
        assert!(c.hibernate && c.zram);
        assert_eq!(c.swap_mib, 32768);
        assert_eq!(
            repeat_command(&c, None).as_deref(),
            Some("bunny install --hibernate")
        );
    }

    #[test]
    fn a_bad_answer_is_asked_again() {
        let mut c = Choices::default();
        let (mut ask_fn, asked) = answers(&["maybe", "n", "y", "lots", "4G"]);
        ask(&mut c, &Given::default(), None, &mut ask_fn).unwrap();
        assert!(c.encrypt);
        assert_eq!(c.swap_mib, 4096);
        assert_eq!(asked.borrow().len(), 5);
    }

    #[test]
    fn options_on_the_command_line_skip_their_questions() {
        // Both answered: nothing asked.
        let mut c = Choices {
            encrypt: false,
            swap_mib: 2048,
            ..Choices::default()
        };
        let given = Given {
            no_encrypt: true,
            swap: true,
        };
        let (mut ask_fn, asked) = answers(&[]);
        ask(&mut c, &given, Some(8192), &mut ask_fn).unwrap();
        assert!(asked.borrow().is_empty());
        assert_eq!(c.swap_mib, 2048);
        // One answered: only the other is asked, without the "recommended"
        // question.
        let mut c = Choices::default();
        let given = Given {
            no_encrypt: false,
            swap: true,
        };
        let (mut ask_fn, asked) = answers(&[""]);
        ask(&mut c, &given, Some(8192), &mut ask_fn).unwrap();
        assert_eq!(*asked.borrow(), ["Encrypt? [Y/n] "]);
    }

    #[test]
    fn sizes_read_back_the_way_they_were_typed() {
        assert_eq!(size_text(8192), "8G");
        assert_eq!(size_text(512), "512M");
        assert_eq!(size_text(1536), "1536M");
    }
}
