//! grooph-link's parser for the MCP server: `show_metronome` checks the link
//! it builds before it answers, so a model hears about notation that does not
//! fit the meter at once, by the rules the app opens the link with.
//!
//! Built for `wasm32-unknown-unknown` (see Trunk.toml, which writes it to
//! `embed/link-check.wasm`) and loaded by `mcp/link-check.js`. The interface
//! is four plain exports and no imports, so neither wasm-bindgen nor its
//! glue is needed: the host asks for a [`buffer`] of the query's length,
//! writes the query into it, calls [`check`], and reads the [`message`] it
//! returns the length of: why the link does not open, or, if [`opened`],
//! what it opens ([`grooph_link::describe`]), so the model sees what the
//! user sees.

use grooph_link::LinkContent;
use std::cell::{Cell, RefCell};

thread_local! {
    /// The query the host wrote.
    static INPUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    /// Why the last checked query does not open, or what it opens.
    static OUTPUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    /// Whether the last checked query opens.
    static OPENED: Cell<bool> = const { Cell::new(false) };
}

/// Why `query` (without `?`) cannot be opened as a link, or `None`: the
/// parser the app opens links with ([`grooph_link::parse_query`]). A query
/// that sets nothing opens too, to the defaults.
pub fn check_query(query: &str) -> Option<String> {
    open_query(query).err()
}

/// What `query` opens ([`grooph_link::describe`]), or why it does not.
/// The server is the metronome only: it refuses a drum kit score
/// ([`grooph_link::is_kit`]), which the app's links open.
pub fn open_query(query: &str) -> Result<String, String> {
    let link = grooph_link::parse_query(query)?.unwrap_or_default();
    if let Some(LinkContent::Score(score)) = &link.content
        && grooph_link::is_kit(score)
    {
        return Err(
            "rhythm: drum kit scores (instruments such as K:q, voices split by ;) are not supported by the metronome".to_owned(),
        );
    }
    Ok(grooph_link::describe(&link))
}

/// A buffer of `len` bytes for the query, in this module's memory.
#[expect(unsafe_code, reason = "exported by name for the host; no unsafe block")]
#[unsafe(no_mangle)]
pub extern "C" fn buffer(len: usize) -> *mut u8 {
    INPUT.with_borrow_mut(|input| {
        input.clear();
        input.resize(len, 0);
        input.as_mut_ptr()
    })
}

/// Checks the query in [`buffer`]; the length of the [`message`]: why it
/// does not open, or (see [`opened`]) what it opens.
#[expect(unsafe_code, reason = "exported by name for the host; no unsafe block")]
#[unsafe(no_mangle)]
pub extern "C" fn check() -> usize {
    let result = INPUT.with_borrow(|input| match std::str::from_utf8(input) {
        Ok(query) => open_query(query),
        Err(_) => Err("the link is not UTF-8".to_owned()),
    });
    OPENED.set(result.is_ok());
    OUTPUT.with_borrow_mut(|output| {
        output.clear();
        output.extend(result.unwrap_or_else(|e| e).into_bytes());
        output.len()
    })
}

/// Whether the last [`check`]ed link opens: its [`message`] says what it
/// opens, else why it does not.
#[expect(unsafe_code, reason = "exported by name for the host; no unsafe block")]
#[unsafe(no_mangle)]
pub extern "C" fn opened() -> u32 {
    u32::from(OPENED.get())
}

/// Where the last [`check`]'s message is.
#[expect(unsafe_code, reason = "exported by name for the host; no unsafe block")]
#[unsafe(no_mangle)]
pub extern "C" fn message() -> *const u8 {
    OUTPUT.with_borrow(|output| output.as_ptr())
}

#[cfg(test)]
mod tests {
    use super::check_query;

    #[test]
    fn links_the_app_opens_pass() {
        assert_eq!(check_query("v=2&bpm=140&ts=7/8&click=%3Eq+r:q.+%28q%29"), None);
        assert_eq!(check_query("v=2&ts=4/4&poly=3:2&swing=66"), None);
    }

    #[test]
    fn the_app_s_own_messages_explain_the_rest() {
        let too_long = check_query("v=2&ts=7/8&click=q+q+q+q").unwrap();
        assert!(too_long.starts_with("click:"), "{too_long}");
        let poly = check_query("v=2&ts=4/4&poly=7:5").unwrap();
        assert!(poly.contains("does not fit 4/4"), "{poly}");
        assert!(check_query("v=2&ts=99/4").unwrap().contains("time signature"));
    }

    #[test]
    fn the_metronome_refuses_a_drum_kit_score() {
        for query in ["v=2&r=K:q+S:q+K:q+S:q", "v=2&r=q+q+q+q+%3B+h+h", "v=2&r=kit+4/4+r:h+r:h"] {
            let err = check_query(query).unwrap();
            assert!(err.contains("drum kit scores") && err.contains("not supported"), "{err}");
        }
        assert!(check_query("v=2&ts=4/4&click=K:q+q+q+q").unwrap().contains("drum kit notation"));
        // A rhythm of the one line opens, as in the app.
        assert_eq!(check_query("v=2&r=%3Eq+(e)+e+r:q+q"), None);
    }

    #[test]
    fn a_link_that_sets_nothing_opens() {
        assert_eq!(check_query(""), None);
        assert_eq!(check_query("embed=mcp&theme=dark"), None);
    }

    #[test]
    fn the_exports_carry_the_message() {
        let run = |query: &str| {
            assert!(!super::buffer(query.len()).is_null());
            super::INPUT.with_borrow_mut(|input| input.copy_from_slice(query.as_bytes()));
            let len = super::check();
            let message = super::OUTPUT.with_borrow(|o| String::from_utf8(o.clone()).unwrap());
            assert_eq!(len, message.len());
            (super::opened() == 1, message)
        };
        let (opened, message) = run("v=2&ts=4/4&poly=7:5");
        assert!(!opened && message.contains("does not fit"), "{message}");
        let (opened, message) = run("v=2&ts=4/4&poly=3:2");
        assert!(opened && message.contains("polyrhythm 3:2"), "{message}");
    }

    #[test]
    fn the_description_shows_grouping_and_accents() {
        use super::open_query;
        // The asked-for 3+2+2 written as accents in the usual 2+3+2: the
        // two disagree, and the description shows it.
        let as_accents = open_query("v=2&bpm=120&ts=7/8&click=%3Ee+e+e+%3Ee+e+%3Ee+e").unwrap();
        assert_eq!(
            as_accents,
            "7/8 grouped 2+3+2 (groups start on eighths 1, 3, 6) at 120 BPM (quarter notes), \
             a click bar of its own >e e e >e e >e e (strong on eighths 1, 4, 6; medium on 2, 3, 5, \
             7); the subdivision picker offers none, e, s, t16, th, 5:4th, 6:4th, 7:4th"
        );
        let grouped = open_query("v=2&ts=7/8&groups=3%2B2%2B2").unwrap();
        assert!(
            grouped.starts_with("7/8 grouped 3+2+2 (groups start on eighths 1, 4, 6)"),
            "{grouped}"
        );
        assert!(
            grouped.ends_with(
                "subdivision eighths (subdivision=e): click bar >e (e) (e) e (e) e (e), strong on 1, \
                 medium on each beat group's start, soft between"
            ),
            "{grouped}"
        );
        // Sixteenths written out by hand are the picker's, if they match.
        let sixteenths = open_query("v=2&ts=7/8&groups=3%2B2%2B2&subdivision=s").unwrap();
        assert!(sixteenths.contains(", subdivision sixteenths (subdivision=s): "), "{sixteenths}");
        // Without a meter, the app keeps its own.
        assert_eq!(
            open_query("v=2&bpm=90&swing=60").unwrap(),
            "the meter left as it is, at 90 BPM (quarter notes), swing 60% 8ths"
        );
        // A rhythm is described by its first meter too.
        let score = open_query("v=2&bpm=80&ts=3/4&r=q+q+q+|+q+q+q").unwrap();
        assert!(
            score.starts_with(
                "a rhythm of 2 bar(s) starting in 3/4 at 80 BPM (quarter notes), no subdivision"
            ),
            "{score}"
        );
        let plain = open_query("v=2&ts=4/4&bpm=90&poly=3:2&swing=66").unwrap();
        assert_eq!(
            plain,
            "4/4 at 90 BPM (quarter notes), no subdivision (subdivision=none): click bar >q q q q, \
             one click per beat group, the first strong, polyrhythm 3:2, swing 66% 8ths"
        );
    }
}
