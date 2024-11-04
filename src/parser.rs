use tree_sitter::{Parser, Query, QueryCursor, Range};

#[derive(Debug)]
pub struct PositionalText {
    pub text: String,
    pub range: Range,
}

pub struct LspParser {}

impl LspParser {
    /// Extract the text of tree-sitter captured node from source.
    fn node_text(node: tree_sitter::Node, src: &str) -> String {
        src[node.start_byte()..node.end_byte()]
            .to_string()
            .trim_matches('"')
            .into()
    }

    pub(crate) fn node_string(node: tree_sitter::Node, src: &str) -> String {
        Self::node_text(node, src)
    }

    pub fn parse_code(source_code: &str) -> Vec<PositionalText> {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_typescript::language_typescript())
            .expect("Error loading typescript grammar");
        let tree = parser.parse(source_code, None).unwrap();

        let user_query = r#"
            (variable_declarator
            name: ((identifier) @id (#eq? @id "folders"))
            value: ((array ((string) @item))))
        "#;
        let query = Query::new(&tree_sitter_typescript::language_typescript(), user_query).unwrap();

        let mut query_cursor = QueryCursor::new();
        let matches = query_cursor.matches(&query, tree.root_node(), source_code.as_bytes());

        // Find the capture index for capture @item
        let array_item_index = query
            .capture_index_for_name("item")
            .expect("couldn't find capture index for `@item`");

        // let query_matches = matches.collect::<Vec<QueryMatch>>();
        matches
            .flat_map(|m| {
                m.captures
                    .iter()
                    .filter(|cap| cap.index == array_item_index)
                    .map(|cap| PositionalText {
                        text: Self::node_string(cap.node, source_code),
                        range: cap.node.range(),
                    })
            })
            .collect::<Vec<PositionalText>>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_code() {
        let source_code = r#"
             export const folders = ["dir_a", "dir_b"];
             export const other = ["other"];
         "#;

        let used_folders = LspParser::parse_code(source_code);
        assert_eq!(2, used_folders.len());
        assert_eq!("dir_a", used_folders[0].text);
        assert_eq!("dir_b", used_folders[1].text);
    }

    #[test]
    fn test_empty_string() {
        let source_code = r#"
             export const folders = [""];
         "#;

        let used_folders = LspParser::parse_code(source_code);
        assert_eq!(1, used_folders.len());
        assert_eq!("", used_folders[0].text);
    }
}
