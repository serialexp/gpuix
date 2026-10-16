//! Compiles app files for QuickJS with oxc. TypeScript types are removed, and
//! JSX is compiled for the app's framework ([`Jsx`]): for React, calls to its
//! automatic runtime, imported from `@gpuix/react/jsx-runtime`; for Solid,
//! what Solid's own compiler (`solidjs-compiler`) writes in universal mode,
//! imported from `@gpuix/solid`, as `@gpuix/solid`'s Bun plugin compiles it.
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
//!
//! In [`Mode::Development`] (`gpx --watch`), React files also get React
//! Refresh's registrations, as `import.meta.$RefreshReg$` and
//! `import.meta.$RefreshSig$` calls the loader provides, and Solid's compiler
//! writes its development output.

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
use oxc::transformer::{
    JsxOptions, JsxRuntime, ReactRefreshOptions, TransformOptions, Transformer, TypeScriptOptions,
};
use oxc_sourcemap::SourceMap;

use super::Mode;
use solidjs_compiler::{CompileOptions, Generate, Syntax};

/// Which framework an app file's JSX is compiled for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Jsx {
    React,
    Solid,
}

impl Jsx {
    /// The package JSX compiled for this framework imports from, which a
    /// tsconfig's `jsxImportSource` names.
    pub(crate) const fn import_source(self) -> &'static str {
        match self {
            Self::React => "@gpuix/react",
            Self::Solid => "@gpuix/solid",
        }
    }
}

/// The control flow `@gpuix/solid` re-exports, which Solid's compiled JSX
/// imports from it, as its Bun plugin lists them.
const SOLID_BUILT_INS: [&str; 8] = [
    "For", "Show", "Switch", "Match", "Loading", "Reveal", "Repeat", "Errored",
];

/// A compiled source map's token: compiled line and UTF-16 column, then
/// source line and column, all from zero, with lines as oxc counts them.
type Token = (u32, u32, u32, u32);

/// The extensions an app module can have.
pub(crate) const EXTENSIONS: [&str; 6] = ["tsx", "ts", "mts", "jsx", "js", "mjs"];

pub(crate) struct Compiled {
    pub code: String,
    pub positions: PositionMap,
    /// Whether the code registers with React Refresh, through
    /// `import.meta.$RefreshReg$` and `import.meta.$RefreshSig$`.
    pub refresh: bool,
}

/// Compiles one app module to JavaScript. `path` picks the syntax by its
/// extension and names the file in errors, and `jsx` says which framework
/// the JSX is for; it is only asked for a file that can hold JSX. Each reads
/// `path:line:column: message`, followed by an indented line for each place
/// the error points to that has its own note. Columns count UTF-16 code
/// units, as editors and Node do. `mode` is the framework builds the app
/// runs with.
pub(crate) fn compile(
    source: &str,
    path: &Path,
    mode: Mode,
    jsx: impl FnOnce() -> Result<Jsx, String>,
) -> Result<Compiled, String> {
    let source_type = source_type(path)?;
    // As Node does, so line 1's columns match the editor's.
    let source = source.strip_prefix('\u{FEFF}').unwrap_or(source);
    // oxc panics on some input it doesn't expect. The loader runs inside a
    // QuickJS callback, which can't unwind, so a panic would abort the app.
    catch_unwind(AssertUnwindSafe(|| {
        // `.ts` files can't hold JSX, so they skip Solid's compiler, and only
        // ask under `--watch`: one in a React app registers its components
        // with React Refresh. A config naming some other JSX source only
        // matters to a file with JSX in it.
        let development = mode == Mode::Development;
        let framework = (source_type.is_jsx() || development).then(jsx);
        let jsx = match framework {
            Some(Ok(jsx)) if source_type.is_jsx() => jsx,
            Some(Err(error)) if has_jsx(source, source_type) => return Err(error),
            _ => Jsx::React,
        };
        let refresh = development && matches!(framework, Some(Ok(Jsx::React)));
        let (code, tokens) = match jsx {
            Jsx::Solid => compile_solid(source, path, source_type, mode)?,
            Jsx::React => compile_source(
                source,
                path,
                source_type,
                Some((jsx, refresh)),
                Origin::Source,
            )?,
        };
        let positions = PositionMap::new(source, &code, tokens.into_iter());
        Ok(Compiled {
            code,
            positions,
            refresh,
        })
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

/// Compiles Solid JSX in two steps. Solid's compiler turns the JSX into calls
/// to `@gpuix/solid` and keeps the TypeScript, which [`compile_source`] then
/// removes. Positions go through both source maps back to the app's source.
fn compile_solid(
    source: &str,
    path: &Path,
    source_type: SourceType,
    mode: Mode,
) -> Result<(String, Vec<Token>), String> {
    // Solid's compiler reports errors without a position, so the source is
    // checked first, as the React path checks it.
    analyze(
        &Allocator::default(),
        source,
        path,
        source_type,
        Origin::Source,
    )?;
    // Solid's compiler picks the syntax from the file name, and reads `.js`
    // as holding no JSX. Bun lets it hold JSX, so it gets named `.jsx`.
    let filename = match path.extension().and_then(|extension| extension.to_str()) {
        Some("js" | "mjs") => path.with_extension("jsx"),
        _ => path.to_path_buf(),
    };
    let options = CompileOptions {
        filename: Some(filename.display().to_string()),
        syntax: Syntax::Jsx,
        module_name: Jsx::Solid.import_source().to_string(),
        generate: Generate::Universal,
        // As the embedded `solid-js` build is.
        dev: mode == Mode::Development,
        source_map: true,
        built_ins: SOLID_BUILT_INS
            .iter()
            .map(|name| (*name).to_string())
            .collect(),
        ..CompileOptions::default()
    };
    let solid = solidjs_compiler::compile(source, &options)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    let map = solid
        .source_map
        .ok_or_else(|| format!("{}: Solid's compiler wrote no source map", path.display()))?;
    let map = SourceMap::from_json_string(&map).map_err(|error| {
        format!(
            "{}: Solid's compiler wrote a bad source map: {error}",
            path.display()
        )
    })?;
    let table = map.generate_lookup_table();
    // The nearest mapping at or before a position, or the line's first, as
    // source map composition does.
    let to_source = |line: u32, column: u32| {
        let token = map.lookup_token_approx(&table, line, column)?;
        Some((token.get_src_line(), token.get_src_col()))
    };
    let origin = Origin::Solid {
        source,
        to_source: &to_source,
    };
    let (code, tokens) = compile_source(&solid.code, path, source_type, None, origin)?;
    let tokens = tokens
        .into_iter()
        .filter_map(|(line, column, middle_line, middle_column)| {
            let (source_line, source_column) = to_source(middle_line, middle_column)?;
            Some((line, column, source_line, source_column))
        })
        .collect();
    Ok((code, tokens))
}

/// Whether `source` holds any JSX. Source that doesn't parse counts as
/// holding some.
fn has_jsx(source: &str, source_type: SourceType) -> bool {
    struct Finder(bool);
    impl<'a> Visit<'a> for Finder {
        fn visit_jsx_element(&mut self, _: &oxc::ast::ast::JSXElement<'a>) {
            self.0 = true;
        }
        fn visit_jsx_fragment(&mut self, _: &oxc::ast::ast::JSXFragment<'a>) {
            self.0 = true;
        }
    }
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, source_type).parse();
    if parsed.panicked
        || parsed
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == Severity::Error)
    {
        return true;
    }
    let mut finder = Finder(false);
    finder.visit_program(&parsed.program);
    finder.0
}

/// Parses `source` and checks it: syntax, scopes, and what QuickJS can't run.
fn analyze<'a>(
    allocator: &'a Allocator,
    source: &'a str,
    path: &Path,
    source_type: SourceType,
    origin: Origin,
) -> Result<(Program<'a>, oxc::semantic::Scoping), String> {
    let parsed = Parser::new(allocator, source, source_type)
        // Otherwise QuickJS finds a bad regex, with no position to show.
        .with_options(ParseOptions {
            parse_regular_expression: true,
            ..ParseOptions::default()
        })
        .parse();
    check(source, path, origin, &parsed.diagnostics)?;
    let program = parsed.program;
    let semantic = SemanticBuilder::new()
        .with_check_syntax_error(true)
        // The transformer needs enum values worked out to compile `enum`.
        .with_enum_eval(true)
        .build(&program);
    check(source, path, origin, &semantic.diagnostics)?;
    check(source, path, origin, &unsupported(&program))?;
    let scoping = semantic.semantic.into_scoping();
    Ok((program, scoping))
}

/// Compiles `source` with oxc, its JSX to React's automatic runtime, or left
/// alone when `jsx` is `None`. With `jsx`'s flag set, React components and
/// hooks also register with React Refresh. Returns the code and its source
/// map's tokens.
fn compile_source(
    source: &str,
    path: &Path,
    source_type: SourceType,
    jsx: Option<(Jsx, bool)>,
    origin: Origin,
) -> Result<(String, Vec<Token>), String> {
    let allocator = Allocator::default();
    let (mut program, scoping) = analyze(&allocator, source, path, source_type, origin)?;
    let options = TransformOptions {
        typescript: TypeScriptOptions {
            only_remove_type_imports: true,
            ..TypeScriptOptions::default()
        },
        jsx: match jsx {
            Some((jsx, refresh)) => JsxOptions {
                runtime: JsxRuntime::Automatic,
                import_source: Some(jsx.import_source().to_string()),
                // As `import.meta` calls, so each module gets its own, and
                // nothing is added that would move the source map's lines.
                refresh: refresh.then(|| ReactRefreshOptions {
                    refresh_reg: "import.meta.$RefreshReg$".to_string(),
                    refresh_sig: "import.meta.$RefreshSig$".to_string(),
                    ..ReactRefreshOptions::default()
                }),
                ..JsxOptions::enable()
            },
            None => JsxOptions::disable(),
        },
        ..TransformOptions::default()
    };
    let transformed =
        Transformer::new(&allocator, path, &options).build_with_scoping(scoping, &mut program);
    check(source, path, origin, &transformed.diagnostics)?;
    let generated = Codegen::new()
        .with_options(CodegenOptions {
            source_map_path: Some(path.to_path_buf()),
            ..CodegenOptions::default()
        })
        .build(&program);
    let code = lone_carriage_returns_to_newlines(generated.code);
    let tokens = generated
        .map
        .map(|map| {
            map.get_tokens()
                .map(|token| {
                    (
                        token.get_dst_line(),
                        token.get_dst_col(),
                        token.get_src_line(),
                        token.get_src_col(),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    Ok((code, tokens))
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

/// The syntax an app module's extension gives it.
pub(crate) fn source_type(path: &Path) -> Result<SourceType, String> {
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

/// Where the code a `check` reads came from, so its errors point at the
/// app's source.
#[derive(Clone, Copy)]
enum Origin<'a> {
    /// The code is the app's source.
    Source,
    /// Solid's compiler wrote the code from `source`. `to_source` takes a
    /// line and column in the code to one in `source`, both as oxc counts.
    Solid {
        source: &'a str,
        to_source: &'a dyn Fn(u32, u32) -> Option<(u32, u32)>,
    },
}

/// Fails with every error `diagnostics` holds, one per line.
fn check(
    code: &str,
    path: &Path,
    origin: Origin,
    diagnostics: &[OxcDiagnostic],
) -> Result<(), String> {
    let mut errors = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == Severity::Error)
        .peekable();
    if errors.peek().is_none() {
        return Ok(());
    }
    let code_lines = Lines::new(code, LineBreaks::Editor);
    // For `Origin::Solid`: the code's lines and the source's, as oxc and as
    // editors count them.
    let solid = match origin {
        Origin::Source => None,
        Origin::Solid { source, to_source } => Some((
            Lines::new(code, LineBreaks::Oxc),
            Lines::new(source, LineBreaks::Oxc),
            Lines::new(source, LineBreaks::Editor),
            to_source,
        )),
    };
    let line_column = |offset: u32| {
        let offset = offset as usize;
        let in_source = solid.as_ref().and_then(|(code, oxc, editor, to_source)| {
            let (line, column) = code.line_column(offset);
            let (line, column) = to_source(line as u32, column as u32)?;
            Some(editor.line_column(oxc.offset(line as usize, column)?))
        });
        in_source.unwrap_or_else(|| code_lines.line_column(offset))
    };
    let errors: Vec<String> = errors
        .map(|diagnostic| {
            let labels: &[_] = &diagnostic.labels;
            let at = |offset: u32| {
                let (line, column) = line_column(offset);
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
    Err(errors.join("\n"))
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
    fn solids_built_ins_match_the_bun_plugins() {
        let plugin = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../solid/src/bun-plugin.ts"),
        )
        .unwrap();
        let list = plugin
            .lines()
            .find_map(|line| line.strip_prefix("const BUILT_INS = ["))
            .and_then(|rest| rest.strip_suffix(']'))
            .expect("bun-plugin.ts declares `const BUILT_INS = [...]` on one line");
        let names: Vec<&str> = list
            .split(',')
            .map(|name| name.trim().trim_matches('"'))
            .collect();
        assert_eq!(names, SOLID_BUILT_INS);
    }

    #[test]
    fn errors_in_solids_output_point_at_the_source() {
        // Solid's output's third line came from the source's second as oxc
        // counts lines, which to an editor is still the first: U+2028
        // doesn't break a line there.
        let source = "x\u{2028}y z";
        let code = "a\nb\nccc";
        let to_source = |line: u32, column: u32| (line == 2).then_some((1, column));
        let origin = Origin::Solid {
            source,
            to_source: &to_source,
        };
        let error = OxcDiagnostic::error("bad").with_label(Span::new(6, 7));
        let path = Path::new("main.tsx");
        assert_eq!(
            check(code, path, origin, std::slice::from_ref(&error)),
            Err("main.tsx:1:5: bad".to_string())
        );
        // Unmapped, a position is the code's own.
        let unmapped = |_: u32, _: u32| None;
        let origin = Origin::Solid {
            source,
            to_source: &unmapped,
        };
        assert_eq!(
            check(code, path, origin, std::slice::from_ref(&error)),
            Err("main.tsx:3:3: bad".to_string())
        );
    }

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
