pub(crate) fn transform(source: &str) -> Result<String, String> {
    Transformer { source }.transform_range(0, source.len())
}

struct Transformer<'a> {
    source: &'a str,
}

struct Element {
    output: String,
    end: usize,
}

struct OpenTag {
    name: String,
    attributes: Vec<(String, String)>,
    self_closing: bool,
    end: usize,
}

enum Child {
    Element(String),
    Expression(String),
    Text(String),
}

impl Transformer<'_> {
    fn transform_range(&self, start: usize, end: usize) -> Result<String, String> {
        let mut output = String::with_capacity(end - start);
        let mut cursor = start;
        while cursor < end {
            if let Some(literal_end) = self.lua_literal_end(cursor, end) {
                output.push_str(&self.source[cursor..literal_end]);
                cursor = literal_end;
                continue;
            }
            if self.byte(cursor) == Some(b'<') && self.can_start_element(cursor, start) {
                if let Some(element) = self.parse_element(cursor, end)? {
                    output.push_str(&element.output);
                    cursor = element.end;
                    continue;
                }
            }
            let character = self.source[cursor..end].chars().next().unwrap();
            output.push(character);
            cursor += character.len_utf8();
        }
        Ok(output)
    }

    fn parse_element(&self, start: usize, end: usize) -> Result<Option<Element>, String> {
        let Some(open) = self.parse_open_tag(start, end)? else {
            return Ok(None);
        };
        let mut children = Vec::new();
        let mut cursor = open.end;

        if !open.self_closing {
            loop {
                if cursor >= end {
                    return Err(self.error(start, format!("unclosed <{}> element", open.name)));
                }
                if self.source[cursor..end].starts_with("</") {
                    cursor = self.parse_close_tag(cursor, end, &open.name)?;
                    break;
                }
                if self.byte(cursor) == Some(b'<') {
                    if let Some(element) = self.parse_element(cursor, end)? {
                        children.push(Child::Element(element.output));
                        cursor = element.end;
                        continue;
                    }
                }
                if self.byte(cursor) == Some(b'{') {
                    let expression_end = self.braced_expression_end(cursor, end)?;
                    let expression = self.transform_range(cursor + 1, expression_end - 1)?;
                    children.push(Child::Expression(expression.trim().to_string()));
                    cursor = expression_end;
                    continue;
                }

                let text_start = cursor;
                while cursor < end
                    && self.byte(cursor) != Some(b'<')
                    && self.byte(cursor) != Some(b'{')
                {
                    let character = self.source[cursor..end].chars().next().unwrap();
                    cursor += character.len_utf8();
                }
                children.push(Child::Text(self.source[text_start..cursor].to_string()));
            }
        }

        let output = self.emit_element(&open.name, open.attributes, children, start)?;
        Ok(Some(Element {
            output,
            end: cursor,
        }))
    }

    fn parse_open_tag(&self, start: usize, end: usize) -> Result<Option<OpenTag>, String> {
        if self.byte(start) != Some(b'<') || self.byte(start + 1) == Some(b'/') {
            return Ok(None);
        }
        let mut cursor = start + 1;
        let name_start = cursor;
        while cursor < end && is_tag_name(self.byte(cursor).unwrap()) {
            cursor += 1;
        }
        if cursor == name_start || !is_tag_start(self.byte(name_start).unwrap()) {
            return Ok(None);
        }
        let name = self.source[name_start..cursor].to_string();
        if !matches!(
            self.byte(cursor),
            Some(b'>') | Some(b'/') | Some(b' ' | b'\t' | b'\r' | b'\n')
        ) {
            return Ok(None);
        }

        let mut attributes = Vec::new();
        loop {
            cursor = self.skip_whitespace(cursor, end);
            if cursor >= end {
                return Ok(None);
            }
            if self.source[cursor..end].starts_with("/>") {
                return Ok(Some(OpenTag {
                    name,
                    attributes,
                    self_closing: true,
                    end: cursor + 2,
                }));
            }
            if self.byte(cursor) == Some(b'>') {
                return Ok(Some(OpenTag {
                    name,
                    attributes,
                    self_closing: false,
                    end: cursor + 1,
                }));
            }

            let attribute_start = cursor;
            while cursor < end && is_attribute_name(self.byte(cursor).unwrap()) {
                cursor += 1;
            }
            if cursor == attribute_start {
                return Ok(None);
            }
            let attribute = self.source[attribute_start..cursor].to_string();
            cursor = self.skip_whitespace(cursor, end);
            if self.byte(cursor) != Some(b'=') {
                attributes.push((attribute, "true".to_string()));
                continue;
            }
            cursor = self.skip_whitespace(cursor + 1, end);
            let Some(value_start) = self.byte(cursor) else {
                return Err(self.error(cursor, format!("missing value for {attribute}")));
            };
            let value = match value_start {
                b'"' | b'\'' => {
                    let value_end = self.quoted_end(cursor, end, value_start).ok_or_else(|| {
                        self.error(cursor, format!("unclosed value for {attribute}"))
                    })?;
                    let value = decode_entities(&self.source[cursor + 1..value_end - 1]);
                    cursor = value_end;
                    quote_lua(&value)
                }
                b'{' => {
                    let value_end = self.braced_expression_end(cursor, end)?;
                    let value = self.transform_range(cursor + 1, value_end - 1)?;
                    cursor = value_end;
                    let value = value.trim();
                    if value.is_empty() {
                        return Err(self.error(cursor, format!("missing value for {attribute}")));
                    }
                    format!("({value})")
                }
                _ => {
                    return Err(self.error(
                        cursor,
                        format!("{attribute} values must be quoted or wrapped in braces"),
                    ));
                }
            };
            attributes.push((attribute, value));
        }
    }

    fn parse_close_tag(&self, start: usize, end: usize, expected: &str) -> Result<usize, String> {
        let mut cursor = self.skip_whitespace(start + 2, end);
        let name_start = cursor;
        while cursor < end && is_tag_name(self.byte(cursor).unwrap()) {
            cursor += 1;
        }
        let actual = &self.source[name_start..cursor];
        cursor = self.skip_whitespace(cursor, end);
        if self.byte(cursor) != Some(b'>') {
            return Err(self.error(start, "malformed closing tag"));
        }
        if actual != expected {
            return Err(self.error(start, format!("expected </{expected}>, found </{actual}>")));
        }
        Ok(cursor + 1)
    }

    fn emit_element(
        &self,
        name: &str,
        attributes: Vec<(String, String)>,
        children: Vec<Child>,
        start: usize,
    ) -> Result<String, String> {
        let is_text = name == "text";
        let has_attributes = !attributes.is_empty();
        let has_content_attribute = attributes.iter().any(|(name, _)| name == "content");
        let mut entries = attributes
            .into_iter()
            .map(|(name, value)| format!("[{}] = {value}", quote_lua(&name)))
            .collect::<Vec<_>>();

        if is_text {
            if has_content_attribute && children.iter().any(|child| !child_is_empty(child)) {
                return Err(self.error(start, "<text> cannot have both content and children"));
            }
            if let Some(content) = emit_text_content(children, self, start)? {
                if !has_attributes {
                    return Ok(format!("(gpuix.text({content}))"));
                }
                entries.push(format!("[\"content\"] = {content}"));
            }
        } else {
            for child in children {
                match child {
                    Child::Element(value) | Child::Expression(value) if !value.is_empty() => {
                        entries.push(value)
                    }
                    Child::Text(value) => {
                        let value = normalize_jsx_text(&decode_entities(&value));
                        if !value.is_empty() {
                            entries.push(format!("gpuix.text({})", quote_lua(&value)));
                        }
                    }
                    _ => {}
                }
            }
        }

        let props = format!("{{{}}}", entries.join(", "));
        let expression = if let Some(helper) = host_helper(name) {
            format!("gpuix.{helper}({props})")
        } else if name.as_bytes()[0].is_ascii_lowercase() {
            format!("gpuix.h({}, {props})", quote_lua(name))
        } else {
            format!("gpuix.h({name}, {props})")
        };
        Ok(format!("({expression})"))
    }

    fn braced_expression_end(&self, start: usize, end: usize) -> Result<usize, String> {
        let mut depth = 1usize;
        let mut cursor = start + 1;
        while cursor < end {
            if let Some(literal_end) = self.lua_literal_end(cursor, end) {
                cursor = literal_end;
                continue;
            }
            match self.byte(cursor) {
                Some(b'{') => depth += 1,
                Some(b'}') => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(cursor + 1);
                    }
                }
                _ => {}
            }
            let character = self.source[cursor..end].chars().next().unwrap();
            cursor += character.len_utf8();
        }
        Err(self.error(start, "unclosed LuaX expression"))
    }

    fn lua_literal_end(&self, start: usize, end: usize) -> Option<usize> {
        if self.source[start..end].starts_with("--") {
            if let Some(close) = self.long_bracket_end(start + 2, end) {
                return Some(close);
            }
            return Some(
                self.source[start..end]
                    .find('\n')
                    .map_or(end, |offset| start + offset + 1),
            );
        }
        match self.byte(start)? {
            quote @ (b'"' | b'\'') => self.quoted_end(start, end, quote),
            b'[' => self.long_bracket_end(start, end),
            _ => None,
        }
    }

    fn quoted_end(&self, start: usize, end: usize, quote: u8) -> Option<usize> {
        let mut cursor = start + 1;
        while cursor < end {
            match self.byte(cursor) {
                Some(b'\\') => cursor = (cursor + 2).min(end),
                Some(value) if value == quote => return Some(cursor + 1),
                _ => cursor += 1,
            }
        }
        None
    }

    fn long_bracket_end(&self, start: usize, end: usize) -> Option<usize> {
        if self.byte(start) != Some(b'[') {
            return None;
        }
        let mut cursor = start + 1;
        while self.byte(cursor) == Some(b'=') {
            cursor += 1;
        }
        if self.byte(cursor) != Some(b'[') {
            return None;
        }
        let equals = cursor - start - 1;
        let closing = format!("]{}]", "=".repeat(equals));
        self.source[cursor + 1..end]
            .find(&closing)
            .map(|offset| cursor + 1 + offset + closing.len())
    }

    fn skip_whitespace(&self, mut cursor: usize, end: usize) -> usize {
        while cursor < end
            && self
                .byte(cursor)
                .is_some_and(|byte| byte.is_ascii_whitespace())
        {
            cursor += 1;
        }
        cursor
    }

    fn can_start_element(&self, start: usize, range_start: usize) -> bool {
        let prefix = &self.source[range_start..start];
        let trimmed = prefix.trim_end();
        let Some(last) = trimmed.chars().next_back() else {
            return true;
        };
        if matches!(last, '=' | '(' | '{' | '[' | ',' | ';' | ':') {
            return true;
        }
        if !is_lua_identifier(last) {
            return matches!(last, '+' | '-' | '*' | '/' | '%' | '^' | '#');
        }
        let word_start = trimmed
            .char_indices()
            .rev()
            .find(|(_, character)| !is_lua_identifier(*character))
            .map_or(0, |(index, character)| index + character.len_utf8());
        matches!(
            &trimmed[word_start..],
            "return" | "and" | "or" | "not" | "then" | "do" | "else" | "elseif" | "in"
        )
    }

    fn byte(&self, index: usize) -> Option<u8> {
        self.source.as_bytes().get(index).copied()
    }

    fn error(&self, index: usize, message: impl AsRef<str>) -> String {
        let prefix = &self.source[..index.min(self.source.len())];
        let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
        let column = prefix
            .rsplit_once('\n')
            .map_or(prefix.chars().count() + 1, |(_, tail)| {
                tail.chars().count() + 1
            });
        format!("LuaX {line}:{column}: {}", message.as_ref())
    }
}

fn emit_text_content(
    children: Vec<Child>,
    transformer: &Transformer<'_>,
    start: usize,
) -> Result<Option<String>, String> {
    let mut parts = Vec::new();
    for child in children {
        match child {
            Child::Element(_) => {
                return Err(transformer.error(start, "<text> cannot contain host elements"));
            }
            Child::Expression(value) if !value.is_empty() => parts.push((value, true)),
            Child::Text(value) => {
                let value = normalize_jsx_text(&decode_entities(&value));
                if !value.is_empty() {
                    parts.push((quote_lua(&value), false));
                }
            }
            _ => {}
        }
    }
    Ok(match parts.len() {
        0 => None,
        1 if parts[0].1 => Some(format!("({})", parts[0].0)),
        1 => Some(parts.pop().unwrap().0),
        _ => Some(format!(
            "({})",
            parts
                .into_iter()
                .map(|(value, expression)| if expression {
                    format!("tostring(({value}))")
                } else {
                    value
                })
                .collect::<Vec<_>>()
                .join(" .. ")
        )),
    })
}

fn child_is_empty(child: &Child) -> bool {
    match child {
        Child::Text(value) => normalize_jsx_text(value).is_empty(),
        Child::Expression(value) => value.is_empty(),
        Child::Element(_) => false,
    }
}

fn normalize_jsx_text(value: &str) -> String {
    if !value.contains('\n') && !value.contains('\r') {
        return value.to_string();
    }
    let lines = value.replace("\r\n", "\n").replace('\r', "\n");
    let line_count = lines.lines().count();
    let mut output = String::new();
    for (index, line) in lines.lines().enumerate() {
        let mut line = line.replace('\t', " ");
        if index != 0 {
            line = line.trim_start().to_string();
        }
        if index + 1 != line_count {
            line = line.trim_end().to_string();
        }
        if line.is_empty() {
            continue;
        }
        output.push_str(&line);
        if index + 1 != line_count {
            output.push(' ');
        }
    }
    output
}

fn decode_entities(value: &str) -> String {
    value
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn quote_lua(value: &str) -> String {
    let mut output = String::with_capacity(value.len() + 2);
    output.push('"');
    for character in value.chars() {
        match character {
            '\\' => output.push_str("\\\\"),
            '"' => output.push_str("\\\""),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            character => output.push(character),
        }
    }
    output.push('"');
    output
}

fn is_tag_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

fn is_tag_name(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':')
}

fn is_attribute_name(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b':')
}

fn is_lua_identifier(character: char) -> bool {
    character.is_ascii_alphanumeric() || character == '_'
}

fn host_helper(name: &str) -> Option<&'static str> {
    Some(match name {
        "div" => "div",
        "text" => "text",
        "input" => "input",
        "img" => "img",
        "svg" => "svg",
        "code" => "code",
        "markdown" => "markdown",
        "diff" => "diff",
        "anchored" => "anchored",
        "virtual-list" => "virtual_list",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transforms_host_elements_attributes_and_text() {
        let output = transform(
            r#"return <div testId="root" style={root_style} enabled>
                <text>Count: {count}</text>
                <img src="avatar.png" />
            </div>"#,
        )
        .unwrap();

        assert!(output.contains("gpuix.div("));
        assert!(output.contains("[\"testId\"] = \"root\""));
        assert!(output.contains("[\"style\"] = (root_style)"));
        assert!(output.contains("[\"enabled\"] = true"));
        assert!(output.contains("gpuix.text((\"Count: \" .. tostring((count))))"));
        assert!(output.contains("gpuix.img("));
    }

    #[test]
    fn transforms_function_components_and_nested_expression_elements() {
        let output = transform(
            r#"return <Panel child={active and <text>Yes</text> or nil}>
                {rows}
            </Panel>"#,
        )
        .unwrap();

        assert!(output.contains("gpuix.h(Panel"));
        assert!(output.contains("active and (gpuix.text("));
        assert!(output.contains(", rows}"));
    }

    #[test]
    fn leaves_lua_strings_comments_and_comparisons_untouched() {
        let source = r#"
            local markup = "<div>not syntax</div>"
            -- <div>also not syntax</div>
            if left < right then return markup end
            if left<right then return markup end
            local compact = left<right and left or right
        "#;
        assert_eq!(transform(source).unwrap(), source);
    }

    #[test]
    fn reports_mismatched_closing_tags_with_a_location() {
        let error = transform("return <div><text>hello</div>").unwrap_err();
        assert!(error.contains("LuaX 1:"));
        assert!(error.contains("expected </text>, found </div>"));
    }
}
