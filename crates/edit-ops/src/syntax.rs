//! The two things every module here starts from: what the language's
//! tokens are, and what the parse tree says about a position.

use syntax_core::{registry, Language, MAX_HIGHLIGHT_BYTES};
use tree_sitter::{Node, Parser, Tree};

/// One language's editing tokens, read out of the registry once.
///
/// Owned rather than borrowed: a registry snapshot may be replaced by a
/// language reload at any time, so nothing read out of it can be held
/// across a call. Four short `String`s per operation is not a cost worth
/// a lifetime parameter.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Tokens {
    pub line_comment: Option<String>,
    pub block_comment: Option<(String, String)>,
    pub brackets: Vec<(String, String)>,
    pub quotes: Vec<String>,
}

impl Tokens {
    /// The tokens `language` declares. Plain text declares none, which is
    /// what makes every operation here a no-op in a file with no grammar
    /// rather than a wrong guess.
    pub fn of(language: Language) -> Self {
        let registry = registry();
        let Some(def) = registry.def(language) else {
            return Self::default();
        };
        Self {
            line_comment: def.line_comment().map(str::to_string),
            block_comment: def
                .block_comment()
                .map(|(open, close)| (open.to_string(), close.to_string())),
            brackets: def
                .brackets()
                .into_iter()
                .map(|(open, close)| (open.to_string(), close.to_string()))
                .collect(),
            quotes: def.quotes().into_iter().map(str::to_string).collect(),
        }
    }

    /// The closing delimiter for `open`, if it is an opening bracket.
    pub fn close_for(&self, open: &str) -> Option<&str> {
        self.brackets
            .iter()
            .find(|(candidate, _)| candidate == open)
            .map(|(_, close)| close.as_str())
    }

    /// The opening delimiter for `close`, if it is a closing bracket.
    pub fn open_for(&self, close: &str) -> Option<&str> {
        self.brackets
            .iter()
            .find(|(_, candidate)| candidate == close)
            .map(|(open, _)| open.as_str())
    }

    pub fn is_quote(&self, text: &str) -> bool {
        self.quotes.iter().any(|quote| quote == text)
    }

    /// The closing delimiter typing `text` should produce, whether that is
    /// a bracket's partner or a quote's twin.
    pub fn closing_for(&self, text: &str) -> Option<String> {
        if self.is_quote(text) {
            return Some(text.to_string());
        }
        self.close_for(text).map(str::to_string)
    }
}

/// A parsed buffer, or the honest absence of one.
///
/// Absent for plain text, for a language whose queries do not compile, and
/// for a file past [`MAX_HIGHLIGHT_BYTES`] — the same ceiling highlighting
/// respects, because a 50 MB log file must not be parsed to answer "am I
/// inside a string?". Every caller has a text-only fallback for that case,
/// and the fallback is the path most exercised in practice.
pub struct Syntax {
    tree: Option<Tree>,
}

impl Syntax {
    pub fn parse(language: Language, text: &str) -> Self {
        Self {
            tree: parse(language, text),
        }
    }

    /// A [`Syntax`] that knows nothing, for callers that already decided
    /// not to parse.
    pub fn none() -> Self {
        Self { tree: None }
    }

    pub fn tree(&self) -> Option<&Tree> {
        self.tree.as_ref()
    }

    pub fn has_tree(&self) -> bool {
        self.tree.is_some()
    }

    /// Whether `offset` is **strictly inside** a string, character or
    /// comment node — the positions where auto-closing a bracket and
    /// counting bracket depth are both wrong.
    ///
    /// Strictly: a position at the very start of a string literal is not
    /// inside it, so typing a quote immediately before an existing string
    /// still behaves like typing at code.
    ///
    /// The node kinds are matched by name (`*string*`, `*comment*`,
    /// `*char*`), because tree-sitter has no cross-grammar node category.
    /// A grammar that names its string node something else degrades to
    /// "not in a string", which is the same answer the no-tree fallback
    /// gives — a heuristic that fails safe.
    pub fn in_literal_or_comment(&self, offset: usize) -> bool {
        let Some(tree) = &self.tree else {
            return false;
        };
        let mut node = tree.root_node().descendant_for_byte_range(offset, offset);
        while let Some(current) = node {
            if current.start_byte() < offset
                && offset < current.end_byte()
                && is_literal_or_comment(current)
            {
                return true;
            }
            node = current.parent();
        }
        false
    }

    /// The smallest node strictly larger than `start..end`, for expanding a
    /// selection. `None` without a tree, and at the root.
    pub fn enclosing_range(&self, start: usize, end: usize) -> Option<std::ops::Range<usize>> {
        let tree = self.tree.as_ref()?;
        let mut node = tree.root_node().descendant_for_byte_range(start, end)?;
        loop {
            if node.start_byte() < start || node.end_byte() > end {
                return Some(node.start_byte()..node.end_byte());
            }
            node = node.parent()?;
        }
    }

    /// The node whose text is exactly `range`, if the tree has one — how a
    /// bracket token is found before its siblings are searched.
    pub fn node_at(&self, range: std::ops::Range<usize>) -> Option<Node<'_>> {
        self.tree
            .as_ref()?
            .root_node()
            .descendant_for_byte_range(range.start, range.end)
    }
}

fn is_literal_or_comment(node: Node<'_>) -> bool {
    let kind = node.kind();
    kind.contains("string") || kind.contains("comment") || kind.contains("char")
}

fn parse(language: Language, text: &str) -> Option<Tree> {
    if text.len() > MAX_HIGHLIGHT_BYTES {
        return None;
    }
    let compiled = registry().compiled(language)?.ok()?;
    let mut parser = Parser::new();
    parser.set_language(&compiled.grammar).ok()?;
    parser.parse(text, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use syntax_core::language_by_id;

    fn rust() -> Language {
        language_by_id("rust").expect("rust is in the catalog")
    }

    #[test]
    fn tokens_come_from_the_registry_and_plain_text_has_none() {
        let tokens = Tokens::of(rust());
        assert_eq!(tokens.line_comment.as_deref(), Some("//"));
        assert_eq!(tokens.close_for("("), Some(")"));
        assert_eq!(tokens.open_for("}"), Some("{"));
        assert!(tokens.is_quote("\""));

        let none = Tokens::of(Language::PLAIN_TEXT);
        assert_eq!(none, Tokens::default());
        assert_eq!(none.closing_for("("), None);
    }

    #[test]
    fn a_position_inside_a_string_or_comment_is_recognised() {
        let text = "fn main() { let s = \"a(b\"; } // (\n";
        let syntax = Syntax::parse(rust(), text);
        assert!(syntax.has_tree());

        let in_string = text.find("a(b").expect("fixture") + 1;
        assert!(syntax.in_literal_or_comment(in_string));

        let in_comment = text.rfind('(').expect("fixture");
        assert!(syntax.in_literal_or_comment(in_comment));

        let in_code = text.find("let").expect("fixture") + 1;
        assert!(!syntax.in_literal_or_comment(in_code));
    }

    #[test]
    fn a_file_past_the_highlight_ceiling_is_not_parsed() {
        let huge = "// x\n".repeat(MAX_HIGHLIGHT_BYTES / 5 + 1);
        assert!(!Syntax::parse(rust(), &huge).has_tree());
        assert!(!Syntax::parse(Language::PLAIN_TEXT, "fn x() {}").has_tree());
    }
}
