use crate::food;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const FALLBACK_MESSAGE_WIDTH: usize = 80;
const PREFIX: &str = "Note: ";

/// A model proposal plus the optional user-facing message that came with it.
/// The message is response-only: it is rendered with the proposal and never
/// written to a day log or food file.
#[derive(Debug)]
pub(crate) struct Proposal<T> {
    pub value: T,
    pub message: Option<String>,
}

impl<T> Proposal<T> {
    pub(crate) fn new(value: T, message: Option<String>) -> Proposal<T> {
        let message = message
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty());
        Proposal { value, message }
    }

    /// Render `body` followed by the optional message block. `table_width`
    /// caps the block to the proposal's table when one was rendered; without
    /// a table, COLUMNS (or a conventional fallback) is used.
    pub(crate) fn render_with_table_width(
        &self,
        body: String,
        table_width: Option<usize>,
    ) -> String {
        self.render_with_width(body, message_width(table_width))
    }

    fn render_with_width(&self, body: String, width: usize) -> String {
        let Some(message) = &self.message else {
            return body;
        };
        let mut out = body.trim_end_matches('\n').to_string();
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        let text_width = width.saturating_sub(PREFIX.len()).max(1);
        let mut first_line = true;
        for source_line in message.lines() {
            for line in wrap_line(source_line, text_width) {
                if line.is_empty() {
                    out.push('\n');
                    continue;
                }
                if first_line {
                    out.push_str(PREFIX);
                    first_line = false;
                } else {
                    out.push_str(&" ".repeat(PREFIX.len()));
                }
                out.push_str(&line);
                out.push('\n');
            }
        }
        out.push('\n');
        out
    }
}

fn message_width(table_width: Option<usize>) -> usize {
    let columns = std::env::var("COLUMNS")
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|width| *width > 0);
    effective_width(table_width, columns)
}

fn effective_width(table_width: Option<usize>, columns: Option<usize>) -> usize {
    match (table_width, columns) {
        (Some(table), Some(columns)) => table.min(columns),
        (Some(table), None) => table,
        (None, Some(columns)) => columns,
        (None, None) => FALLBACK_MESSAGE_WIDTH,
    }
}

fn wrap_line(line: &str, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut current_width = 0;

    for word in line.split_whitespace() {
        let word_width = UnicodeWidthStr::width(word);
        if current.is_empty() && word_width <= width {
            current.push_str(word);
            current_width = word_width;
        } else if !current.is_empty() && current_width + 1 + word_width <= width {
            current.push(' ');
            current.push_str(word);
            current_width += 1 + word_width;
        } else {
            if !current.is_empty() {
                out.push(std::mem::take(&mut current));
                current_width = 0;
            }
            if word_width <= width {
                current.push_str(word);
                current_width = word_width;
            } else {
                let mut chunks = hard_wrap_word(word, width).into_iter().peekable();
                while let Some(chunk) = chunks.next() {
                    if chunks.peek().is_some() {
                        out.push(chunk);
                    } else {
                        current_width = UnicodeWidthStr::width(chunk.as_str());
                        current = chunk;
                    }
                }
            }
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

fn hard_wrap_word(word: &str, width: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut chunk = String::new();
    let mut chunk_width = 0;
    for c in word.chars() {
        let char_width = UnicodeWidthChar::width(c).unwrap_or(0);
        if chunk_width > 0 && chunk_width + char_width > width {
            chunks.push(std::mem::take(&mut chunk));
            chunk_width = 0;
        }
        chunk.push(c);
        chunk_width += char_width;
    }
    if !chunk.is_empty() {
        chunks.push(chunk);
    }
    chunks
}

/// Parse a food proposal, taking the optional top-level `message` out before
/// deserializing the rest as a normal `Food`. Removing the key first preserves
/// `Food`'s strict schema and keeps the response field out of the written file.
pub(crate) fn parse_food(s: &str) -> Result<Proposal<food::Food>, String> {
    let mut table: toml::Table = toml::from_str(s).map_err(|e| e.to_string())?;
    let message = match table.remove("message") {
        None => None,
        Some(toml::Value::String(s)) => Some(s),
        Some(_) => return Err("`message` must be a string".to_string()),
    };
    let food: food::Food = toml::Value::Table(table)
        .try_into()
        .map_err(|e: toml::de::Error| e.to_string())?;
    Ok(Proposal::new(food, message))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn food_toml(prefix: &str) -> String {
        format!(
            "{prefix}title = \"My Food\"\nservings = 1\n\n[[ingredients]]\nname = \"A\"\ncalories = 1\nprotein_g = 0\nfiber_g = 0\nfat_g = 0\ncarbs_g = 0\nalcohol_g = 0\n"
        )
    }

    #[test]
    fn test_render_without_message_keeps_body() {
        let proposal: Proposal<()> = Proposal::new((), None);
        assert_eq!(
            proposal.render_with_table_width("Body\n".to_string(), None),
            "Body\n"
        );
    }

    #[test]
    fn test_render_appends_message_block() {
        let proposal: Proposal<()> =
            Proposal::new((), Some("Assumed the latte used whole milk.".to_string()));
        assert_eq!(
            proposal.render_with_width("Body".to_string(), 72),
            "Body\n\nNote: Assumed the latte used whole milk.\n\n"
        );
    }

    #[test]
    fn test_render_indents_multiline_message() {
        let proposal: Proposal<()> = Proposal::new((), Some("First.\nSecond.".to_string()));
        assert_eq!(
            proposal.render_with_width("Body\n".to_string(), 72),
            "Body\n\nNote: First.\n      Second.\n\n"
        );
    }

    #[test]
    fn test_effective_width_prefers_table_and_columns_shrink_it() {
        assert_eq!(effective_width(Some(50), Some(100)), 50);
        assert_eq!(effective_width(Some(100), Some(60)), 60);
        assert_eq!(effective_width(Some(100), None), 100);
        assert_eq!(effective_width(None, Some(60)), 60);
        assert_eq!(effective_width(None, None), FALLBACK_MESSAGE_WIDTH);
    }

    #[test]
    fn test_render_wraps_long_message() {
        let message = "Assumed the latte used whole milk; correct me with feedback if it was actually skim milk instead.";
        let proposal: Proposal<()> = Proposal::new((), Some(message.to_string()));
        let rendered = proposal.render_with_width("Body".to_string(), 72);
        let lines: Vec<&str> = rendered.lines().collect();
        assert_eq!(lines[0], "Body");
        assert_eq!(lines[1], "");
        assert!(lines[2].starts_with("Note: "), "got: {rendered}");
        assert!(lines[3].starts_with("      "), "got: {rendered}");
        for line in rendered.lines() {
            assert!(
                UnicodeWidthStr::width(line) <= 72,
                "line too wide: {line:?}"
            );
        }
    }

    #[test]
    fn test_render_shrinks_to_smaller_width() {
        let message = "Assumed the latte used whole milk; correct me with feedback if it was actually skim milk instead.";
        let proposal: Proposal<()> = Proposal::new((), Some(message.to_string()));
        let rendered = proposal.render_with_width("Body".to_string(), 48);
        assert!(rendered.lines().count() > 4, "got: {rendered}");
        for line in rendered.lines() {
            assert!(
                UnicodeWidthStr::width(line) <= 48,
                "line too wide: {line:?}"
            );
        }
    }

    #[test]
    fn test_render_hard_wraps_long_word() {
        let proposal: Proposal<()> = Proposal::new((), Some("x".repeat(100)));
        let rendered = proposal.render_with_width("Body".to_string(), 72);
        assert!(rendered.lines().count() > 3, "got: {rendered}");
        for line in rendered.lines() {
            assert!(
                UnicodeWidthStr::width(line) <= 72,
                "line too wide: {line:?}"
            );
        }
    }

    #[test]
    fn test_blank_message_is_dropped() {
        let proposal: Proposal<()> = Proposal::new((), Some("  \n".to_string()));
        assert_eq!(proposal.message, None);
        assert_eq!(
            proposal.render_with_table_width("Body".to_string(), None),
            "Body"
        );
    }

    #[test]
    fn test_parse_food_extracts_message_and_keeps_recipe_notes() {
        let proposal = parse_food(&food_toml(
            "message = \"Used whole milk.\"\nnotes = \"Serve warm\"\n",
        ))
        .unwrap();
        assert_eq!(proposal.message.as_deref(), Some("Used whole milk."));
        assert_eq!(proposal.value.title, "My Food");
        assert_eq!(proposal.value.notes, "Serve warm");
    }

    #[test]
    fn test_parse_food_without_message() {
        let proposal = parse_food(&food_toml("")).unwrap();
        assert_eq!(proposal.message, None);
        assert_eq!(proposal.value.title, "My Food");
    }

    #[test]
    fn test_parse_food_rejects_non_string_message() {
        let err = parse_food(&food_toml("message = [\"one\", \"two\"]\n")).unwrap_err();
        assert!(err.contains("`message` must be a string"), "got: {err}");
    }

    #[test]
    fn test_parse_food_rejects_message_after_ingredient_table() {
        let toml = format!("{}message = \"misplaced\"\n", food_toml(""));
        assert!(parse_food(&toml).is_err());
    }

    #[test]
    fn test_parse_food_rejects_unknown_field() {
        let err = parse_food(&food_toml("bogus = 1\n")).unwrap_err();
        assert!(err.contains("unknown field"), "got: {err}");
    }
}
