//! TypeScript type stripping.
//!
//! A `.ts` or `.mts` module is reduced to JavaScript by erasing type syntax.
//! Nothing is emitted for a type and nothing is transpiled: constructs whose
//! JavaScript form is more than an erasure are rejected instead of being
//! compiled, so what runs is always the author's own value code.
//!
//! Erased:
//!
//! - type annotations on bindings, parameters, properties, and returns
//! - `interface`, `type` aliases, `abstract` members, and overload signatures
//! - `implements` clauses, `declare` fields, `readonly`, and access modifiers
//! - `as`, `as const`, `satisfies`, `<T>expr`, and non-null assertions
//! - type arguments and type parameters
//! - `import type`, `export type`, and inline `type` specifiers
//! - ambient declarations, including `declare namespace`, `declare module`,
//!   `declare enum`, and `declare global`
//!
//! A type-only import that reuses the name of another declaration in the same
//! module is a `TS_SEMANTIC` error, as it is in TypeScript, even though the
//! import itself is erased.
//!
//! An import declaration whose specifiers are all `type` is erased as a whole,
//! exactly like `import type`, so a module that only holds types does not have
//! to exist at run time. A declaration that keeps at least one value specifier,
//! and a bare `import "./side"`, keep their source and their side effects.
//!
//! Rejected with a `TS_UNSUPPORTED` diagnostic:
//!
//! - `enum` and `const enum` declarations (they emit a runtime object)
//! - `namespace` and `module` declarations with a body (they emit a closure)
//! - constructor parameter properties (they assign to `this`)
//! - decorators (they are not erasable in any JavaScript position)
//! - `import x = ...` and `export = ...` (they need a module transform)
//! - `export as namespace` (UMD-only)
//!
//! Syntax errors and semantic errors such as a duplicate declaration are
//! reported before evaluation. There is no type checking: nothing here knows
//! whether an annotated type is correct.
//!
//! Output is regenerated from the parsed module rather than edited in place.
//! Value imports, side-effect imports, and import specifiers are kept exactly
//! as written, but blank lines, indentation, and comments that sat only on an
//! erased node are not. No source map is produced; a stack trace points at the
//! generated JavaScript, not at the original TypeScript.
//!
//! Diagnostics are single lines of the form
//! `<path>:<line>:<column>: <message> [<code>]`, where the code is `TS_PARSE`,
//! `TS_SEMANTIC`, `TS_UNSUPPORTED`, `TS_TRANSFORM`, or `TS_EXTENSION`. Only the
//! first diagnostic is reported.

use std::path::Path;

use oxc_allocator::Allocator;
use oxc_ast::ast::{
    Decorator, FormalParameter, ImportDeclarationSpecifier, ImportOrExportKind, Program, Statement,
    TSEnumDeclaration, TSExportAssignment, TSImportEqualsDeclaration, TSModuleDeclaration,
    TSNamespaceExportDeclaration,
};
use oxc_ast_visit::{Visit, walk};
use oxc_codegen::Codegen;
use oxc_diagnostics::OxcDiagnostic;
use oxc_parser::Parser;
use oxc_semantic::{Scoping, SemanticBuilder};
use oxc_span::{SourceType, Span};
use oxc_transformer::{TransformOptions, Transformer};

/// Erase type syntax when `name` is a TypeScript module.
///
/// Returns `Ok(None)` for a JavaScript name so the caller keeps the original
/// source byte for byte. `Err` carries a formatted diagnostic.
pub(crate) fn strip_if_typescript(name: &str, source: &str) -> Result<Option<String>, String> {
    validate_extension(name)?;
    match Path::new(name).extension().and_then(|ext| ext.to_str()) {
        Some("ts" | "mts") => strip(name, source).map(Some),
        _ => Ok(None),
    }
}

pub(crate) fn validate_extension(name: &str) -> Result<(), String> {
    let path = Path::new(name);
    let extension = path.extension().and_then(|ext| ext.to_str());
    // A declaration file parses to an empty module; running one is always a
    // mistake, so say so instead of silently doing nothing.
    if name.ends_with(".d.ts") || name.ends_with(".d.mts") {
        return Err(format!(
            "{name}:1:1: declaration files are not executable modules [TS_EXTENSION]"
        ));
    }
    match extension {
        Some("cts" | "tsx") => Err(format!(
            "{name}:1:1: `.cts` and `.tsx` modules are not supported; \
             only `.ts` and `.mts` can be stripped [TS_EXTENSION]"
        )),
        _ => Ok(()),
    }
}

/// True when [`strip_if_typescript`] has work to do for `name`, which lets the
/// loader leave JavaScript bytes untouched.
pub(crate) fn is_typescript(name: &str) -> bool {
    matches!(
        Path::new(name).extension().and_then(|ext| ext.to_str()),
        Some("ts" | "mts")
    )
}

fn strip(name: &str, source: &str) -> Result<String, String> {
    let source_type = SourceType::from_path(Path::new(name)).map_err(|error| {
        format!(
            "{name}:1:1: {} [TS_EXTENSION]",
            message_line(&error.to_string())
        )
    })?;
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, source_type).parse();
    if let Some(error) = parsed.errors.first() {
        return Err(format_diagnostic(name, source, error, "TS_PARSE"));
    }
    if parsed.panicked {
        return Err(format!("{name}:1:1: parser panicked [TS_PARSE]"));
    }

    let mut program = parsed.program;

    // Check the program as written. Erasing a type-only import must not hide a
    // conflict it had with another declaration, so this runs before erasure.
    let checked = SemanticBuilder::new()
        .with_check_syntax_error(true)
        .build(&program);
    if let Some(error) = checked.errors.first() {
        return Err(format_diagnostic(name, source, error, "TS_SEMANTIC"));
    }
    let scoping = checked.semantic.into_scoping();
    // Oxc's checker compares imports against imports, not against type
    // declarations, so the remaining conflict rule is applied here.
    let type_only = type_only_imports(&program);
    if let Some(error) = type_import_conflict(name, source, &scoping, &type_only.bindings) {
        return Err(error);
    }

    erase_import_declarations(&mut program, &type_only.erased_declarations);
    // Rebuild scoping for the tree the transform will walk, because the first
    // one describes declarations that no longer exist.
    let semantic = SemanticBuilder::new()
        .with_check_syntax_error(false)
        .build(&program);

    let mut unsupported = Unsupported::new(name, source);
    unsupported.visit_program(&program);
    if let Some(error) = unsupported.errors.into_iter().next() {
        return Err(error);
    }

    let mut options = TransformOptions::default();
    // Keep imports that are not marked `type`, even when no value uses them:
    // importing a module runs its side effects. Only explicit type imports and
    // specifiers are erased.
    options.typescript.only_remove_type_imports = true;
    // `using` is not TypeScript syntax, so it must not be rewritten into
    // try/finally here.
    options.proposals.explicit_resource_management = false;
    let transformer = Transformer::new(&allocator, Path::new(name), &options);
    let transformed =
        transformer.build_with_scoping(semantic.semantic.into_scoping(), &mut program);
    if let Some(error) = transformed.errors.first() {
        return Err(format_diagnostic(name, source, error, "TS_TRANSFORM"));
    }
    Ok(Codegen::new().build(&program).code)
}

/// Type-only import declarations to erase, and every type-only local binding.
struct TypeOnlyImports {
    /// Declarations whose specifiers are all `type` and that are erased whole.
    erased_declarations: Vec<Span>,
    /// Span and name of every binding declared by a type-only specifier,
    /// including `import type` declarations the transform removes.
    bindings: Vec<(Span, String)>,
}

/// Collects the type-only bindings before anything is erased.
///
/// `import type { T } from "./t"` and `import { type T } from "./t"` bind the
/// same way. A declaration whose specifiers are all `type` is removed here, so
/// its module is never evaluated; `import "./side"` and a declaration with at
/// least one value specifier keep their side effects.
fn type_only_imports(program: &Program<'_>) -> TypeOnlyImports {
    let mut erased_declarations = Vec::new();
    let mut bindings = Vec::new();
    for statement in &program.body {
        let Statement::ImportDeclaration(declaration) = statement else {
            continue;
        };
        let Some(specifiers) = &declaration.specifiers else {
            continue;
        };
        let declared_type_only = matches!(declaration.import_kind, ImportOrExportKind::Type);
        let mut all_specifiers_are_types = !declared_type_only && !specifiers.is_empty();
        for specifier in specifiers {
            match specifier {
                ImportDeclarationSpecifier::ImportSpecifier(specifier) => {
                    if declared_type_only
                        || matches!(specifier.import_kind, ImportOrExportKind::Type)
                    {
                        bindings.push((
                            specifier.local.span,
                            specifier.local.name.as_str().to_owned(),
                        ));
                    } else {
                        all_specifiers_are_types = false;
                    }
                }
                ImportDeclarationSpecifier::ImportDefaultSpecifier(specifier)
                    if declared_type_only =>
                {
                    bindings.push((
                        specifier.local.span,
                        specifier.local.name.as_str().to_owned(),
                    ));
                }
                ImportDeclarationSpecifier::ImportNamespaceSpecifier(specifier)
                    if declared_type_only =>
                {
                    bindings.push((
                        specifier.local.span,
                        specifier.local.name.as_str().to_owned(),
                    ));
                }
                ImportDeclarationSpecifier::ImportDefaultSpecifier(_)
                | ImportDeclarationSpecifier::ImportNamespaceSpecifier(_) => {
                    all_specifiers_are_types = false;
                }
            }
        }
        if all_specifiers_are_types {
            erased_declarations.push(declaration.span);
        }
    }
    TypeOnlyImports {
        erased_declarations,
        bindings,
    }
}

/// Erases the import declarations collected by [`type_only_imports`].
fn erase_import_declarations(program: &mut Program<'_>, declarations: &[Span]) {
    program.body.retain(|statement| match statement {
        Statement::ImportDeclaration(declaration) => !declarations.contains(&declaration.span),
        _ => true,
    });
}

/// Reports a type-only binding that another declaration in the module reuses.
///
/// Oxc already rejects an import that collides with an import. It does not
/// compare an import binding with a `type`, `interface`, or value declaration,
/// which TypeScript rejects, and erasing the import would hide the conflict.
/// The binder records such a collision as a redeclaration of one symbol, so the
/// symbol table is the cheapest reliable source.
fn type_import_conflict(
    name: &str,
    source: &str,
    scoping: &Scoping,
    bindings: &[(Span, String)],
) -> Option<String> {
    let root = scoping.root_scope_id();
    for (span, local) in bindings {
        let Some(symbol_id) = scoping.get_binding(root, local) else {
            continue;
        };
        if scoping.symbol_redeclarations(symbol_id).is_empty() {
            continue;
        }
        let (line, column) = position(source, span.start as usize);
        return Some(format!(
            "{name}:{line}:{column}: the type-only import of `{local}` conflicts with another \
             declaration of `{local}` [TS_SEMANTIC]"
        ));
    }
    None
}

/// Rejects constructs that would need a real TypeScript transform.
///
/// Ambient declarations are allowed: an enclosing `declare` context means the
/// whole subtree is erased, including `enum` and `namespace` inside it.
struct Unsupported<'s> {
    name: &'s str,
    source: &'s str,
    ambient: usize,
    errors: Vec<String>,
}

impl<'s> Unsupported<'s> {
    fn new(name: &'s str, source: &'s str) -> Self {
        Self {
            name,
            source,
            ambient: 0,
            errors: Vec::new(),
        }
    }

    fn reject(&mut self, span: Span, construct: &str, why: &str) {
        let (line, column) = position(self.source, span.start as usize);
        self.errors.push(format!(
            "{}:{line}:{column}: {construct} cannot be erased; {why} [TS_UNSUPPORTED]",
            self.name
        ));
    }

    fn is_ambient(&self, declare: bool) -> bool {
        declare || self.ambient > 0
    }
}

impl<'a> Visit<'a> for Unsupported<'_> {
    fn visit_ts_enum_declaration(&mut self, declaration: &TSEnumDeclaration<'a>) {
        if !self.is_ambient(declaration.declare) {
            self.reject(
                declaration.span,
                "an `enum` declaration",
                "an enum emits a runtime object, so neo-js does not strip it",
            );
        }
        walk::walk_ts_enum_declaration(self, declaration);
    }

    fn visit_ts_module_declaration(&mut self, declaration: &TSModuleDeclaration<'a>) {
        let ambient = self.is_ambient(declaration.declare);
        if !ambient && declaration.body.is_some() {
            self.reject(
                declaration.span,
                "a `namespace` declaration",
                "a namespace emits a runtime closure, so neo-js does not strip it",
            );
        }
        if ambient {
            self.ambient += 1;
        }
        walk::walk_ts_module_declaration(self, declaration);
        if ambient {
            self.ambient -= 1;
        }
    }

    fn visit_formal_parameter(&mut self, parameter: &FormalParameter<'a>) {
        if parameter.accessibility.is_some() || parameter.readonly || parameter.r#override {
            self.reject(
                parameter.span,
                "a constructor parameter property",
                "a parameter property assigns to `this`, so neo-js does not strip it",
            );
        }
        walk::walk_formal_parameter(self, parameter);
    }

    fn visit_decorator(&mut self, decorator: &Decorator<'a>) {
        self.reject(
            decorator.span,
            "a decorator",
            "decorators are not type syntax, so neo-js does not strip them",
        );
        walk::walk_decorator(self, decorator);
    }

    fn visit_ts_import_equals_declaration(&mut self, declaration: &TSImportEqualsDeclaration<'a>) {
        self.reject(
            declaration.span,
            "an `import =` declaration",
            "it needs a module transform, so neo-js does not strip it",
        );
        walk::walk_ts_import_equals_declaration(self, declaration);
    }

    fn visit_ts_export_assignment(&mut self, assignment: &TSExportAssignment<'a>) {
        self.reject(
            assignment.span,
            "an `export =` assignment",
            "it needs a module transform, so neo-js does not strip it",
        );
        walk::walk_ts_export_assignment(self, assignment);
    }

    fn visit_ts_namespace_export_declaration(
        &mut self,
        declaration: &TSNamespaceExportDeclaration<'a>,
    ) {
        self.reject(
            declaration.span,
            "an `export as namespace` declaration",
            "it is UMD-only, so neo-js does not strip it",
        );
    }
}

fn format_diagnostic(name: &str, source: &str, error: &OxcDiagnostic, code: &str) -> String {
    // Point at the primary label if there is one; otherwise at the last label,
    // which is the site that finally rejected the code.
    let offset = error
        .labels
        .as_ref()
        .and_then(|labels| {
            labels
                .iter()
                .find(|label| label.primary())
                .or_else(|| labels.last())
        })
        .map_or(0, |label| label.offset());
    let (line, column) = position(source, offset);
    format!(
        "{name}:{line}:{column}: {} [{code}]",
        message_line(&error.to_string())
    )
}

/// Collapses a diagnostic message to one line so the caller can prefix it.
fn message_line(message: &str) -> String {
    let mut collapsed = String::with_capacity(message.len());
    for word in message.split_whitespace() {
        if !collapsed.is_empty() {
            collapsed.push(' ');
        }
        collapsed.push_str(word);
    }
    if collapsed.is_empty() {
        "invalid TypeScript".to_owned()
    } else {
        collapsed
    }
}

/// One-based line and character column of a byte offset.
fn position(source: &str, offset: usize) -> (usize, usize) {
    let offset = offset.min(source.len());
    let offset = (0..=offset)
        .rev()
        .find(|&offset| source.is_char_boundary(offset))
        .unwrap_or(0);
    let before = &source[..offset];
    let line = before.matches('\n').count() + 1;
    let column = match before.rfind('\n') {
        Some(newline) => before[newline + 1..].chars().count() + 1,
        None => before.chars().count() + 1,
    };
    (line, column)
}
