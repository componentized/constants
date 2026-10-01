use wit_parser::Docs;

/// Doc comment tag introducing a function's value.
pub const TAG: &str = "@value";

/// Returns the WAVE value following the `@value` tag in a doc comment, if any.
///
/// The tag must start a line. Everything after it, including any following
/// lines, is the value, so a value may span multiple lines:
///
/// ```wit
/// /// The origin of the plane.
/// ///
/// /// @value {
/// ///   x: 0,
/// ///   y: 0,
/// /// }
/// origin: func() -> point;
/// ```
pub fn value(docs: &Docs) -> Option<&str> {
    let contents = docs.contents.as_deref()?;
    let mut offset = 0;
    for line in contents.split_inclusive('\n') {
        let indent = line.len() - line.trim_start().len();
        if let Some(rest) = line[indent..].strip_prefix(TAG) {
            if rest.is_empty() || rest.starts_with(char::is_whitespace) {
                return Some(&contents[offset + indent + TAG.len()..]);
            }
        }
        offset += line.len();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value_of(contents: &str) -> Option<String> {
        let docs = Docs {
            contents: Some(contents.to_string()),
        };
        value(&docs).map(String::from)
    }

    #[test]
    fn it_finds_values_after_the_tag() {
        assert_eq!(value(&Docs::default()), None);
        assert_eq!(value_of("Just docs."), None);
        assert_eq!(value_of("@value 42").as_deref(), Some(" 42"));
        assert_eq!(value_of("An answer.\n\n@value 42").as_deref(), Some(" 42"));
        assert_eq!(
            value_of("A point.\n@value {\n  x: 0,\n}").as_deref(),
            Some(" {\n  x: 0,\n}")
        );
        // the tag must start a line and be followed by whitespace
        assert_eq!(value_of("See @value 1"), None);
        assert_eq!(value_of("@values 1"), None);
    }
}
