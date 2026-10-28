//! grooph-link's parser for the MCP server: `show_metronome` checks the link
//! it builds before it answers, so a model hears about notation that does not
//! fit the meter at once, by the rules the app opens the link with.
//!
//! Built for `wasm32-unknown-unknown` (see Trunk.toml, which writes it to
//! `embed/link-check.wasm`) and loaded by `mcp/link-check.js`. The interface
//! is three plain exports and no imports, so neither wasm-bindgen nor its
//! glue is needed: the host asks for a [`buffer`] of the query's length,
//! writes the query into it, calls [`check`], and reads the [`message`] it
//! returns the length of (0: the link opens).

use std::cell::RefCell;

thread_local! {
    /// The query the host wrote.
    static INPUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    /// Why the last checked query does not open, empty if it does.
    static OUTPUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

/// Why `query` (without `?`) cannot be opened as a link, or `None`. The same
/// parser the app opens links with ([`grooph_link::parse_query_current`]).
pub fn check_query(query: &str) -> Option<String> {
    match grooph_link::parse_query_current(query) {
        Ok(Some(_)) => None,
        Ok(None) => Some("the link sets nothing".to_owned()),
        Err(err) => Some(err),
    }
}

/// A buffer of `len` bytes for the query, in this module's memory.
// The export attribute counts as unsafe code; there is no unsafe block.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn buffer(len: usize) -> *mut u8 {
    INPUT.with_borrow_mut(|input| {
        input.clear();
        input.resize(len, 0);
        input.as_mut_ptr()
    })
}

/// Checks the query in [`buffer`]; the length of the [`message`], 0 if the
/// link opens.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn check() -> usize {
    let problem = INPUT.with_borrow(|input| match std::str::from_utf8(input) {
        Ok(query) => check_query(query),
        Err(_) => Some("the link is not UTF-8".to_owned()),
    });
    OUTPUT.with_borrow_mut(|output| {
        output.clear();
        output.extend(problem.unwrap_or_default().into_bytes());
        output.len()
    })
}

/// Where the last [`check`]'s message is.
#[allow(unsafe_code)]
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
        assert!(check_query("").is_some());
    }

    #[test]
    fn the_exports_carry_the_message() {
        let query = "v=2&ts=4/4&poly=7:5";
        let ptr = super::buffer(query.len());
        assert!(!ptr.is_null());
        super::INPUT.with_borrow_mut(|input| input.copy_from_slice(query.as_bytes()));
        let len = super::check();
        let message = super::OUTPUT.with_borrow(|o| String::from_utf8(o.clone()).unwrap());
        assert_eq!(len, message.len());
        assert!(message.contains("does not fit"));
        super::INPUT.with_borrow_mut(|input| input.clear());
        assert_eq!(super::check(), "the link sets nothing".len());
    }
}
