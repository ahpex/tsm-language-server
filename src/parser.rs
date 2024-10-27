use tree_sitter::{Parser, Query, QueryCursor};

pub struct LspParser {}

impl LspParser {
    /// Extract the text of tree-sitter captured node from source.
    fn node_text(node: tree_sitter::Node, src: &str) -> String {
        src[node.start_byte()..node.end_byte()].to_string()
    }

    pub(crate) fn node_string(node: tree_sitter::Node, src: &str) -> String {
        Self::node_text(node, src)
    }

    pub fn parse_code(source_code: &str) -> Vec<String> {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_typescript::language_typescript())
            .expect("Error loading typescript grammar");
        let tree = parser.parse(source_code, None).unwrap();

        let user_query = r#"
            (variable_declarator
            name: ((identifier) @id (#eq? @id "folders"))
            value: ((array ((string ((string_fragment) @item))))))
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
                    .map(|cap| Self::node_string(cap.node, source_code))
            })
            .collect::<Vec<String>>()
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
        assert_eq!("dir_a", used_folders[0]);
        assert_eq!("dir_b", used_folders[1]);
    }
}
