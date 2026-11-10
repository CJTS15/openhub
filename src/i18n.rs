//! Translation helpers on top of gettext. Messages with values use `{}`
//! placeholders, filled in order, so translators can move them:
//! `gettext_f("{} connected", &[&name])`.

pub use gettextrs::{gettext, ngettext};

/// Marks a string for translation without translating it, for tables
/// that are translated where they're shown.
pub const fn gettext_noop(message: &'static str) -> &'static str {
    message
}

/// A translated message with its `{}` placeholders filled in order.
pub fn gettext_f(message: &str, args: &[&str]) -> String {
    fill(&gettext(message), args)
}

/// The singular or plural form for `n`, with its `{}` placeholders filled.
pub fn ngettext_f(singular: &str, plural: &str, n: u32, args: &[&str]) -> String {
    fill(&ngettext(singular, plural, n), args)
}

fn fill(template: &str, args: &[&str]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut args = args.iter();
    let mut rest = template;
    while let Some(at) = rest.find("{}") {
        out.push_str(&rest[..at]);
        out.push_str(args.next().copied().unwrap_or("{}"));
        rest = &rest[at + 2..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fills_placeholders_in_order() {
        assert_eq!(fill("{} of {}", &["3", "9"]), "3 of 9");
        assert_eq!(fill("No values", &["x"]), "No values");
        // A missing value leaves its placeholder, rather than panicking.
        assert_eq!(fill("{} and {}", &["a"]), "a and {}");
    }

    #[test]
    fn falls_back_to_english_without_a_translation() {
        assert_eq!(
            gettext_f("{} connected", &["MX Master 3S"]),
            "MX Master 3S connected"
        );
        assert_eq!(
            ngettext_f("{} device", "{} devices", 2, &["2"]),
            "2 devices"
        );
    }
}
