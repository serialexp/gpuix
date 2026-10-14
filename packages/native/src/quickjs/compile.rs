//! Compiles app files for QuickJS with oxc. TypeScript types are removed, and
//! JSX becomes calls to React's automatic runtime, imported from
//! `@gpuix/react/jsx-runtime` as a GPUIX app's `jsxImportSource` sets it.
//! Nothing else is lowered: QuickJS runs current JavaScript. What it can't
//! run (decorators, `accessor` fields) and TypeScript's CommonJS forms fail
//! the compile with their position, instead of failing later at a position
//! in the compiled code.
//!
//! Imports are kept as written unless they are marked `type`, as
//! TypeScript's `verbatimModuleSyntax` does, so an import kept only for its
//! side effects still runs.
//!
//! Each compile also returns a [`PositionMap`], which the runtime uses to
//! put the app's own lines and columns in stack traces.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;

use oxc::allocator::Allocator;
use oxc::ast::ast::{
    AccessorProperty, AccessorPropertyType, Declaration, Decorator, Program, TSExportAssignment,
    TSImportEqualsDeclaration, TSModuleReference,
};
use oxc::ast_visit::{walk, Visit};
use oxc::codegen::{Codegen, CodegenOptions};
use oxc::diagnostics::{OxcDiagnostic, Severity};
use oxc::parser::{ParseOptions, Parser};
use oxc::semantic::SemanticBuilder;
use oxc::span::{SourceType, Span};
use oxc::transformer::{JsxOptions, JsxRuntime, TransformOptions, Transformer, TypeScriptOptions};

const JSX_IMPORT_SOURCE: &str = "@gpuix/react";

/// The extensions an app module can have.
pub(crate) const EXTENSIONS: [&str; 6] = ["tsx", "ts", "mts", "jsx", "js", "mjs"];

pub(crate) struct Compiled {
    pub code: String,
    pub positions: PositionMap,
}

/// Compiles one app module to JavaScript. `path` picks the syntax by its
/// extension and names the file in errors. Each reads
/// `path:line:column: message`, followed by an indented line for each place
/// the error points to that has its own note. Columns count UTF-16 code
/// units, as editors and Node do.
pub(crate) fn compile(source: &str, path: &Path) -> Result<Compiled, String> {
    let source_type = source_type(path)?;
    // As Node does, so line 1's columns match the editor's.
    let source = source.strip_prefix('\u{FEFF}').unwrap_or(source);
    // oxc panics on some input it doesn't expect. The loader runs inside a
    // QuickJS callback, which can't unwind, so a panic would abort the app.
    catch_unwind(AssertUnwindSafe(|| {
        compile_source(source, path, source_type)
    }))
    .unwrap_or_else(|panic| {
        let message = panic
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
            .unwrap_or("no message");
        Err(format!(
            "{}: the compiler failed: {message}",
            path.display()
        ))
    })
}

fn compile_source(source: &str, path: &Path, source_type: SourceType) -> Result<Compiled, String> {
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, source_type)
        // Otherwise QuickJS finds a bad regex, with no position to show.
        .with_options(ParseOptions {
            parse_regular_expression: true,
            ..ParseOptions::default()
        })
        .parse();
    check(source, path, &parsed.diagnostics)?;
    let mut program = parsed.program;
    let semantic = SemanticBuilder::new()
        .with_check_syntax_error(true)
        // The transformer needs enum values worked out to compile `enum`.
        .with_enum_eval(true)
        .build(&program);
    check(source, path, &semantic.diagnostics)?;
    check(source, path, &unsupported(&program))?;
    let options = TransformOptions {
        typescript: TypeScriptOptions {
            only_remove_type_imports: true,
            ..TypeScriptOptions::default()
        },
        jsx: JsxOptions {
            runtime: JsxRuntime::Automatic,
            import_source: Some(JSX_IMPORT_SOURCE.to_string()),
            ..JsxOptions::enable()
        },
        ..TransformOptions::default()
    };
    let transformed = Transformer::new(&allocator, path, &options)
        .build_with_scoping(semantic.semantic.into_scoping(), &mut program);
    check(source, path, &transformed.diagnostics)?;
    let generated = Codegen::new()
        .with_options(CodegenOptions {
            source_map_path: Some(path.to_path_buf()),
            ..CodegenOptions::default()
        })
        .build(&program);
    let code = lone_carriage_returns_to_newlines(generated.code);
    let positions = generated
        .map
        .map(|map| {
            let tokens = map.get_tokens().map(|token| {
                (
                    token.get_dst_line(),
                    token.get_dst_col(),
                    token.get_src_line(),
                    token.get_src_col(),
                )
            });
            PositionMap::new(source, &code, tokens)
        })
        .unwrap_or_default();
    Ok(Compiled { code, positions })
}

/// Codegen escapes line breaks in strings, so a lone `\r` can only be left
/// in a template or a comment. QuickJS counts it as a new line in a template
/// but not in a block comment. A template reads it as `\n` anyway, so
/// writing `\n` keeps the meaning and every byte offset, and leaves `\n` the
/// only line break QuickJS counts.
fn lone_carriage_returns_to_newlines(code: String) -> String {
    if !code.contains('\r') {
        return code;
    }
    let mut bytes = code.into_bytes();
    for index in 0..bytes.len() {
        if bytes[index] == b'\r' && bytes.get(index + 1) != Some(&b'\n') {
            bytes[index] = b'\n';
        }
    }
    String::from_utf8(bytes).expect("replacing one ASCII byte keeps UTF-8 valid")
}

fn source_type(path: &Path) -> Result<SourceType, String> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("");
    if !EXTENSIONS.contains(&extension) || name.ends_with(".d.ts") {
        return Err(format!(
            "{}: an app module must be a .{} file",
            path.display(),
            EXTENSIONS.join(", .")
        ));
    }
    let source_type =
        SourceType::from_path(path).map_err(|error| format!("{}: {error}", path.display()))?;
    // Every app file is a module, and `.js` files may hold JSX, as in Bun.
    let jsx = !source_type.is_typescript() || extension == "tsx";
    Ok(source_type.with_module(true).with_jsx(jsx))
}

/// Errors for the syntax that compiles but can't run in QuickJS.
fn unsupported(program: &Program<'_>) -> Vec<OxcDiagnostic> {
    let mut finder = Unsupported(Vec::new());
    finder.visit_program(program);
    finder.0
}

struct Unsupported(Vec<OxcDiagnostic>);

impl Unsupported {
    fn report(&mut self, span: Span, message: &'static str, help: &'static str) {
        self.0.push(
            OxcDiagnostic::error(message)
                .with_label(span)
                .with_help(help),
        );
    }
}

/// TypeScript removes `declare` declarations, abstract members and
/// type-only imports, so what they hold never runs and isn't checked.
impl<'a> Visit<'a> for Unsupported {
    fn visit_declaration(&mut self, it: &Declaration<'a>) {
        if !it.declare() {
            walk::walk_declaration(self, it);
        }
    }

    fn visit_decorator(&mut self, it: &Decorator<'a>) {
        self.report(
            it.span,
            "Decorators aren't supported",
            "QuickJS can't run them yet",
        );
        walk::walk_decorator(self, it);
    }

    fn visit_accessor_property(&mut self, it: &AccessorProperty<'a>) {
        if it.r#type == AccessorPropertyType::TSAbstractAccessorProperty {
            return;
        }
        self.report(
            it.span,
            "`accessor` fields aren't supported",
            "QuickJS can't run them yet; write a getter and a setter",
        );
        walk::walk_accessor_property(self, it);
    }

    fn visit_ts_import_equals_declaration(&mut self, it: &TSImportEqualsDeclaration<'a>) {
        if it.import_kind.is_value()
            && matches!(
                it.module_reference,
                TSModuleReference::ExternalModuleReference(_)
            )
        {
            self.report(
                it.span,
                "`import ... = require(...)` isn't supported",
                "apps are ES modules; use `import ... from`",
            );
        }
        walk::walk_ts_import_equals_declaration(self, it);
    }

    fn visit_ts_export_assignment(&mut self, it: &TSExportAssignment<'a>) {
        self.report(
            it.span,
            "`export =` isn't supported",
            "apps are ES modules; use `export default`",
        );
        walk::walk_ts_export_assignment(self, it);
    }
}

/// Fails with every error `diagnostics` holds, one per line.
fn check(source: &str, path: &Path, diagnostics: &[OxcDiagnostic]) -> Result<(), String> {
    let source_lines = Lines::new(source, LineBreaks::Editor);
    let errors: Vec<String> = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == Severity::Error)
        .map(|diagnostic| {
            let labels: &[_] = &diagnostic.labels;
            let at = |offset: u32| {
                let (line, column) = source_lines.line_column(offset as usize);
                format!("{}:{}:{}", path.display(), line + 1, column + 1)
            };
            let mut text = match labels.first() {
                Some(label) => format!("{}: {}", at(label.offset()), diagnostic.message),
                None => format!("{}: {}", path.display(), diagnostic.message),
            };
            for label in labels {
                if let Some(note) = label.label() {
                    text.push_str(&format!("\n  {}: {note}", at(label.offset())));
                }
            }
            if let Some(help) = &diagnostic.help {
                text.push_str(&format!("\n  help: {help}"));
            }
            text
        })
        .collect();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("\n"))
    }
}

/// Which characters start a new line.
#[derive(Clone, Copy)]
enum LineBreaks {
    /// `\n`, `\r\n` and a lone `\r`, as editors count them.
    Editor,
    /// Also U+2028 and U+2029, as oxc's source maps count them.
    Oxc,
    /// Only `\n`, as QuickJS counts them in compiled code.
    QuickJs,
}

/// A text's lines, as one reader counts them, and its UTF-16 positions.
struct Lines<'a> {
    text: &'a str,
    /// The byte offset of each line's start.
    starts: Vec<usize>,
    /// The byte start, byte end and UTF-16 end of each non-ASCII character,
    /// so converting a position is a binary search, not a scan.
    wide: Vec<(usize, usize, usize)>,
}

impl<'a> Lines<'a> {
    fn new(text: &'a str, breaks: LineBreaks) -> Self {
        let mut starts = vec![0];
        let mut wide = Vec::new();
        let mut units = 0;
        let mut characters = text.char_indices().peekable();
        while let Some((offset, character)) = characters.next() {
            let after = offset + character.len_utf8();
            units += character.len_utf16();
            if !character.is_ascii() {
                wide.push((offset, after, units));
            }
            let breaks_line = match (breaks, character) {
                (_, '\n') => true,
                (LineBreaks::QuickJs, _) => false,
                (_, '\r') => characters.peek().map(|&(_, next)| next) != Some('\n'),
                (LineBreaks::Oxc, '\u{2028}' | '\u{2029}') => true,
                _ => false,
            };
            if breaks_line {
                starts.push(after);
            }
        }
        Self { text, starts, wide }
    }

    /// The line, from zero, that holds a byte offset.
    fn line_of(&self, offset: usize) -> usize {
        self.starts.partition_point(|&start| start <= offset) - 1
    }

    /// The byte offset of a line and UTF-16 column, both from zero. A column
    /// past the line's end gives the end; one inside a character, its start.
    fn offset(&self, line: usize, column: u32) -> Option<usize> {
        let start = *self.starts.get(line)?;
        let end = self
            .starts
            .get(line + 1)
            .copied()
            .unwrap_or(self.text.len());
        Some(self.byte(self.units(start) + column as usize).min(end))
    }

    /// The line and UTF-16 column, both from zero, of a byte offset. An
    /// offset inside a character counts as that character's start.
    fn line_column(&self, offset: usize) -> (usize, usize) {
        let mut offset = offset.min(self.text.len());
        while !self.text.is_char_boundary(offset) {
            offset -= 1;
        }
        let line = self.line_of(offset);
        (line, self.units(offset) - self.units(self.starts[line]))
    }

    /// The UTF-16 offset of a byte offset on a character boundary.
    fn units(&self, byte: usize) -> usize {
        match self.wide.partition_point(|&(_, end, _)| end <= byte) {
            0 => byte,
            index => {
                let (_, end, units) = self.wide[index - 1];
                units + (byte - end)
            }
        }
    }

    /// The byte offset of a UTF-16 offset, or of the character it is inside.
    fn byte(&self, units: usize) -> usize {
        let index = self.wide.partition_point(|&(_, _, end)| end <= units);
        let byte = match index {
            0 => units,
            index => {
                let (_, end, end_units) = self.wide[index - 1];
                end + (units - end_units)
            }
        };
        match self.wide.get(index) {
            Some(&(start, _, _)) => byte.min(start),
            None => byte,
        }
    }
}

/// Where each position in a compiled module came from in its source.
#[derive(Default)]
pub(crate) struct PositionMap {
    /// For each compiled line, as QuickJS counts them, `(byte column, source
    /// line, source column)` sorted by column, all from zero. Source lines
    /// are counted as editors count them, and source columns in UTF-16 code
    /// units; compiled columns count bytes, as QuickJS does.
    lines: Vec<Vec<(u32, u32, u32)>>,
}

impl PositionMap {
    /// From a source map's tokens: compiled line and UTF-16 column, then
    /// source line and column, with lines as oxc counts them. Each side goes
    /// through its byte offset to the lines its reader counts.
    fn new(source: &str, code: &str, tokens: impl Iterator<Item = (u32, u32, u32, u32)>) -> Self {
        let oxc_source = Lines::new(source, LineBreaks::Oxc);
        let oxc_code = Lines::new(code, LineBreaks::Oxc);
        let editor_source = Lines::new(source, LineBreaks::Editor);
        let quickjs_code = Lines::new(code, LineBreaks::QuickJs);
        let mut lines: Vec<Vec<(u32, u32, u32)>> = vec![Vec::new(); quickjs_code.starts.len()];
        for (line, column, source_line, source_column) in tokens {
            let (Some(offset), Some(source_offset)) = (
                oxc_code.offset(line as usize, column),
                oxc_source.offset(source_line as usize, source_column),
            ) else {
                continue;
            };
            let line = quickjs_code.line_of(offset);
            let column = offset - quickjs_code.starts[line];
            let (source_line, source_column) = editor_source.line_column(source_offset);
            lines[line].push((column as u32, source_line as u32, source_column as u32));
        }
        for line in &mut lines {
            line.sort_unstable();
        }
        Self { lines }
    }

    /// The source line and column, from one, of a compiled line and byte
    /// column, from one, as QuickJS reports them: the nearest mapping at or
    /// before the column, or the line's first one.
    pub(crate) fn source_position(&self, line: u32, column: u32) -> Option<(u32, u32)> {
        let mappings = self.lines.get(line.checked_sub(1)? as usize)?;
        let column = column.saturating_sub(1);
        let index = mappings
            .partition_point(|mapping| mapping.0 <= column)
            .saturating_sub(1);
        let &(_, line, column) = mappings.get(index)?;
        Some((line + 1, column + 1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = "a\r\nb\rc\u{2028}d\ne";

    #[test]
    fn each_reader_counts_its_own_line_breaks() {
        let starts = |breaks| Lines::new(TEXT, breaks).starts;
        assert_eq!(starts(LineBreaks::Editor), [0, 3, 5, 11]);
        assert_eq!(starts(LineBreaks::Oxc), [0, 3, 5, 9, 11]);
        assert_eq!(starts(LineBreaks::QuickJs), [0, 3, 11]);
    }

    #[test]
    fn line_columns_count_utf16_units_from_the_line_start() {
        let lines = Lines::new(TEXT, LineBreaks::Editor);
        // `d` sits after the three-byte, one-unit line separator.
        assert_eq!(lines.line_column(9), (2, 2));
        // Inside the separator counts as its start.
        assert_eq!(lines.line_column(7), (2, 1));
        assert_eq!(lines.line_column(100), (3, 1));
        assert_eq!(lines.offset(2, 2), Some(9));
        assert_eq!(lines.offset(2, 50), Some(11));
        assert_eq!(lines.offset(4, 0), None);
        // A column inside a surrogate pair counts as the emoji's start.
        let emoji = Lines::new("a😀b", LineBreaks::Editor);
        assert_eq!(emoji.offset(0, 2), Some(1));
        assert_eq!(emoji.offset(0, 3), Some(5));
        assert_eq!(emoji.line_column(5), (0, 3));
    }

    #[test]
    fn positions_map_quickjs_lines_and_bytes_to_editor_lines_and_units() {
        // Compiled line 0 holds a line separator, so oxc's line 1 is still
        // QuickJS's line 1 (from one), after the separator's three bytes.
        let code = "x\u{2028}y\nz";
        let source = "😀x\ry";
        let map = PositionMap::new(
            source,
            code,
            [(0, 0, 0, 2), (1, 0, 1, 0), (2, 0, 1, 0), (9, 0, 0, 0)].into_iter(),
        );
        assert_eq!(map.source_position(1, 1), Some((1, 3)));
        assert_eq!(map.source_position(1, 5), Some((2, 1)));
        assert_eq!(map.source_position(2, 1), Some((2, 1)));
        // Before a line's first mapping, the first one; out of range, none.
        assert_eq!(map.source_position(1, 3), Some((1, 3)));
        assert_eq!(map.source_position(3, 1), None);
        assert_eq!(map.source_position(0, 1), None);
    }

    #[test]
    fn only_lone_carriage_returns_become_newlines() {
        assert_eq!(
            lone_carriage_returns_to_newlines("a\rb\r\nc\r".to_string()),
            "a\nb\r\nc\n"
        );
    }
}
