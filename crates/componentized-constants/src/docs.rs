use anyhow::{Result, bail};
use wit_parser::Docs;

/// Doc comment tag introducing a function's value.
pub const TAG: &str = "@value";

/// Doc comment tag introducing the expression generating the items of a
/// stream that follow its `@value`.
pub const EXPRESSION_TAG: &str = "@expression";

const TAGS: [&str; 2] = [TAG, EXPRESSION_TAG];

/// Returns the WAVE value following the `@value` tag in a doc comment, if any.
///
/// The tag must start a line. Everything after it, including any following
/// lines up to another tag, is the value, so a value may span multiple lines:
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
    tag(docs, TAG)
}

/// Returns the expression following the `@expression` tag in a doc comment,
/// if any, see [`value`].
pub fn expression(docs: &Docs) -> Option<&str> {
    tag(docs, EXPRESSION_TAG)
}

/// Ensures a doc comment has at most one of each tag.
pub fn check(docs: &Docs) -> Result<()> {
    let tags: Vec<&str> = lines(docs).filter_map(|(_, _, tag)| tag).collect();
    for (i, tag) in tags.iter().enumerate() {
        if tags[..i].contains(tag) {
            bail!("duplicate `{tag}` tag, a doc comment may have one of each tag");
        }
    }
    Ok(())
}

/// The contents following `tag`, up to the next line starting with a tag.
fn tag<'a>(docs: &'a Docs, tag: &str) -> Option<&'a str> {
    let contents = docs.contents.as_deref()?;
    let mut start = None;
    for (offset, indent, found) in lines(docs) {
        match (start, found) {
            (None, Some(t)) if t == tag => start = Some(offset + indent + t.len()),
            (Some(start), Some(_)) => return Some(&contents[start..offset]),
            _ => {}
        }
    }
    start.map(|start| &contents[start..])
}

/// Each line of a doc comment's contents: its offset, its indent, and the tag
/// it starts with, if any. A tag must be followed by whitespace or the end of
/// the line.
fn lines(docs: &Docs) -> impl Iterator<Item = (usize, usize, Option<&'static str>)> + '_ {
    let contents = docs.contents.as_deref().unwrap_or_default();
    contents
        .split_inclusive('\n')
        .scan(0, |offset, line| {
            let start = *offset;
            *offset += line.len();
            Some((start, line))
        })
        .map(|(offset, line)| {
            let indent = line.len() - line.trim_start().len();
            let found = TAGS.into_iter().find(|t| {
                line[indent..]
                    .strip_prefix(t)
                    .is_some_and(|rest| rest.is_empty() || rest.starts_with(char::is_whitespace))
            });
            (offset, indent, found)
        })
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

    #[test]
    fn it_rejects_duplicate_tags() {
        let docs = |contents: &str| Docs {
            contents: Some(contents.into()),
        };
        assert!(check(&Docs::default()).is_ok());
        assert!(check(&docs("@value 1\n@expression |n| n\nSee @value 2")).is_ok());
        for contents in [
            "@value 1\n@value 2",
            "@expression |n| n\n@value [1]\n  @expression |n| n",
        ] {
            let err = check(&docs(contents)).expect_err(contents).to_string();
            assert!(err.starts_with("duplicate `@"), "{err}");
        }
    }

    #[test]
    fn it_ends_values_at_the_next_tag() {
        let docs = Docs {
            contents: Some("Counts.\n@value [\n  0,\n]\n@expression |n| n + 1\n".into()),
        };
        assert_eq!(value(&docs), Some(" [\n  0,\n]\n"));
        assert_eq!(expression(&docs), Some(" |n| n + 1\n"));

        let docs = Docs {
            contents: Some("@expression |n| n\n@value [0]".into()),
        };
        assert_eq!(value(&docs), Some(" [0]"));
        assert_eq!(expression(&docs), Some(" |n| n\n"));
    }
}
